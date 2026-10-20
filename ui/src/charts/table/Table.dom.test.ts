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
import type { fixture } from './Table.dom.fixture';

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

test('narrow tables hide prioritised columns or scroll sideways', async context => {
  const edge = (process.env.BATTLE_PLAN_CANVAS_BROWSER ?? process.env.HOI4_CANVAS_BROWSER) ?? [process.env['ProgramFiles(x86)'], process.env.ProgramFiles, process.env.LOCALAPPDATA]
    .filter((directory): directory is string => Boolean(directory))
    .map(directory => resolve(directory, 'Microsoft/Edge/Application/msedge.exe')).find(existsSync);
  assert.ok(edge && existsSync(edge), 'Headless Edge is required; set BATTLE_PLAN_CANVAS_BROWSER to its executable');
  const profile = await mkdtemp(resolve(tmpdir(), 'table-dom-'));
  const vite = await createServer({ configFile: false, root: resolve('.'),
    cacheDir: resolve(profile, 'vite'), optimizeDeps: { noDiscovery: true,
      include: ['react', 'react-dom', 'react-dom/client', 'react/jsx-runtime', 'react/jsx-dev-runtime'] },
    server: { host: '127.0.0.1', port: 0, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'table-page', configureServer(server) {
      server.middlewares.use('/table-page', (_request, response) => {
        response.setHeader('Content-Type', 'text/html'); response.end('<!doctype html><title>Synthetic table</title>');
      });
    } }] });
  let browser: ReturnType<typeof spawn> | undefined, endpoint = '', browserPid: number | undefined;
  try {
    await vite.listen();
    const address = vite.httpServer!.address(); assert.ok(address && typeof address !== 'string');
    browser = spawn(edge, ['--headless', '--disable-gpu', '--no-first-run', '--edge-skip-compat-layer-relaunch',
      '--remote-debugging-port=0', `--user-data-dir=${profile}`], { windowsHide: true });
    browser.stdout?.resume(); browser.stderr?.resume();
    endpoint = await browserStage(browser, 'Table Edge startup', async signal => {
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
    await cdp(page, 'Page.navigate', { url: `http://127.0.0.1:${address.port}/table-page` });
    await browserStage(browser, 'Table page navigation', async signal => {
      while (await evaluate('location.pathname') !== '/table-page') await pause(50, undefined, { signal });
    });
    await evaluate(`import('/ui/src/charts/table/Table.dom.fixture.tsx')`);
    const act = (expression: string) => evaluate(`window.tableFixture.${expression}`);
    const snapshot = () => act('snapshot()') as Promise<Snapshot>;
    await browserStage(browser, 'Table DOM assertions', async () => {
      await context.test('a narrowing table hides prioritised columns into the row tooltip and restores them', async () => {
        await act('mount(900)');
        const wide = await snapshot();
        assert.deepEqual(wide.headers, ['Country', 'First measure', 'Second measure', 'Third measure', 'Fourth measure']);
        assert.ok(wide.scrollWidth <= wide.clientWidth + 1, 'a wide table needs no sideways scroll');
        await act('resize(300)');
        const narrow = await snapshot();
        assert.deepEqual(narrow.headers, ['Country', 'First measure'], 'the largest priorities hide first, the later of equal ones first');
        assert.equal(narrow.cells, 2, 'rows drop the hidden cells with their headers');
        assert.ok(narrow.scrollWidth <= narrow.clientWidth + 1, 'the remaining columns fit without sideways scroll');
        await act('focusRow(1)');
        assert.equal((await snapshot()).description, 'Synthetic country 38; Fourth measure r38; Second measure 200,038; Third measure 30,000,038',
          'hidden values follow the widget’s rows, and a value the tooltip already shows is not repeated');
        await act('focusRow(0)');
        assert.match((await snapshot()).description ?? '', /Third measure —$/, 'a missing hidden value reads as its dash');
        await act('resize(900)');
        assert.deepEqual((await snapshot()).headers, wide.headers, 'widening shows every column again');
      });
      await context.test('a table without its own tooltip gains one for hidden columns', async () => {
        await act('mount(300, "hide", false)'); await act('focusRow(1)');
        const described = (await snapshot()).description ?? '';
        assert.equal(described, 'Synthetic country 38; Second measure 200,038; Third measure 30,000,038');
      });
      await context.test('the sorted column and the first column never hide', async () => {
        await act('mount(300, "hide", true, "third", 9)');
        const sorted = (await snapshot()).headers;
        assert.ok(sorted.includes('Third measure'), 'the measure the table is sorted by stays');
        assert.equal(sorted[0], 'Country', 'the row identity stays even with a priority');
      });
      await context.test('columns restored from a narrow mount line up with their cells', async () => {
        await act('mount(300)'); await act('resize(900)');
        const restored = await snapshot();
        assert.equal(restored.headers.length, 5);
        for (const [header, cell] of restored.alignment) assert.ok(cell != null && Math.abs(header - cell) < 1, `header at ${header}, cell at ${cell}`);
      });
      await context.test('a column that becomes narrower is shown again once it fits', async () => {
        await act('mount(560)'); await act('setFourth("240px")');
        assert.ok(!(await snapshot()).headers.includes('Fourth measure'), 'the wide column hides');
        await act('setFourth("60px")');
        assert.ok((await snapshot()).headers.includes('Fourth measure'), 'the narrower column fits again');
      });
      await context.test('hidden values make a fitted tooltip scroll instead of clipping', async () => {
        await act('mount(300)'); await act('setTall()'); await act('focusRow(1)');
        const tall = await snapshot();
        assert.match(tall.description ?? '', /Second measure 200,038/);
        assert.equal(tall.scrollable, true, 'the shared scrolling layer keeps every row reachable');
      });
      await context.test('page keys and the wheel reach the last hidden value of a scrolling tooltip', async () => {
        await act('mount(300)'); await act('setTall()'); await act('focusRow(1)');
        const before = await act('layer()') as { open: boolean; scrollTop: number; rows: string[] };
        assert.ok(before.open && before.scrollTop === 0);
        assert.ok(!before.rows.some(row => row.includes('Third measure')), 'the last hidden value starts below the fold');
        for (let i = 0; i < 6; i++) await act('rowKey(1, "PageDown")');
        const paged = await act('layer()') as { scrollTop: number; clientHeight: number; scrollHeight: number; rows: string[] };
        assert.ok(paged.scrollTop > 0 && paged.scrollTop + paged.clientHeight >= paged.scrollHeight - 1, 'page keys scroll the tooltip to its end');
        assert.ok(paged.rows.some(row => row.includes('Third measure30,000,038')), 'the last hidden value is reached');
        for (let i = 0; i < 6; i++) await act('rowKey(1, "PageUp")');
        assert.equal((await act('layer()') as { scrollTop: number }).scrollTop, 0);
        assert.equal(await act('wheel(5000)'), true, 'the wheel scrolls the tooltip instead of the page');
        assert.ok((await act('layer()') as { rows: string[] }).rows.some(row => row.includes('Third measure30,000,038')));
      });
      await context.test('a shorter window makes a fitted tooltip with hidden values scroll', async () => {
        const viewport = new WebSocket(page);
        const resize = (height: number) => new Promise<void>((done, reject) => {
          const id = height;
          const listener = (event: MessageEvent) => { const result = JSON.parse(String(event.data)); if (result.id !== id) return;
            viewport.removeEventListener('message', listener); if (result.error) reject(new Error(JSON.stringify(result.error))); else done(); };
          viewport.addEventListener('message', listener);
          viewport.send(JSON.stringify({ id, method: 'Emulation.setDeviceMetricsOverride', params: { width: 1000, height, deviceScaleFactor: 1, mobile: false } }));
        });
        await new Promise<void>((done, reject) => { viewport.addEventListener('open', () => done()); viewport.addEventListener('error', () => reject(new Error('Viewport CDP connection failed'))); });
        try {
          await resize(760); await pause(100);
          await act('mount(300)'); await act('setTall(36)'); await act('focusRow(1)');
          assert.equal((await snapshot()).scrollable, false, 'the combined tooltip fits the taller window');
          await resize(640); await pause(200); await act('settle()');
          assert.equal((await snapshot()).scrollable, true, 'the same tooltip scrolls in the shorter window');
        } finally { await resize(900).catch(() => {}); viewport.close(); }
      });
      await context.test('turning the rank column off gives its width back to hidden columns', async () => {
        let width = 0;
        for (let candidate = 420; candidate <= 900 && !width; candidate += 10) {
          await act(`mount(${candidate}, "hide", true, "first", undefined, true)`);
          const ranked = (await snapshot()).headers.filter(Boolean).length;
          await act(`mount(${candidate})`);
          if ((await snapshot()).headers.filter(Boolean).length > ranked) width = candidate;
        }
        assert.ok(width, 'some width hides a column only when ranked');
        await act(`mount(${width})`);
        const unranked = (await snapshot()).headers;
        await act(`mount(${width}, "hide", true, "first", undefined, true)`); await act('setRank(false)');
        assert.deepEqual((await snapshot()).headers, unranked, 'toggling rank off matches a fresh unranked mount');
      });
      await context.test('the scroll setting keeps every column and scrolls sideways', async () => {
        await act('mount(300, "scroll")');
        const scrolled = await snapshot();
        assert.equal(scrolled.headers.length, 5);
        assert.ok(scrolled.scrollWidth > scrolled.clientWidth + 1, 'the table scrolls sideways instead');
      });
      await context.test('group header rows open and close by click and keys, and arrows step in and out', async () => {
        await act('mountTree()');
        const tree = () => act('tree()') as Promise<{ rows: string[]; focused: string | null }>;
        assert.deepEqual((await tree()).rows, ['line0', 'line1+', 'line1:0', 'line2+', 'line2:0', 'line2:1']);
        await act('focusRow(1)'); await act('press("Enter")');
        assert.deepEqual((await tree()).rows, ['line0', 'line1-', 'line2+', 'line2:0', 'line2:1'], 'Enter closes a group');
        await act('press("ArrowRight")');
        assert.equal((await tree()).rows[1], 'line1+', 'ArrowRight opens a closed group');
        await act('press("ArrowRight")');
        assert.equal((await tree()).focused, 'line1:0', 'ArrowRight on an open group steps into its first row');
        await act('press("ArrowLeft")');
        assert.equal((await tree()).focused, 'line1', 'ArrowLeft on a nested row returns to its group');
        await act('press("ArrowLeft")');
        assert.equal((await tree()).rows[1], 'line1-', 'ArrowLeft closes an open group');
        await act('click(2)');
        assert.deepEqual((await tree()).rows, ['line0', 'line1-', 'line2-'], 'a click toggles a group');
        await act('click(0)');
        assert.equal((await tree()).rows[0], 'line0', 'an empty group has nothing to open');
        await act('mountTree(250)');
        const narrow = await snapshot();
        assert.equal(narrow.headers[0], 'Country', 'the identity column stays');
        assert.ok(narrow.headers.length < 4 && !narrow.headers.includes('Second measure'), 'prioritised columns hide from a nested table too');
        await act('focusRow(1)');
        const described = (await snapshot()).description ?? '';
        assert.match(described, /^Synthetic line 1; /, 'a group header row is titled by its identity');
        assert.match(described, /Second measure 1/, 'its hidden values join its tooltip like any row');
        await act('mountTree(250, true)'); await act('focusRow(1)');
        assert.equal((await act('layer()') as { scrollTop: number }).scrollTop, 0);
        for (let i = 0; i < 6; i++) await act('rowKey(1, "PageDown")');
        const paged = await act('layer()') as { scrollTop: number; rows: string[] };
        assert.ok(paged.scrollTop > 0 && paged.rows.some(row => row.includes('Second measure1')), 'the tall tooltip of a group header row pages to its hidden values');
        assert.equal((await tree()).rows[1], 'line1+', 'page keys scroll the tooltip and leave the group as it was');
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
