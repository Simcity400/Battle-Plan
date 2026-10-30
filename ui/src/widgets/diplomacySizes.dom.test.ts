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
import type { fixture } from './diplomacySizes.dom.fixture';

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

test('the Diplomacy widgets’ chart forms fit their cards at every size', async context => {
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
        response.setHeader('Content-Type', 'text/html'); response.end('<!doctype html><title>Synthetic diplomacy widgets</title>');
      });
    } }] });
  let browser: ReturnType<typeof spawn> | undefined, endpoint = '', browserPid: number | undefined;
  try {
    await vite.listen();
    const address = vite.httpServer!.address(); assert.ok(address && typeof address !== 'string');
    browser = spawn(edge, ['--headless', '--disable-gpu', '--no-first-run', '--edge-skip-compat-layer-relaunch',
      '--remote-debugging-port=0', `--user-data-dir=${profile}`], { windowsHide: true });
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
    await evaluate(`import('/ui/src/widgets/diplomacySizes.dom.fixture.tsx')`);
    const act = (expression: string) => evaluate(`window.diplomacySizesFixture.${expression}`);
    const snapshot = () => act('snapshot()') as Promise<Snapshot>;
    await browserStage(browser, 'Diplomacy sizes DOM assertions', async () => {
      // A card at the smallest default minimum (4 x 5 grid cells), a small one and a large one.
      const sizes = [[300, 220], [420, 320], [1200, 640]] as const;
      for (const form of await act('forms()') as string[]) await context.test(`${form} stays inside its card at minimum, small and large sizes`, async () => {
        for (const [width, height] of sizes) {
          await act(`mount(${JSON.stringify(form)}, ${width}, ${height})`);
          const shown = await snapshot();
          assert.ok(shown.cardScrollWidth <= shown.cardWidth + 1, `${form} at ${width}x${height} overflows its card sideways (${shown.cardScrollWidth} > ${shown.cardWidth})`);
          assert.ok(shown.text.length > 0 || shown.canvas, `${form} at ${width}x${height} draws something`);
          if (form === 'faction-pooled-strength') assert.ok(shown.drawnMembers > 0 && shown.drawnMembers < 80, `a 5,000-member pool mounts only the rows in view (${shown.drawnMembers}), so only their flags load`);
          if (width >= 420) for (const extra of shown.tableScroll) assert.ok(extra <= 1, `${form} at ${width}x${height}: its table scrolls sideways by ${extra}px instead of hiding columns`);
        }
      });
      // A whole widget in its production hooks, reading from a client the test answers.
      await evaluate(`import('/ui/src/widgets/diplomacyWidgets.dom.fixture.tsx')`);
      const widget = (expression: string) => evaluate(`window.widgetsFixture.${expression}`);
      await context.test('a mounted pooled-strength list names a member loading only while its read is going, and reads no flags before preparation chooses them', async () => {
        await widget(`mount('manpower', 100)`);
        assert.deepEqual(await widget('members()'), ['Loading name…', 'Loading name…'], `names still being read are loading: ${await widget('text()')}`);
        await widget('release()');
        assert.deepEqual(await widget('members()'), ['Synthetic country AAA', 'Country name unavailable'],
          'once the read completes, a name the recording lacks is unavailable, not loading for ever');
        // This recording holds no country timeline: a flag is what preparation chose, so none is asked of the recording.
        assert.equal(await widget('flagRequests()'), 0, 'the mounted rows wait for preparation to choose their flags');
        await widget(`mount('resources', 100)`);
        await widget(`mount('resources', 120)`);
        assert.match(await widget('text()') as string, /Synthetic oil/);
        assert.equal(await widget('flagRequests()'), 0);
      });
      // RRR passes from its earlier holder (magenta) to a later one (blue) at hour 24, inside
      // one 720-hour look-sampling interval of the branch start.
      const hours = async (tag: string) => [...new Set(await widget(`styleHours(${JSON.stringify(tag)})`) as number[])].sort((x, y) => x - y);
      await context.test('a mounted pair history names and colours its countries as they were at the hour chosen', async () => {
        await widget('reset()');
        await widget(`show('diplomatic-relations', { time: 48, countries: { kind: 'tags', tags: ['RRR', 'SSS'] }, options: { show: 'pair', pair: ${JSON.stringify(':5b::22:RRR:22::2c::22:SSS:22::2c:48:5d:')} } })`);
        assert.deepEqual(await hours('RRR'), [48], `read at the chosen hour itself, never a sample before the tag passed on: ${await widget('text()')}`);
        assert.ok((await widget('labels()') as string[]).includes('Later holder and Synthetic SSS opinions over time'), JSON.stringify(await widget('labels()')));
        assert.deepEqual([await widget(`drawn('blue')`), await widget(`drawn('green')`), await widget(`drawn('magenta')`)], [true, true, false],
          'the lines are the later holder’s and SSS’s colours, never the earlier holder’s');
      });
      await context.test('a mounted war contribution colours its members as they were at the war’s read hour', async () => {
        await widget('reset()');
        await widget(`show('war-contribution', { time: 48, countries: { kind: 'tags', tags: ['SSS'] } })`);
        assert.deepEqual(await hours('RRR'), [48], `read at the war’s read hour itself: ${await widget('text()')}`);
        assert.deepEqual([await widget(`drawn('blue')`), await widget(`drawn('green')`), await widget(`drawn('magenta')`)], [true, true, false]);
      });
      await context.test('a mounted volunteer diagram asks only for the chosen country’s tags across the branch', async () => {
        await widget('reset()');
        await widget(`show('volunteers', { countries: { kind: 'tags', tags: ['RRR'] } })`);
        assert.deepEqual(await widget(`requestTags('volunteers-expeditionaries')`), [['QQQ', 'RRR']], `both holders of RRR, with the earlier one’s later tag: ${await widget('text()')}`);
        await widget('reset()');
        await widget(`show('volunteers', { countries: { kind: 'all' } })`);
        assert.deepEqual(await widget(`requestTags('volunteers-expeditionaries')`), [null], 'only All asks for every link');
      });
      await context.test('a mounted volunteer diagram of an hourly group with no tags to ask for tells unknown, empty and still-preparing apart', async () => {
        const hourly = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false };
        for (const [recording, kind, text] of [['gap', 'unavailable', 'This group’s membership was not recorded at this date.'],
          ['empty', 'empty', 'This group has no members at this date.'], ['partial', 'loading', 'This range is still being prepared.']] as const) {
          await widget('reset()');
          await widget(`show('volunteers', { recording: '${recording}', time: 48, countries: ${JSON.stringify(hourly)} })`);
          const shown = await widget('text()') as string;
          assert.ok(shown.includes(text), `${recording}: ${kind} reads “${text}”, not “${shown}”`);
          assert.doesNotMatch(shown, /not in this campaign|unavailable in this campaign/, `${recording}: never absence`);
          assert.deepEqual(await widget(`requestTags('volunteers-expeditionaries')`), [], `${recording}: nothing is requested, never every link`);
        }
        // Chosen countries, while the groups are still being prepared, are loading, not unavailable.
        await widget('reset()');
        await widget(`show('volunteers', { recording: 'partial', countries: { kind: 'tags', tags: ['RRR'] } })`);
        assert.match(await widget('text()') as string, /still being prepared/);
        assert.deepEqual(await widget(`requestTags('volunteers-expeditionaries')`), []);
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
