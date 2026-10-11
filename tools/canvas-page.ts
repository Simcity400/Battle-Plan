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

function cdp(endpoint: string, method: string, params: Record<string, unknown> = {}) {
  return new Promise<Record<string, unknown>>((done, reject) => {
    const socket = new WebSocket(endpoint);
    const timeout = setTimeout(() => { socket.close(); reject(new Error(`${method} timeout`)); }, 30_000);
    socket.addEventListener('open', () => socket.send(JSON.stringify({ id: 1, method, params })));
    socket.addEventListener('message', event => {
      const message = JSON.parse(String(event.data));
      if (message.id !== 1) return;
      clearTimeout(timeout); socket.close();
      if (message.error) reject(new Error(JSON.stringify(message.error))); else done(message.result);
    });
    socket.addEventListener('error', () => { clearTimeout(timeout); reject(new Error(`${method} connection failed`)); });
  });
}

function exited(child: ChildProcess) {
  return child.exitCode != null || child.signalCode != null;
}

async function stopTreePid(pid: number) {
  await new Promise<void>((done, reject) => {
    const stop = spawn('taskkill', ['/PID', String(pid), '/T', '/F'], { windowsHide: true });
    let output = '';
    stop.stdout?.on('data', chunk => { output += String(chunk); });
    stop.stderr?.on('data', chunk => { output += String(chunk); });
    const timer = setTimeout(() => { stop.kill(); reject(new Error('Stopping owned Headless Edge tree timed out')); }, 5_000);
    stop.once('error', error => { clearTimeout(timer); reject(error); });
    stop.once('close', code => {
      clearTimeout(timer);
      if (code === 0) done();
      else reject(new Error(`Stopping owned Headless Edge tree ${pid} failed (${code}): ${output.trim()}`));
    });
  });
}

async function stopOwnedTree(child: ChildProcess) {
  if (exited(child) || child.pid == null) return;
  if (process.platform !== 'win32') { child.kill('SIGKILL'); return; }
  await stopTreePid(child.pid);
}

type OwnedEdgeProcess = { pid: number; parentId: number };

async function ownedEdgeProcesses(profile: string, launcherPid: number | undefined, stop = false, browserPid: number | undefined = undefined): Promise<OwnedEdgeProcess[]> {
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
}
ConvertTo-Json -InputObject $live -Compress
`;
  return new Promise((done, reject) => {
    const query = spawn('powershell.exe', ['-NoProfile', '-NonInteractive', '-EncodedCommand', Buffer.from(script, 'utf16le').toString('base64')], {
      windowsHide: true,
      env: { ...process.env, BATTLE_PLAN_CANVAS_PROFILE: profile, BATTLE_PLAN_CANVAS_LAUNCHER: String(launcherPid ?? 0), BATTLE_PLAN_CANVAS_STOP: stop ? '1' : '0', BATTLE_PLAN_CANVAS_BROWSER: String(browserPid ?? 0) },
    });
    let output = '', errors = '';
    query.stdout?.on('data', chunk => { output += String(chunk); });
    query.stderr?.on('data', chunk => { errors += String(chunk); });
    const timer = setTimeout(() => { query.kill(); reject(new Error('Observing owned Headless Edge processes timed out')); }, 10_000);
    query.once('error', error => { clearTimeout(timer); reject(error); });
    query.once('close', code => {
      clearTimeout(timer);
      if (code !== 0) { reject(new Error(`Observing owned Headless Edge processes failed (${code}): ${errors.trim()}`)); return; }
      try {
        const processes: unknown = JSON.parse(output.trim());
        assert.ok(Array.isArray(processes) && processes.every(row => Number.isSafeInteger(row.pid) && row.pid > 0 && Number.isSafeInteger(row.parentId)));
        done(processes);
      } catch (error) { reject(error); }
    });
  });
}

async function stopOwnedProfile(profile: string, launcherPid: number | undefined, browserPid: number | undefined) {
  await ownedEdgeProcesses(profile, launcherPid, true, browserPid);
}

/** Observe shutdown before requesting it, and confirm exit before deleting a profile. */
export async function stopBrowser(child: ChildProcess, requestClose: () => Promise<unknown>, forceStop: () => Promise<void> = () => stopOwnedTree(child), timeout = 5_000, verifyStopped: () => Promise<boolean> = async () => exited(child)) {
  let closed!: () => void;
  const stopped = new Promise<void>(done => { closed = done; child.once('close', closed); });
  let timer: ReturnType<typeof setTimeout> | undefined;
  const wait = async () => {
    try {
      await Promise.race([stopped, new Promise<void>(done => { timer = setTimeout(done, timeout); })]);
    } finally { clearTimeout(timer); }
  };
  let confirmed = false;
  try {
    if (exited(child) && await verifyStopped()) { confirmed = true; return; }
    await requestClose().catch(() => {});
    confirmed = await verifyStopped();
    if (!confirmed) { await wait(); confirmed = await verifyStopped(); }
    if (!confirmed) {
      let failure: unknown;
      try { await forceStop(); } catch (error) { failure = error; }
      await wait();
      // A graceful exit can race the PID-scoped force request. Its observed
      // exit settles shutdown even if the force utility found no live process.
      confirmed = await verifyStopped();
      if (!confirmed && failure) throw failure;
    }
    assert.ok(confirmed, 'Headless Edge did not stop; its profile is retained');
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
      if (child) await stopBrowser(child, async () => {
        if (endpoint) await cdp(endpoint, 'Browser.close');
      }, () => stopOwnedProfile(profile, exited(child!) ? undefined : child!.pid, browserPid), 5_000,
      async () => (await ownedEdgeProcesses(profile, exited(child!) ? undefined : child!.pid)).length === 0);
    } finally {
      await new Promise<void>(done => { server.close(() => done()); server.closeAllConnections(); });
    }
    assert.ok(profile.startsWith(resolve(tmpdir()) + sep));
    await rm(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
  }
}
