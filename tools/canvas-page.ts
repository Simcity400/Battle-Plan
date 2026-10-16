/** Synthetic Canvas pages for pixel assertions and standalone profiling. */
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve, sep } from 'node:path';
import { spawn } from 'node:child_process';
import type { ChildProcess } from 'node:child_process';
import { stripTypeScriptTypes } from 'node:module';
import { setTimeout as pause } from 'node:timers/promises';

async function endpointAt(profile: string, signal: AbortSignal) {
  for (;;) {
    signal.throwIfAborted();
    try {
      const [port, path] = (await readFile(resolve(profile, 'DevToolsActivePort'), { encoding: 'utf8', signal })).trim().split('\n');
      if (port && path) return `ws://127.0.0.1:${port}/${path.replace(/^\//, '')}`;
    } catch (error) {
      signal.throwIfAborted();
      const code = (error as NodeJS.ErrnoException).code;
      if (code !== 'ENOENT' && code !== 'EBUSY') throw error;
    }
    await pause(50, undefined, { signal });
  }
}

/** Each browser stage ends on a result, a browser failure or its deadline. */
export async function browserStage<T>(child: ChildProcess, label: string, run: (signal: AbortSignal) => Promise<T>, timeout = 30_000): Promise<T> {
  const controller = new AbortController();
  const failed = new Promise<never>((_, reject) => {
    controller.signal.addEventListener('abort', () => reject(controller.signal.reason), { once: true });
  });
  const error = (reason: Error) => controller.abort(reason);
  const close = (code: number | null, signal: NodeJS.Signals | null) => error(new Error(`${label}: Edge exited (${signal ?? code})`));
  child.once('error', error); child.once('close', close);
  const timer = setTimeout(() => error(new Error(`${label} timed out`)), timeout);
  try {
    if (child.exitCode != null || child.signalCode != null) close(child.exitCode, child.signalCode);
    return await Promise.race([failed, controller.signal.aborted ? failed : run(controller.signal)]);
  } finally {
    clearTimeout(timer); child.removeListener('error', error); child.removeListener('close', close);
  }
}

function cdp(endpoint: string, method: string, params: Record<string, unknown> = {}, signal?: AbortSignal) {
  return new Promise<Record<string, unknown>>((done, reject) => {
    signal?.throwIfAborted();
    const socket = new WebSocket(endpoint);
    const finish = (error?: Error, result?: Record<string, unknown>) => {
      clearTimeout(timeout); signal?.removeEventListener('abort', aborted); socket.close();
      if (error) reject(error); else done(result!);
    };
    const aborted = () => finish(signal!.reason);
    const timeout = setTimeout(() => finish(new Error(`${method} timeout`)), 30_000);
    signal?.addEventListener('abort', aborted, { once: true });
    socket.addEventListener('open', () => socket.send(JSON.stringify({ id: 1, method, params })));
    socket.addEventListener('message', event => {
      const message = JSON.parse(String(event.data));
      if (message.id !== 1) return;
      if (message.error) finish(new Error(JSON.stringify(message.error))); else finish(undefined, message.result);
    });
    socket.addEventListener('error', () => finish(new Error(`${method} connection failed`)));
  });
}

function exited(child: ChildProcess) {
  return child.exitCode != null || child.signalCode != null;
}

async function stopTreePid(pid: number, signal?: AbortSignal) {
  await new Promise<void>((done, reject) => {
    signal?.throwIfAborted();
    const stop = spawn('taskkill', ['/PID', String(pid), '/T', '/F'], { windowsHide: true });
    const abort = () => { clearTimeout(timer); stop.kill(); reject(signal!.reason); };
    signal?.addEventListener('abort', abort, { once: true });
    let output = '';
    stop.stdout?.on('data', chunk => { output += String(chunk); });
    stop.stderr?.on('data', chunk => { output += String(chunk); });
    const timer = setTimeout(() => { stop.kill(); reject(new Error('Stopping owned Headless Edge tree timed out')); }, 5_000);
    stop.once('error', error => { clearTimeout(timer); signal?.removeEventListener('abort', abort); reject(error); });
    stop.once('close', code => {
      clearTimeout(timer); signal?.removeEventListener('abort', abort);
      if (code === 0) done();
      else reject(new Error(`Stopping owned Headless Edge tree ${pid} failed (${code}): ${output.trim()}`));
    });
  });
}

async function stopOwnedTree(child: ChildProcess, signal?: AbortSignal) {
  if (exited(child) || child.pid == null) return;
  if (process.platform !== 'win32') { child.kill('SIGKILL'); return; }
  await stopTreePid(child.pid, signal);
}

type OwnedEdgeProcess = { pid: number; parentId: number };

export async function ownedEdgeProcesses(profile: string, launcherPid: number | undefined, stop = false, browserPid: number | undefined = undefined, signal?: AbortSignal): Promise<OwnedEdgeProcess[]> {
  // A vanished launcher PID is insufficient: compatibility relaunches and
  // descendants can keep this exact profile alive. Match whole arguments.
  const script = `
$ErrorActionPreference = 'Stop'
$pattern = '(?:^|\\s)(?:"--user-data-dir={0}"|--user-data-dir="{0}"|--user-data-dir={0})(?:\\s|$)' -f [regex]::Escape($env:BATTLE_PLAN_CANVAS_PROFILE)
$all = @(Get-CimInstance Win32_Process -Filter "Name = 'msedge.exe'")
$owned = [System.Collections.Generic.HashSet[uint32]]::new()
foreach ($entry in $all) {
  if ($entry.CommandLine -match $pattern) { $null = $owned.Add($entry.ProcessId) }
}
do {
  $added = $false
  foreach ($entry in $all) {
    if ($owned.Contains($entry.ParentProcessId) -and $owned.Add($entry.ProcessId)) { $added = $true }
  }
} while ($added)
$handles = @{}
$live = @(
  foreach ($entry in $all) {
    if (-not $owned.Contains($entry.ProcessId) -and $entry.ProcessId -ne [uint32]$env:BATTLE_PLAN_CANVAS_LAUNCHER) { continue }
    $nativeProcess = Get-Process -Id $entry.ProcessId -ErrorAction SilentlyContinue
    if ($nativeProcess -and -not $nativeProcess.HasExited) {
      if (-not $owned.Contains($entry.ProcessId)) { throw 'Cannot verify the retained Edge launcher belongs to the owned profile' }
      $null = $nativeProcess.Handle
      $handles[$entry.ProcessId] = $nativeProcess
      [pscustomobject]@{pid=$entry.ProcessId; parentId=$entry.ParentProcessId}
    }
  }
)
if ($env:BATTLE_PLAN_CANVAS_STOP -eq '1') {
  $ids = [System.Collections.Generic.HashSet[uint32]]::new()
  foreach ($entry in $live) { $null = $ids.Add($entry.pid) }
  foreach ($root in $live) {
    if ($ids.Contains($root.parentId)) { continue }
    $nativeRoot = $handles[$root.pid]
    if ($nativeRoot.HasExited) { continue }
    $fresh = Get-CimInstance Win32_Process -Filter ('ProcessId = ' + $root.pid)
    if (-not $fresh -or $fresh.CommandLine -notmatch $pattern) { throw 'Cannot verify an owned Edge tree root before stopping it' }
    $start = [System.Diagnostics.ProcessStartInfo]::new('taskkill.exe', ('/PID ' + $root.pid + ' /T /F'))
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $stopper = [System.Diagnostics.Process]::Start($start)
    if (-not $stopper.WaitForExit(5000)) { $stopper.Kill(); throw 'Stopping owned Edge tree timed out' }
    $message = $stopper.StandardOutput.ReadToEnd() + $stopper.StandardError.ReadToEnd()
    if ($stopper.ExitCode -ne 0 -and -not $nativeRoot.HasExited) {
      throw ('Stopping owned Edge tree ' + $root.pid + ' failed (' + $stopper.ExitCode + '), launcher ' + $env:BATTLE_PLAN_CANVAS_LAUNCHER + ', browser ' + $env:BATTLE_PLAN_CANVAS_BROWSER + ': ' + $message.Trim())
    }
  }
  # A compatibility relaunch can leave a new exact-profile root outside the
  # observed tree. Stop those roots too; never match another profile by prefix.
  foreach ($entry in @(Get-CimInstance Win32_Process -Filter "Name = 'msedge.exe'")) {
    if ($entry.CommandLine -notmatch $pattern) { continue }
    $nativeProcess = Get-Process -Id $entry.ProcessId -ErrorAction SilentlyContinue
    if (-not $nativeProcess -or $nativeProcess.HasExited) { continue }
    $null = $nativeProcess.Handle
    $fresh = Get-CimInstance Win32_Process -Filter ('ProcessId = ' + $entry.ProcessId)
    if ($nativeProcess.HasExited) { continue }
    if (-not $fresh -or $fresh.CommandLine -notmatch $pattern) { throw 'Cannot verify an owned Edge profile process before stopping it' }
    Stop-Process -InputObject $nativeProcess -Force -ErrorAction Stop
  }
}
ConvertTo-Json -InputObject $live -Compress
`;
  return new Promise((done, reject) => {
    signal?.throwIfAborted();
    const query = spawn('powershell.exe', ['-NoProfile', '-NonInteractive', '-EncodedCommand', Buffer.from(script, 'utf16le').toString('base64')], {
      windowsHide: true,
      env: { ...process.env, BATTLE_PLAN_CANVAS_PROFILE: profile, BATTLE_PLAN_CANVAS_LAUNCHER: String(launcherPid ?? 0), BATTLE_PLAN_CANVAS_STOP: stop ? '1' : '0', BATTLE_PLAN_CANVAS_BROWSER: String(browserPid ?? 0) },
    });
    const abort = () => { clearTimeout(timer); query.kill(); reject(signal!.reason); };
    signal?.addEventListener('abort', abort, { once: true });
    let output = '', errors = '';
    query.stdout?.on('data', chunk => { output += String(chunk); });
    query.stderr?.on('data', chunk => { errors += String(chunk); });
    const timer = setTimeout(() => { query.kill(); reject(new Error('Observing owned Headless Edge processes timed out')); }, 10_000);
    query.once('error', error => { clearTimeout(timer); signal?.removeEventListener('abort', abort); reject(error); });
    query.once('close', code => {
      clearTimeout(timer); signal?.removeEventListener('abort', abort);
      if (code !== 0) { reject(new Error(`Observing owned Headless Edge processes failed (${code}): ${errors.trim()}`)); return; }
      try {
        const processes: unknown = JSON.parse(output.trim());
        assert.ok(Array.isArray(processes) && processes.every(row => Number.isSafeInteger(row.pid) && row.pid > 0 && Number.isSafeInteger(row.parentId)));
        done(processes);
      } catch (error) { reject(error); }
    });
  });
}

function observationTimedOut(error: unknown) {
  return error instanceof Error && error.message === 'Observing owned Headless Edge processes timed out';
}

type CleanupOptions = { profile?: string; timeoutMs?: number; maxAttempts?: number; finalAttemptMs?: number };

class CleanupLimitError extends Error {}

/** Termination and verification spend one deadline and attempt allowance. */
export class BrowserCleanupBudget {
  profile: string | undefined;
  final = false;
  private readonly deadline: number;
  private readonly retryDeadline: number;
  private readonly maxAttempts: number;
  private attempts = 0;

  constructor(options: CleanupOptions = {}) {
    const timeout = options.timeoutMs ?? 30_000;
    const finalAttempt = options.finalAttemptMs ?? Math.min(5_000, timeout / 3);
    this.maxAttempts = options.maxAttempts ?? 24;
    assert.ok(timeout > 0 && finalAttempt > 0 && finalAttempt < timeout && this.maxAttempts >= 8);
    this.profile = options.profile;
    this.deadline = Date.now() + timeout;
    this.retryDeadline = this.deadline - finalAttempt;
  }

  get remaining() { return Math.max(0, (this.final ? this.deadline : this.retryDeadline) - Date.now()); }
  get canRetry() { return this.remaining > 0 && this.attempts < this.maxAttempts - (this.final ? 0 : 4); }

  async run<T>(work: (signal: AbortSignal) => Promise<T>, timeout = this.remaining): Promise<T> {
    if (!this.canRetry) throw new CleanupLimitError('Browser cleanup budget exhausted');
    this.attempts++;
    const controller = new AbortController();
    const failure = new Promise<never>((_, reject) => {
      controller.signal.addEventListener('abort', () => reject(controller.signal.reason), { once: true });
    });
    const timer = setTimeout(() => controller.abort(new CleanupLimitError('Browser cleanup deadline reached')), Math.min(timeout, this.remaining));
    try { return await Promise.race([failure, work(controller.signal)]); }
    finally { clearTimeout(timer); controller.abort(new CleanupLimitError('Browser cleanup step ended')); }
  }
}

function cleanupFailure(profile: string | undefined, cause: unknown) {
  return new Error(`Headless Edge did not stop; its profile is retained${profile ? `: ${profile}` : ''}`, { cause });
}

/** Retry observation within the caller's budget and sweep exact-profile survivors. */
export async function stopOwnedProfile(profile: string, launcherPid: number | undefined, browserPid: number | undefined,
  observe: typeof ownedEdgeProcesses = ownedEdgeProcesses, sharedBudget?: BrowserCleanupBudget) {
  const budget = sharedBudget ?? new BrowserCleanupBudget({ profile });
  budget.profile ??= profile;
  assert.equal(budget.profile, profile, 'Browser cleanup must retain its exact owned profile');
  const sweep = async () => {
    await budget.run(signal => observe(profile, launcherPid, true, browserPid, signal));
    // Retain the ownership check through a timed-out stop, but omit the old
    // numeric launcher after a completed request can let its PID be reused.
    launcherPid = undefined;
    return (await budget.run(signal => observe(profile, undefined, false, undefined, signal))).length === 0;
  };
  let failure: unknown;
  do {
    try { if (await sweep()) return; }
    catch (error) {
      failure = error;
      if (!observationTimedOut(error) && !(error instanceof CleanupLimitError)) break;
    }
  } while (!budget.final && budget.canRetry);
  if (!sharedBudget && !budget.final) {
    budget.final = true;
    try { if (await sweep()) return; } catch (error) { failure = error; }
  }
  if (sharedBudget) throw failure ?? new CleanupLimitError('Owned profile still has live processes');
  throw cleanupFailure(profile, failure);
}

type CleanupWork<T> = (budget: BrowserCleanupBudget, signal: AbortSignal) => Promise<T>;

/** Confirm absence before deleting a profile; all shutdown work has one budget. */
export async function stopBrowser(child: ChildProcess, requestClose: CleanupWork<unknown>, forceStop: CleanupWork<void> = (_budget, signal) => stopOwnedTree(child, signal), timeout = 5_000, verifyStopped: CleanupWork<boolean> = async () => exited(child), options: CleanupOptions = {}) {
  const budget = new BrowserCleanupBudget(options);
  let closed!: () => void;
  const stopped = new Promise<void>(done => { closed = done; child.once('close', closed); });
  let timer: ReturnType<typeof setTimeout> | undefined;
  const wait = async () => {
    try {
      await Promise.race([stopped, new Promise<void>(done => { timer = setTimeout(done, Math.min(timeout, budget.remaining)); })]);
    } finally { clearTimeout(timer); }
  };
  let confirmed = false;
  let observationFailure: unknown;
  const verify = async () => {
    observationFailure = undefined;
    try { return await budget.run(signal => verifyStopped(budget, signal)); }
    catch (error) { observationFailure = error; return false; }
  };
  try {
    if (exited(child) && await verify()) { confirmed = true; return; }
    await budget.run(signal => requestClose(budget, signal), timeout).catch(() => {});
    confirmed = await verify();
    if (!confirmed && !observationFailure) { await wait(); confirmed = await verify(); }
    let failure: unknown;
    while (!confirmed && budget.canRetry) {
      failure = undefined;
      try { await budget.run(signal => forceStop(budget, signal)); } catch (error) { failure = error; }
      // A graceful exit can race the PID-scoped force request. Its observed
      // exit settles shutdown even if the force utility found no live process.
      confirmed = await verify();
      if (!confirmed && !observationFailure) { await wait(); confirmed = await verify(); }
      if (confirmed) break;
      if (observationTimedOut(observationFailure) || observationTimedOut(failure)) continue;
      break;
    }
    if (!confirmed) {
      budget.final = true;
      // Keep Node's owned native process handle until absence is confirmed.
      // It can terminate the launcher even when CIM cannot answer; never use
      // a retained numeric PID to bypass the profile/native-handle checks.
      try { if (child.pid != null && !exited(child)) child.kill('SIGKILL'); } catch (error) { failure = error; }
      try { await budget.run(signal => forceStop(budget, signal)); } catch (error) { failure = error; }
      confirmed = await verify();
      if (!confirmed) throw cleanupFailure(budget.profile, observationFailure ?? failure);
    }
  } finally {
    // The close promise can remain pending after a refused shutdown.
    child.removeListener('close', closed);
    if (confirmed && !exited(child)) {
      // Native process-family absence is authoritative when Node's exit/pipe
      // notification remains pending. Release only this stopped child's handles.
      child.unref(); child.stdout?.destroy(); child.stderr?.destroy();
    }
  }
}

export async function canvasPage(code: string, gpu = false) {
  const edge = (process.env.BATTLE_PLAN_CANVAS_BROWSER ?? process.env.HOI4_CANVAS_BROWSER) ?? [process.env['ProgramFiles(x86)'], process.env.ProgramFiles, process.env.LOCALAPPDATA]
    .filter((directory): directory is string => Boolean(directory))
    .map(directory => resolve(directory, 'Microsoft/Edge/Application/msedge.exe')).find(existsSync);
  assert.ok(edge && existsSync(edge), 'Headless Edge is required; set BATTLE_PLAN_CANVAS_BROWSER to its executable');
  const root = resolve('ui/src');
  const profile = await mkdtemp(resolve(tmpdir(), 'share-canvas-'));
  let finish!: (result: Record<string, unknown>) => void;
  const completed = new Promise<Record<string, unknown>>(done => { finish = done; });
  const html = `<!doctype html><canvas id="plot" width="840" height="360"></canvas>
<script>
window.finish=result=>fetch('/result',{method:'POST',body:JSON.stringify(result)});
addEventListener('error',event=>finish({error:event.message}));
addEventListener('unhandledrejection',event=>finish({error:String(event.reason)}));
</script><script type="module">${code}</script>`;
  const server = createServer(async (request, response) => {
    try {
      const url = new URL(request.url ?? '/', 'http://localhost');
      if (url.pathname === '/result') {
        let body = '';
        for await (const chunk of request) body += chunk;
        finish(JSON.parse(body)); response.end(); return;
      }
      if (url.pathname === '/') { response.setHeader('Content-Type', 'text/html'); response.end(html); return; }
      if (url.pathname === '/favicon.ico') { response.statusCode = 204; response.end(); return; }
      let path = resolve(root, `.${url.pathname}`);
      assert.ok(path.startsWith(root + sep));
      if (!path.endsWith('.ts')) path += '.ts';
      response.setHeader('Content-Type', 'text/javascript');
      response.end(stripTypeScriptTypes(await readFile(path, 'utf8')));
    } catch (error) { response.statusCode = 500; response.end(String(error)); }
  });
  let endpoint = '';
  let child: ReturnType<typeof spawn> | undefined;
  let browserPid: number | undefined;
  try {
    await new Promise<void>(done => server.listen(0, '127.0.0.1', done));
    const address = server.address(); assert.ok(address && typeof address !== 'string');
    child = spawn(edge, ['--headless', ...(gpu ? ['--enable-gpu', '--ignore-gpu-blocklist', '--use-angle=d3d11'] : ['--disable-gpu']),
      '--no-first-run', '--edge-skip-compat-layer-relaunch', '--remote-debugging-port=0', `--user-data-dir=${profile}`], { windowsHide: true });
    child.stdout?.resume(); child.stderr?.resume();
    endpoint = await browserStage(child, 'Edge startup', signal => endpointAt(profile, signal));
    const result = await browserStage(child, 'Canvas page', async () => {
      await cdp(endpoint, 'Target.createTarget', { url: `http://127.0.0.1:${address.port}/` });
      return completed;
    }, 120_000);
    const error = result.error;
    assert.equal(error, undefined, error instanceof Error || typeof error === 'string' ? error : undefined);
    const processes = await cdp(endpoint, 'SystemInfo.getProcessInfo');
    const browser = (processes.processInfo as { type: string; id: number }[]).find(row => row.type === 'browser');
    assert.ok(browser && Number.isSafeInteger(browser.id) && browser.id > 0, 'Edge must report its browser process identity');
    browserPid = browser.id;
    const graphics = await cdp(endpoint, 'SystemInfo.getInfo').then(info => {
      const gpu = info.gpu as { devices: unknown; featureStatus: unknown; auxAttributes: { glRenderer?: string } };
      return { devices: gpu.devices, featureStatus: gpu.featureStatus, renderer: gpu.auxAttributes.glRenderer };
    });
    return { graphics, result };
  } finally {
    try {
      if (child) await stopBrowser(child, async (_budget, signal) => {
        if (endpoint) await cdp(endpoint, 'Browser.close', {}, signal);
      }, budget => stopOwnedProfile(profile, exited(child!) ? undefined : child!.pid, browserPid, ownedEdgeProcesses, budget), 5_000,
      async (_budget, signal) => (await ownedEdgeProcesses(profile, exited(child!) ? undefined : child!.pid, false, undefined, signal)).length === 0, { profile });
    } finally {
      await new Promise<void>(done => { server.close(() => done()); server.closeAllConnections(); });
    }
    assert.ok(profile.startsWith(resolve(tmpdir()) + sep));
    await rm(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
  }
}
