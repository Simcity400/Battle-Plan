import assert from 'node:assert/strict';
import test from 'node:test';
import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve, sep } from 'node:path';
import { setTimeout as pause } from 'node:timers/promises';
import { createServer } from 'vite';
import { browserStage, ownedEdgeProcesses, stopBrowser, stopOwnedProfile } from '../../../tools/canvas-page.ts';
import type { fixture } from './dockyardSizes.dom.fixture';

type Snapshot = ReturnType<typeof fixture.snapshot>;

function cdp(endpoint: string, method: string, params: Record<string, unknown> = {}, signal?: AbortSignal) {
  signal?.throwIfAborted();
  return new Promise<any>((done, reject) => {
    const socket = new WebSocket(endpoint);
    const cleanup = () => { clearTimeout(timeout); signal?.removeEventListener('abort', abort); socket.close(); };
    const abort = () => { cleanup(); reject(signal!.reason); };
    const timeout = setTimeout(() => { cleanup(); reject(new Error(`${method} timed out`)); }, 30_000);
    signal?.addEventListener('abort', abort, { once: true });
    socket.addEventListener('open', () => socket.send(JSON.stringify({ id: 1, method, params })));
    socket.addEventListener('message', event => {
      const message = JSON.parse(String(event.data)); if (message.id !== 1) return;
      cleanup();
      if (message.error) reject(new Error(JSON.stringify(message.error))); else done(message.result);
    });
    socket.addEventListener('error', () => { cleanup(); reject(new Error(`${method} connection failed`)); });
  });
}

test('Dockyard spending’s views fit their cards at every size', async context => {
  const edge = (process.env.BATTLE_PLAN_CANVAS_BROWSER ?? process.env.HOI4_CANVAS_BROWSER) ?? [process.env['ProgramFiles(x86)'], process.env.ProgramFiles, process.env.LOCALAPPDATA]
    .filter((directory): directory is string => Boolean(directory))
    .map(directory => resolve(directory, 'Microsoft/Edge/Application/msedge.exe')).find(existsSync);
  assert.ok(edge && existsSync(edge), 'Headless Edge is required; set BATTLE_PLAN_CANVAS_BROWSER to its executable');
  const profile = await mkdtemp(resolve(tmpdir(), 'sizes-dom-'));
  const vite = await createServer({ configFile: false, root: resolve('.'),
    cacheDir: resolve(profile, 'vite'), optimizeDeps: { noDiscovery: true,
      include: ['react', 'react-dom', 'react-dom/client', 'react/jsx-runtime', 'react/jsx-dev-runtime'] },
    server: { host: '127.0.0.1', port: 0, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'sizes-page', configureServer(server) {
      server.middlewares.use('/sizes-page', (_request, response) => {
        response.setHeader('Content-Type', 'text/html'); response.end('<!doctype html><title>Synthetic dockyard spending</title>');
      });
    } }] });
  let browser: ReturnType<typeof spawn> | undefined, endpoint = '', browserPid: number | undefined;
  try {
    await vite.listen();
    const address = vite.httpServer!.address(); assert.ok(address && typeof address !== 'string');
    browser = spawn(edge, ['--headless', '--disable-gpu', '--no-first-run', '--edge-skip-compat-layer-relaunch',
      '--remote-debugging-port=0', '--window-size=1400,900', `--user-data-dir=${profile}`], { windowsHide: true });
    browser.stdout?.resume(); browser.stderr?.resume();
    endpoint = await browserStage(browser, 'Sizes Edge startup', async signal => {
      for (;;) {
        signal.throwIfAborted();
        try {
          const [port, path] = (await readFile(resolve(profile, 'DevToolsActivePort'), { encoding: 'utf8', signal })).trim().split('\n');
          if (port && path) return `ws://127.0.0.1:${port}/${path.replace(/^\//, '')}`;
        } catch (error) { signal.throwIfAborted(); if (!['ENOENT', 'EBUSY'].includes((error as NodeJS.ErrnoException).code ?? '')) throw error; }
        await pause(50, undefined, { signal });
      }
    });
    const processes = await cdp(endpoint, 'SystemInfo.getProcessInfo');
    browserPid = processes.processInfo.find((entry: { type: string }) => entry.type === 'browser')?.id;
    assert.ok(browserPid && Number.isSafeInteger(browserPid));
    const { targetId } = await cdp(endpoint, 'Target.createTarget', { url: 'about:blank' });
    const port = new URL(endpoint).port;
    const targets = await fetch(`http://127.0.0.1:${port}/json/list`).then(response => response.json()) as { id: string; webSocketDebuggerUrl: string }[];
    const page = targets.find(target => target.id === targetId)!.webSocketDebuggerUrl;
    const evaluate = async (expression: string) => {
      const result = await cdp(page, 'Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
      assert.equal(result.exceptionDetails, undefined, JSON.stringify(result.exceptionDetails));
      return result.result.value;
    };
    await cdp(page, 'Page.navigate', { url: `http://127.0.0.1:${address.port}/sizes-page` });
    await browserStage(browser, 'Sizes page navigation', async signal => {
      while (await evaluate('location.pathname') !== '/sizes-page') await pause(50, undefined, { signal });
    });
    await evaluate(`import('/ui/src/widgets/dockyardSizes.dom.fixture.tsx')`);
    const act = (expression: string) => evaluate(`window.dockyardSizesFixture.${expression}`);
    const snapshot = () => act('snapshot()') as Promise<Snapshot>;
    await browserStage(browser, 'Dockyard sizes DOM assertions', async () => {
      // The widget's minimum card, a small one and a large one.
      const sizes = [[420, 320], [1200, 640]] as const;
      for (const form of await act('forms()') as string[]) await context.test(`${form} stays inside its card at minimum, small and large sizes`, async () => {
        const minimum = await act('minimum()') as [number, number];
        for (const [width, height] of [minimum, ...sizes]) {
          await act(`mount(${JSON.stringify(form)}, ${width}, ${height})`);
          const shown = await snapshot();
          // Optional review images of each size (never in ordinary runs).
          if (process.env.DOCKYARD_SIZES_SHOTS) {
            const shot = await cdp(page, 'Page.captureScreenshot', { format: 'png', clip: { x: 0, y: 0, width, height, scale: 1 } });
            await writeFile(resolve(process.env.DOCKYARD_SIZES_SHOTS, `${form}-${width}x${height}.png`), Buffer.from(shot.data, 'base64'));
          }
          assert.ok(shown.cardScrollWidth <= shown.cardWidth + 1, `${form} at ${width}x${height} overflows its card sideways (${shown.cardScrollWidth} > ${shown.cardWidth})`);
          assert.ok(shown.text.length > 0 || shown.canvas, `${form} at ${width}x${height} draws something`);
          assert.deepEqual(shown.clipped, [], `${form} at ${width}x${height} pushes controls past its card`);
          assert.ok(shown.canvasInside, `${form} at ${width}x${height} draws past its card`);
        }
      });
    }, 60_000);
  } finally {
    try {
      if (browser) await stopBrowser(browser, async (_budget, signal) => { if (endpoint) await cdp(endpoint, 'Browser.close', {}, signal); }, async budget => {
        if (process.platform === 'win32') {
          await stopOwnedProfile(profile, browser!.exitCode !== null || browser!.signalCode !== null ? undefined : browser!.pid, browserPid, ownedEdgeProcesses, budget); return;
        }
        assert.ok(browserPid, 'Only the browser identified by this owned CDP endpoint may be stopped'); process.kill(browserPid, 'SIGKILL');
      }, 5_000, async (_budget, signal) => {
        if (process.platform === 'win32') return (await ownedEdgeProcesses(profile, browser!.exitCode !== null || browser!.signalCode !== null ? undefined : browser!.pid, false, undefined, signal)).length === 0;
        if (!browserPid) return browser!.exitCode !== null || browser!.signalCode !== null;
        try { process.kill(browserPid, 0); return false; } catch (error) { if ((error as NodeJS.ErrnoException).code === 'ESRCH') return true; throw error; }
      }, { profile });
    } finally { await vite.close(); }
    assert.ok(profile.startsWith(resolve(tmpdir()) + sep));
    await rm(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
  }
});
