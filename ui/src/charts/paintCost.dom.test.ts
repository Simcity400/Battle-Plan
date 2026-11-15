import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';

type Played = { asked: { layout: number; measure: number }; frames: number; charts: Record<string, number[]>; feedCommits: number };
const percentile = (values: readonly number[], share: number) => {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor(share * sorted.length))] ?? 0;
};

test('time charts and an event feed following a playing head paint each frame without reading layout or measuring text again', async t => {
  await withNativePage('paint-cost', '/ui/src/charts/paintCost.dom.fixture.tsx', page => page.stage('paint cost', async () => {
    const act = <T,>(expression: string) => page.evaluate<T>(`window.paintCost.${expression}`);
    await act('mount()');
    const played = await act<Played>('play(240, 15)');
    // The feed paints only its rows' entry layer, while rows enter: the charts are the rest.
    const charts = Object.entries(played.charts).filter(([widget]) => widget !== 'feed');
    for (const [chart, ms] of charts)
      t.diagnostic(`${chart}: ${ms.length} paints, p50 ${percentile(ms, 0.5).toFixed(2)} ms, p95 ${percentile(ms, 0.95).toFixed(2)} ms`);
    t.diagnostic(`over ${played.frames} frames: ${played.asked.layout} layout reads, ${played.asked.measure} texts measured, ${played.feedCommits} feed commits`);
    assert.ok(charts.length === 7 && charts.every(([, ms]) => ms.length >= played.frames - 2),
      `every chart painted every frame as the head moved: ${JSON.stringify(Object.fromEntries(Object.entries(played.charts).map(([chart, ms]) => [chart, ms.length])))}`);
    // Run 16's Army: each commit of a log read its view and its scroll back, laying out the rows React had just written
    // (325 ms each under the trace). The view is read by its resize observer, the scroll never read back.
    assert.equal(played.asked.layout, 0, `neither a frame of playback nor a commit of the feed reads layout back: ${played.asked.layout} reads, ${played.feedCommits} feed commits`);
    assert.ok(played.asked.measure <= played.frames, `a frame measures only text it has not measured: ${played.asked.measure} in ${played.frames} frames`);
  }));
});

// Run 16's Army: each commit of a log read its view and wrote and read back its scroll, laying out the rows React had just
// written (325 ms a commit under the trace). The view is read by its resize observer, and the scroll is never read back.
test('a log committing new rows reads no layout back, in the commit or the frames after', async t => {
  await withNativePage('paint-commits', '/ui/src/charts/paintCost.dom.fixture.tsx', page => page.stage('commits', async () => {
    const played = await page.evaluate<{ layout: number; commits: number }>('window.paintCost.commits(20)');
    t.diagnostic(`${played.commits} commits, ${played.layout} layout reads`);
    assert.ok(played.commits >= 20, `the log committed each time its rows changed: ${played.commits}`);
    assert.equal(played.layout, 0, `no layout read back while committing: ${played.layout} reads in ${played.commits} commits`);
  }));
});

// Run 16's Politics: a virtual list measured its view's rectangles and computed style in every commit, and again in the
// frame after (its new observers' first reports), laying out what React had just written. It reads them only as the
// page has just been laid out (its observers) or as the reader scrolls.
test('a virtual list committing new rows reads no layout back, in the commit or the frames after', async t => {
  await withNativePage('paint-virtual', '/ui/src/charts/paintCost.dom.fixture.tsx', page => page.stage('virtual', async () => {
    const played = await page.evaluate<{ layout: number; shown: number }>('window.paintCost.virtualCommits(20)');
    t.diagnostic(`${played.layout} layout reads; ${played.shown} rows shown`);
    assert.ok(played.shown > 5, `the list shows the rows in its view: ${played.shown}`);
    assert.equal(played.layout, 0, `no layout read back while committing: ${played.layout} reads in 20 commits`);
  }));
});

test('a widget off screen hears no frame of the playing clock and hears one the moment it is back; its listeners are timed as its painting', async () => {
  await withNativePage('widget-frames', '/ui/src/charts/paintCost.dom.fixture.tsx', page => page.stage('widget frames', async () => {
    const gate = await page.evaluate<{ offScreen: { shown: number; hidden: number; painted: string[] }; shownAgain: number; after: { shown: number; hidden: number };
      sameMethod: boolean; hour: boolean }>('window.paintCost.gate()');
    assert.ok(gate.offScreen.shown >= 25, `the widget on screen heard every frame: ${gate.offScreen.shown}`);
    assert.equal(gate.offScreen.hidden, 0, 'the widget off screen heard none');
    assert.deepEqual(gate.offScreen.painted, ['shown'], 'only the widget on screen was timed painting');
    assert.equal(gate.shownAgain, 1, 'back on screen, it heard one frame at once');
    assert.ok(gate.after.hidden >= 5, `and every frame after: ${gate.after.hidden}`);
    assert.ok(gate.sameMethod && gate.hour, 'the rest of the clock is the page clock, its methods one value each');
  }));
});

// Run 15: World tension and Ruling party painted 6-11 ms a frame. Their stacks were drawn as two-point pieces, a fill and
// a stroke per band per pixel column every frame the head moved.
test('a stacked chart following the head fills and outlines each band once a frame, not once a pixel column', async t => {
  await withNativePage('paint-stack', '/ui/src/charts/paintCost.dom.fixture.tsx', page => page.stage('stack', async () => {
    const bands = 8;
    const played = await page.evaluate<{ frames: number; fills: number; strokes: number; paints: number[] }>(`window.paintCost.stack(${bands}, 120, 15)`);
    t.diagnostic(`${played.frames} frames: ${(played.fills / played.frames).toFixed(1)} fills, ${(played.strokes / played.frames).toFixed(1)} strokes a frame; `
      + `paint p50 ${percentile(played.paints, 0.5).toFixed(2)} ms, p95 ${percentile(played.paints, 0.95).toFixed(2)} ms`);
    assert.ok(played.paints.length >= played.frames - 2, `the stack painted every frame the head moved: ${played.paints.length} of ${played.frames}`);
    assert.ok(played.fills <= 2 * bands * played.frames, `each band filled once a frame for its one stretch: ${played.fills} fills in ${played.frames} frames`);
  }));
});

type TraceSummary = { marked: boolean; frames: number | null; main: number; kinds: Record<string, number>; described?: unknown;
  forced?: { cause: string; count: number; ms: number }[]; invalidated?: { cause: string; count: number }[] };
type TraceModule = {
  TRACE_MARKS: { from: string; to: string };
  startTrace(cdp: unknown): Promise<() => Promise<unknown[]>>;
  summariseTrace(events: unknown[]): TraceSummary;
  summaryNodes(summary: TraceSummary): number[];
  describeNodes(cdp: unknown, ids: number[]): Promise<unknown>;
  perFrame(summary: TraceSummary): Record<string, unknown>;
};
type CdpModule = { cdpSession(url: string): Promise<{ close(): void }> };

test("the suite's browser trace of a playing window names the main thread's rendering, script and GC a frame, and what dirtied layout", async t => {
  await withNativePage('paint-trace', '/ui/src/charts/paintCost.dom.fixture.tsx', page => page.stage('trace', async () => {
    // The suite's own modules, as the playback phase uses them on the app's page.
    const trace = await import(new URL('../../../tools/e2e/trace.mjs', import.meta.url).href) as TraceModule;
    const { cdpSession } = await import(new URL('../../../tools/e2e/cdp.mjs', import.meta.url).href) as CdpModule;
    const session = await cdpSession(page.page);
    try {
      await page.evaluate('window.paintCost.mount()');
      const stop = await trace.startTrace(session);
      await page.evaluate(`window.paintCost.play(120, 15, ${JSON.stringify(trace.TRACE_MARKS)})`);
      const events = await stop();
      const summary = trace.summariseTrace(events);
      summary.described = await trace.describeNodes(session, trace.summaryNodes(summary));
      t.diagnostic(`${events.length} events, ${summary.frames} frames: ${JSON.stringify(trace.perFrame(summary))}`);
      assert.ok(summary.marked, "the window is cut at the page's marks");
      assert.ok((summary.frames ?? 0) >= 60, `the frames played are counted: ${summary.frames}`);
      assert.ok(summary.kinds.script > 0, "the charts' painting is script on the main thread");
      assert.ok(summary.kinds.style + summary.kinds.layout + summary.kinds.prePaint + summary.kinds.paint > 0, "the browser's rendering is named");
      assert.ok(summary.main >= summary.kinds.script, 'what the main thread was busy with holds its parts');
      // Run 16: setting a font on a canvas in the page brought the whole page's style up to date first, 160 times in a
      // Politics window (8.4 ms of style a frame): charts draw into their canvas's offscreen canvas instead.
      const charts = (summary.forced ?? []).filter(entry => /\((draw|TimeChart|Timeline|race|RaceChart|Sunburst|textWidth)[^:]*\.tsx?:/.test(entry.cause));
      assert.deepEqual(charts, [], `a chart's drawing forces no style or layout pass: ${JSON.stringify(summary.forced)}`);
    } finally { session.close(); }
  }, 120_000));
});

// PF2's trace of the feed: each frame that revealed a row wrote a new translateY on every row drawn, which restarted every
// row's 450 ms slide, so every row recalculated its style every frame (6.1 ms a frame, 2.9 with row transitions off). The
// rows' entry (fade and flash) is off in the traced window: it restyles only the rows entering, and is not what moves them.
test('a log revealing rows each frame glides them by one offset: no row restarts its slide, and the frame restyles only what changed', async t => {
  await withNativePage('paint-glide', '/ui/src/charts/paintCost.dom.fixture.tsx', page => page.stage('glide', async () => {
    const trace = await import(new URL('../../../tools/e2e/trace.mjs', import.meta.url).href) as TraceModule & {
      traceSpans(events: unknown[]): { name: string; ts: number; args?: { elementCount?: number } }[] };
    const { cdpSession } = await import(new URL('../../../tools/e2e/cdp.mjs', import.meta.url).href) as CdpModule;
    const session = await cdpSession(page.page);
    try {
      const stop = await trace.startTrace(session);
      const played = await page.evaluate<{ frames: number; revealed: number; started: { rows: number; glide: number; other: number }; scroll: { height: number; list: number; view: number } }>(
        `window.paintCost.glide(120, 15, ${JSON.stringify(trace.TRACE_MARKS)})`);
      const events = await stop() as { name: string; ts: number; cat?: string }[];
      const markAt = (name: string) => events.find(event => event.name === name && String(event.cat ?? '').includes('user_timing'))!.ts;
      const [from, to] = [markAt(trace.TRACE_MARKS.from), markAt(trace.TRACE_MARKS.to)];
      const restyled = trace.traceSpans(events).filter(span => span.name === 'UpdateLayoutTree' && span.ts >= from && span.ts <= to)
        .reduce((sum, span) => sum + (span.args?.elementCount ?? 0), 0);
      const summary = trace.summariseTrace(events);
      t.diagnostic(`${played.frames} frames, ${played.revealed} revealing a row; transform transitions started: ${played.started.rows} on rows, ${played.started.glide} on the glide, ${played.started.other} else; `
        + `${(restyled / played.frames).toFixed(1)} elements restyled, style ${(summary.kinds.style / played.frames).toFixed(2)} ms a frame; scroll ${JSON.stringify(played.scroll)}`);
      assert.ok(played.revealed >= played.frames * 0.9, `rows were revealed nearly every frame: ${played.revealed} of ${played.frames}`);
      assert.equal(played.started.rows, 0, 'revealing rows above them restarts no row\'s slide');
      // The month's divider above the rows revealed under it holds still against the glide: one element a frame.
      assert.ok(played.started.other <= played.frames, `only an item moving apart from the rows restarts its slide: ${played.started.other}`);
      assert.ok(restyled <= 24 * played.frames, `a frame restyles the rows it reveals and the glide, not every row drawn: ${restyled} elements in ${played.frames} frames`);
      assert.ok(played.started.glide >= played.revealed * 0.9, `the rows still glide each frame a row is revealed: ${played.started.glide} glides`);
      assert.ok(played.scroll.height <= played.scroll.list + 32, `the glide never grows the scroll past the list: ${JSON.stringify(played.scroll)}`);
    } finally { session.close(); }
  }, 120_000));
});

// Run 16's Overview: Game score's and Scoreboard's bars glided by their width, so every bar laid the page out every frame
// its value moved (5,870 layout invalidations in four seconds). They glide by a scale on the compositor instead.
test('a table whose live bars move every frame lays nothing out for them', async t => {
  await withNativePage('paint-bars', '/ui/src/charts/paintCost.dom.fixture.tsx', page => page.stage('bars', async () => {
    const trace = await import(new URL('../../../tools/e2e/trace.mjs', import.meta.url).href) as TraceModule;
    const { cdpSession } = await import(new URL('../../../tools/e2e/cdp.mjs', import.meta.url).href) as CdpModule;
    const session = await cdpSession(page.page);
    try {
      const stop = await trace.startTrace(session);
      const played = await page.evaluate<{ bars: number }>(`window.paintCost.tableBars(60, ${JSON.stringify(trace.TRACE_MARKS)})`);
      const summary = trace.summariseTrace(await stop());
      const bars = (summary.invalidated ?? []).filter(entry => entry.cause.startsWith('layout:') && entry.cause.includes('table-cellbar'));
      t.diagnostic(`${played.bars} bars; invalidated: ${JSON.stringify(summary.invalidated)}`);
      assert.ok(played.bars >= 10, `the table shows its bars: ${played.bars}`);
      assert.deepEqual(bars, [], 'a bar moving lays nothing out');
    } finally { session.close(); }
  }, 120_000));
});

// PF3 left the rows' entry on the main thread: with the fade and flash on, ~57 rows sat inside their 700 ms entry at once
// and each was restyled every frame (~245 elements a frame), however few rows the frame revealed.
test('rows entering a revealing log fade and flash in on the compositor: a frame restyles the rows it reveals, not every row still entering', async t => {
  await withNativePage('paint-entry', '/ui/src/charts/paintCost.dom.fixture.tsx', page => page.stage('entry', async () => {
    const trace = await import(new URL('../../../tools/e2e/trace.mjs', import.meta.url).href) as TraceModule & {
      traceSpans(events: unknown[]): { name: string; ts: number; args?: { elementCount?: number } }[] };
    const { cdpSession } = await import(new URL('../../../tools/e2e/cdp.mjs', import.meta.url).href) as CdpModule;
    const session = await cdpSession(page.page);
    try {
      const stop = await trace.startTrace(session);
      const played = await page.evaluate<{ frames: number; revealed: number; entering: number; layer: number; animated: number }>(
        `window.paintCost.glide(120, 15, ${JSON.stringify(trace.TRACE_MARKS)}, true)`);
      const events = await stop() as { name: string; ts: number; cat?: string }[];
      const markAt = (name: string) => events.find(event => event.name === name && String(event.cat ?? '').includes('user_timing'))!.ts;
      const [from, to] = [markAt(trace.TRACE_MARKS.from), markAt(trace.TRACE_MARKS.to)];
      const restyled = trace.traceSpans(events).filter(span => span.name === 'UpdateLayoutTree' && span.ts >= from && span.ts <= to)
        .reduce((sum, span) => sum + (span.args?.elementCount ?? 0), 0);
      const summary = trace.summariseTrace(events);
      t.diagnostic(`${played.frames} frames, ${played.revealed} revealing a row, ${played.entering} rows entering at the end, ${played.animated} animations running; `
        + `${(restyled / played.frames).toFixed(1)} elements restyled, style ${(summary.kinds.style / played.frames).toFixed(2)} ms a frame`);
      assert.ok(played.revealed >= played.frames * 0.9, `rows were revealed nearly every frame: ${played.revealed} of ${played.frames}`);
      assert.ok(played.entering >= 20, `many rows are inside their entry at once, as at 1,440 h/s: ${played.entering}`);
      assert.equal(played.layer, 2, 'the shared layer draws their flash under them and their fade over them');
      assert.ok(restyled <= 24 * played.frames, `a frame restyles the rows it reveals and the glide, not every row entering: ${restyled} elements in ${played.frames} frames`);
    } finally { session.close(); }
  }, 120_000));
});
