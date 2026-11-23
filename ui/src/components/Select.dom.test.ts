import assert from 'node:assert/strict';
import test from 'node:test';
import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { resolve, sep } from 'node:path';
import { setTimeout as pause } from 'node:timers/promises';
import { createServer } from 'vite';
import { browserStage, ownedEdgeProcesses, stopBrowser, stopOwnedProfile } from '../../../tools/canvas-page.ts';
import type { fixture } from './Select.dom.fixture';

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

test('mounted Select preserves authored choices, focus and label layout', async context => {
  const edge = (process.env.BATTLEPLAN_CANVAS_BROWSER ?? process.env.HOI4_CANVAS_BROWSER) ?? [process.env['ProgramFiles(x86)'], process.env.ProgramFiles, process.env.LOCALAPPDATA]
    .filter((directory): directory is string => Boolean(directory))
    .map(directory => resolve(directory, 'Microsoft/Edge/Application/msedge.exe')).find(existsSync);
  assert.ok(edge && existsSync(edge), 'Headless Edge is required; set BATTLEPLAN_CANVAS_BROWSER to its executable');
  const profile = await mkdtemp(resolve(tmpdir(), 'select-dom-'));
  const vite = await createServer({ configFile: false, root: resolve('.'),
    cacheDir: resolve(profile, 'vite'), optimizeDeps: { noDiscovery: true,
      include: ['react', 'react-dom', 'react-dom/client', 'react/jsx-runtime', 'react/jsx-dev-runtime'] },
    server: { host: '127.0.0.1', port: 0, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'select-page', configureServer(server) {
      server.middlewares.use('/select-page', (_request, response) => {
        response.setHeader('Content-Type', 'text/html'); response.end('<!doctype html><title>Synthetic choices</title>');
      });
    } }] });
  let browser: ReturnType<typeof spawn> | undefined, endpoint = '', browserPid: number | undefined;
  try {
    await vite.listen();
    const address = vite.httpServer!.address(); assert.ok(address && typeof address !== 'string');
    browser = spawn(edge, ['--headless', '--disable-gpu', '--no-first-run', '--edge-skip-compat-layer-relaunch',
      '--remote-debugging-port=0', `--user-data-dir=${profile}`], { windowsHide: true });
    browser.stdout?.resume(); browser.stderr?.resume();
    endpoint = await browserStage(browser, 'Select Edge startup', async signal => {
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
    await cdp(page, 'Page.navigate', { url: `http://127.0.0.1:${address.port}/select-page` });
    await browserStage(browser, 'Select page navigation', async signal => {
      while (await evaluate('location.pathname') !== '/select-page') await pause(50, undefined, { signal });
    });
    await evaluate(`import('/ui/src/components/Select.dom.fixture.tsx')`);
    const act = (expression: string) => evaluate(`window.selectFixture.${expression}`);
    const snapshot = () => act('snapshot()') as Promise<Snapshot>;
    const press = async (key: 'ArrowDown' | 'ArrowUp' | 'Home' | 'End' | 'Enter' | 'Tab', shift = false) => {
      await cdp(page, 'Input.dispatchKeyEvent', { type: 'keyDown', key, code: key, windowsVirtualKeyCode: ({ ArrowDown: 40, ArrowUp: 38, Home: 36, End: 35, Enter: 13, Tab: 9 })[key], modifiers: shift ? 8 : 0,
        ...(key === 'Enter' ? { text: '\r', unmodifiedText: '\r' } : {}) });
      await cdp(page, 'Input.dispatchKeyEvent', { type: 'keyUp', key, code: key, modifiers: shift ? 8 : 0 });
      await act('settle()');
    };
    await browserStage(browser, 'Select DOM assertions', async () => {
      await context.test('an arriving catalogue preserves the active saved token and native Arrow/Enter', async () => {
        await act('mount(true)'); await act('open()');
        assert.equal((await snapshot()).activeLabel, 'Saved choice unavailable');
        await act('loadChoices()');
        const loaded = await snapshot();
        assert.equal(loaded.activeRole, 'option', 'the saved option retains keyboard focus when its index changes');
        assert.equal(loaded.activeValue, 'saved');
        assert.equal(loaded.activeLabel, 'Synthetic recorded choice 100 whose full name is meaningful');
        assert.ok(loaded.rows.length < 40, 'the catalogue remains virtual');
        await press('ArrowDown'); assert.equal((await snapshot()).activeValue, 'v101');
        await press('Enter'); assert.equal((await snapshot()).chosen, 'v101');
        assert.equal((await snapshot()).open, false);
      });
      await context.test('reordered active choices and unavailable choices reconcile by value', async () => {
        await act('mount()'); await act('open()'); await press('ArrowDown');
        await act('replaceChoices([...selectFixture.choices().filter(option => option.value !== "v101"), {value:"v101",label:"Moved active choice"}])');
        assert.equal((await snapshot()).activeValue, 'v101');
        assert.equal((await snapshot()).activeLabel, 'Moved active choice');
        await act('replaceChoices(selectFixture.choices().map(option => ({...option,disabled:option.value === "v101"})))');
        assert.equal((await snapshot()).activeValue, 'v0', 'a disabled active value falls back to the first enabled choice');
        await act('replaceChoices([{value:"disabled",label:"Disabled",disabled:true},{value:"enabled",label:"Enabled"}])');
        assert.equal((await snapshot()).activeValue, 'enabled', 'a removed active value falls back to the first enabled choice');
        await press('Enter'); assert.equal((await snapshot()).chosen, 'enabled');
        await act('mount()'); await act('open()');
        await act('replaceChoices([{value:"first",label:"First"},{value:"explained",label:"Explained",disabled:true,reason:"Needs a loaded campaign"},{value:"last",label:"Last"}])');
        await press('Home');
        assert.equal((await snapshot()).activeValue, 'first');
        await press('ArrowDown');
        assert.equal((await snapshot()).activeValue, 'explained', 'ArrowDown focuses a disabled option that explains why');
        await press('Enter');
        assert.equal((await snapshot()).chosen, null, 'Enter does not choose a disabled option');
        assert.equal((await snapshot()).open, true, 'menu stays open when pressing Enter on a disabled option');
        await press('ArrowDown');
        assert.equal((await snapshot()).activeValue, 'last');
        await press('Enter');
        assert.equal((await snapshot()).chosen, 'last');
      });
      await context.test('offscreen authored labels determine desktop width without horizontal scrolling', async () => {
        await act('mount(false, 5000)'); await act('open()');
        const sized = await snapshot(), long = sized.rows.find(row => row.index === 100)!;
        assert.ok(sized.menuWidth > 350, 'a full label supplies the menu intrinsic width');
        assert.ok(sized.rows.length < 40, '5,000 complete options stay windowed');
        assert.equal(sized.listScrollWidth, sized.listWidth, 'the menu has no horizontal overflow');
        assert.equal(long.labelWidth, long.labelScrollWidth, 'the full label is visible even when its tooltip contains a reason');
        await press('Home');
        assert.ok(!(await snapshot()).rows.some(row => row.index === 100), 'the widest label is offscreen');
        assert.equal((await snapshot()).menuWidth, sized.menuWidth, 'scrolling does not collapse the menu width');
        await press('End'); assert.equal((await snapshot()).activeValue, 'v4999');
      });
      await context.test('Tab and Shift+Tab leave the open select without choosing; arrows move between choices', async () => {
        await act('mount(false, 5000)'); await act('open()');
        const first = (await snapshot()).activeValue;
        await press('ArrowDown'); assert.notEqual((await snapshot()).activeValue, first, 'arrows move between choices');
        await press('Tab');
        const left = await snapshot();
        assert.equal(left.open, false, 'Tab closes the menu');
        assert.notEqual(left.activeRole, 'option', 'Tab moves on from the select');
        assert.equal(left.chosen, null, 'leaving does not choose');
        await act('open()'); await press('Tab', true);
        const back = await snapshot();
        assert.equal(back.open, false, 'Shift+Tab closes the menu');
        assert.notEqual(back.activeRole, 'option', 'Shift+Tab moves back from the select');
        assert.equal(back.chosen, null);
      });
      await context.test('narrow viewports wrap and measure long rows while preserving keyboard access', async () => {
        const viewport = new WebSocket(page);
        await new Promise<void>((done, reject) => {
          viewport.addEventListener('open', () => viewport.send(JSON.stringify({ id: 1, method: 'Emulation.setDeviceMetricsOverride',
            params: { width: 280, height: 700, deviceScaleFactor: 1, mobile: false } })));
          viewport.addEventListener('message', event => {
            const result = JSON.parse(String(event.data)); if (result.id !== 1) return;
            if (result.error) reject(new Error(JSON.stringify(result.error))); else done();
          });
          viewport.addEventListener('error', () => reject(new Error('Viewport CDP connection failed')));
        });
        try {
          await act('mount()'); await act('open()');
          const narrow = await snapshot(), long = narrow.rows.find(row => row.index === 100)!;
          assert.equal(narrow.viewportWidth, 280);
          assert.ok(narrow.menuLeft >= 11 && narrow.menuRight <= narrow.viewportWidth - 11);
          assert.equal(narrow.listScrollWidth, narrow.listWidth, 'narrow menus do not require horizontal scrolling');
          assert.ok(long.height > 32, 'wrapped text increases the measured row height');
          assert.equal(long.labelWidth, long.labelScrollWidth);
          assert.ok(long.top >= narrow.scrollTop! && long.top + long.height <= narrow.scrollTop! + narrow.listHeight!, 'the full focused row is revealed after measuring its wrapped text');
          assert.equal(narrow.rows.find(row => row.index === 101)!.top, long.top + long.height, 'the next row starts after the wrapped label');
          const same = await act('sameChoicesNow()') as { before: [string, number][]; after: [string, number][] };
          assert.deepEqual(same.after, same.before, 'an equal list passed anew keeps the wrapped rows where they were');
          await act('settle()');
          await press('ArrowDown'); assert.equal((await snapshot()).activeValue, 'v101');
          await press('End'); assert.equal((await snapshot()).activeValue, 'v149');
          await press('Enter'); assert.equal((await snapshot()).chosen, 'v149');
        } finally { viewport.close(); }
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
