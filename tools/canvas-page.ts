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
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
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
  try {
    await new Promise<void>(done => server.listen(0, '127.0.0.1', done));
    const address = server.address(); assert.ok(address && typeof address !== 'string');
    child = spawn(edge, ['--headless', ...(gpu ? ['--enable-gpu', '--ignore-gpu-blocklist', '--use-angle=d3d11'] : ['--disable-gpu']),
      '--no-first-run', '--remote-debugging-port=0', `--user-data-dir=${profile}`], { windowsHide: true });
    child.stdout?.resume(); child.stderr?.resume();
    endpoint = await browserStage(child, 'Edge startup', signal => endpointAt(profile, signal));
    const result = await browserStage(child, 'Canvas page', async () => {
      await cdp(endpoint, 'Target.createTarget', { url: `http://127.0.0.1:${address.port}/` });
      return completed;
    }, 120_000);
    const error = result.error;
    assert.equal(error, undefined, error instanceof Error || typeof error === 'string' ? error : undefined);
    const graphics = await cdp(endpoint, 'SystemInfo.getInfo').then(info => {
      const gpu = info.gpu as { devices: unknown; featureStatus: unknown; auxAttributes: { glRenderer?: string } };
      return { devices: gpu.devices, featureStatus: gpu.featureStatus, renderer: gpu.auxAttributes.glRenderer };
    });
    return { graphics, result };
  } finally {
    if (endpoint) await cdp(endpoint, 'Browser.close').catch(() => {});
    if (child && child.exitCode == null && child.signalCode == null) {
      await new Promise<void>(done => {
        const closed = () => { clearTimeout(timer); child!.removeListener('close', closed); done(); };
        const timer = setTimeout(closed, 5_000);
        child!.once('close', closed); child!.kill();
      });
    }
    await new Promise<void>(done => { server.close(() => done()); server.closeAllConnections(); });
    assert.ok(!child || child.exitCode != null || child.signalCode != null, 'Headless Edge did not stop; its profile is retained');
    assert.ok(profile.startsWith(resolve(tmpdir()) + sep));
    await rm(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
  }
}
