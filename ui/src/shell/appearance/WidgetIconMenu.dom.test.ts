import assert from 'node:assert/strict';
import test from 'node:test';
import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve, sep } from 'node:path';
import { setTimeout as pause } from 'node:timers/promises';
import { createServer } from 'vite';
import { browserStage, ownedEdgeProcesses, stopBrowser, stopOwnedProfile } from '../../../../tools/canvas-page.ts';
import type { fixture } from './WidgetIconMenu.dom.fixture';

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

async function desktopImagePolicies() {
  const config = JSON.parse(await readFile(resolve('crates/desktop/tauri.conf.json'), 'utf8'));
  const security = config.app?.security ?? config.tauri?.security;
  return (['csp', 'devCsp'] as const).map(key => {
    const directive = String(security[key]).split(';').map(part => part.trim()).find(part => part.startsWith('img-src'));
    assert.ok(directive, `the desktop ${key} names an img-src`);
    return directive;
  });
}

test('a widget header icon picker opened by right-click returns focus to the widget on close', async context => {
  const policies = await desktopImagePolicies();
  let policy = policies[0];
  const edge = (process.env.BATTLE_PLAN_CANVAS_BROWSER ?? process.env.HOI4_CANVAS_BROWSER) ?? [process.env['ProgramFiles(x86)'], process.env.ProgramFiles, process.env.LOCALAPPDATA]
    .filter((directory): directory is string => Boolean(directory))
    .map(directory => resolve(directory, 'Microsoft/Edge/Application/msedge.exe')).find(existsSync);
  assert.ok(edge && existsSync(edge), 'Headless Edge is required; set BATTLE_PLAN_CANVAS_BROWSER to its executable');
  const profile = await mkdtemp(resolve(tmpdir(), 'icon-menu-dom-'));
  const vite = await createServer({ configFile: false, root: resolve('.'),
    cacheDir: resolve(profile, 'vite'), optimizeDeps: { noDiscovery: true,
      include: ['react', 'react-dom', 'react-dom/client', 'react/jsx-runtime', 'react/jsx-dev-runtime'] },
    server: { host: '127.0.0.1', port: 0, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'icon-menu-page', configureServer(server) {
      server.middlewares.use('/icon-menu-page', (_request, response) => {
        response.setHeader('Content-Type', 'text/html'); response.setHeader('Content-Security-Policy', policy);
        response.end('<!doctype html><title>Synthetic widget header</title>');
      });
    } }] });
  let browser: ReturnType<typeof spawn> | undefined, endpoint = '', browserPid: number | undefined;
  try {
    await vite.listen();
    const address = vite.httpServer!.address(); assert.ok(address && typeof address !== 'string');
    browser = spawn(edge, ['--headless', '--disable-gpu', '--no-first-run', '--edge-skip-compat-layer-relaunch',
      '--remote-debugging-port=0', `--user-data-dir=${profile}`], { windowsHide: true });
    browser.stdout?.resume(); browser.stderr?.resume();
    endpoint = await browserStage(browser, 'Icon menu Edge startup', async signal => {
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
    await cdp(page, 'Page.navigate', { url: `http://127.0.0.1:${address.port}/icon-menu-page` });
    await browserStage(browser, 'Icon menu page navigation', async signal => {
      while (await evaluate('location.pathname') !== '/icon-menu-page') await pause(50, undefined, { signal });
    });
    await evaluate(`import('/ui/src/shell/appearance/WidgetIconMenu.dom.fixture.tsx')`);
    const act = (expression: string) => evaluate(`window.iconFixture.${expression}`);
    const snapshot = () => act('snapshot()') as Promise<Snapshot>;
    const escape = async () => {
      await cdp(page, 'Input.dispatchKeyEvent', { type: 'keyDown', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 });
      await cdp(page, 'Input.dispatchKeyEvent', { type: 'keyUp', key: 'Escape', code: 'Escape', windowsVirtualKeyCode: 27 });
      await act('settle()');
    };
    await browserStage(browser, 'Icon menu DOM assertions', async () => {
      await context.test('Escape from the icon picker opened on the header icon focuses the widget settings button', async () => {
        await act('mount()'); await act('rightClickIcon()'); await act('choose("Change icon")');
        const open = await snapshot();
        assert.equal(open.pickerOpen, true);
        assert.equal(open.focusInPicker, true, 'the picker takes focus for its search');
        await escape();
        const closed = await snapshot();
        assert.equal(closed.pickerOpen, false);
        assert.equal(closed.active, 'settings', 'focus returns to the widget it was opened from, not the page');
      });
      await context.test('the colour picker opened the same way returns focus the same way', async () => {
        await act('mount()'); await act('rightClickIcon()'); await act('choose("Change icon colour")');
        assert.equal((await snapshot()).pickerOpen, true);
        await escape();
        assert.equal((await snapshot()).active, 'settings');
      });
      await context.test('uploads become icons under the desktop app\'s image policy', async () => {
        for (const next of policies) {
          policy = next;
          await cdp(page, 'Page.reload', { ignoreCache: true });
          while (await evaluate('document.readyState') !== 'complete') await pause(50);
          await evaluate(`import('/ui/src/shell/appearance/WidgetIconMenu.dom.fixture.tsx')`);
          assert.deepEqual(await act('rasteriseUploads()'), [true, true], `a PNG and an untyped SVG under "${next}"`);
        }
      });
      await context.test('closing never takes focus from somewhere the user moved it', async () => {
        await act('mount()'); await act('rightClickIcon()'); await act('choose("Change icon")');
        await act('focusElsewhere()');
        assert.equal((await snapshot()).active, 'elsewhere');
        await escape();
        assert.equal((await snapshot()).active, 'elsewhere');
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
