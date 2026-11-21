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
    const charts = Object.entries(played.charts).filter(([widget]) => widget !== 'feed');
    for (const [chart, ms] of charts)
      t.diagnostic(`${chart}: ${ms.length} paints, p50 ${percentile(ms, 0.5).toFixed(2)} ms, p95 ${percentile(ms, 0.95).toFixed(2)} ms`);
    t.diagnostic(`over ${played.frames} frames: ${played.asked.layout} layout reads, ${played.asked.measure} texts measured, ${played.feedCommits} feed commits`);
    assert.ok(charts.length === 7 && charts.every(([, ms]) => ms.length >= played.frames - 2),
      `every chart painted every frame as the head moved: ${JSON.stringify(Object.fromEntries(Object.entries(played.charts).map(([chart, ms]) => [chart, ms.length])))}`);
    assert.equal(played.asked.layout, 0, `neither a frame of playback nor a commit of the feed reads layout back: ${played.asked.layout} reads, ${played.feedCommits} feed commits`);
    assert.ok(played.asked.measure <= played.frames, `a frame measures only text it has not measured: ${played.asked.measure} in ${played.frames} frames`);
  }));
});

test('a log committing new rows reads no layout back, in the commit or the frames after', async t => {
  await withNativePage('paint-commits', '/ui/src/charts/paintCost.dom.fixture.tsx', page => page.stage('commits', async () => {
    const played = await page.evaluate<{ layout: number; commits: number }>('window.paintCost.commits(20)');
    t.diagnostic(`${played.commits} commits, ${played.layout} layout reads`);
    assert.ok(played.commits >= 20, `the log committed each time its rows changed: ${played.commits}`);
    assert.equal(played.layout, 0, `no layout read back while committing: ${played.layout} reads in ${played.commits} commits`);
  }));
});

test('a virtual list committing new rows reads no layout back, in the commit or the frames after', async t => {
  await withNativePage('paint-virtual', '/ui/src/charts/paintCost.dom.fixture.tsx', page => page.stage('virtual', async () => {
    const played = await page.evaluate<{ layout: number; commits: number; shown: number }>('window.paintCost.virtualCommits(20)');
    t.diagnostic(`${played.layout} layout reads; ${played.commits} commits; ${played.shown} rows shown`);
    assert.ok(played.commits <= 20, `one commit a change of rows, none for the gap or a scroll inside the rows drawn: ${played.commits} in 20`);
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
      const charts = (summary.forced ?? []).filter(entry => /\((draw|TimeChart|Timeline|race|RaceChart|Sunburst|textWidth)[^:]*\.tsx?:/.test(entry.cause));
      assert.deepEqual(charts, [], `a chart's drawing forces no style or layout pass: ${JSON.stringify(summary.forced)}`);
    } finally { session.close(); }
  }, 120_000));
});

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
      assert.ok(played.started.other <= played.frames, `only an item moving apart from the rows restarts its slide: ${played.started.other}`);
      assert.ok(restyled <= 24 * played.frames, `a frame restyles the rows it reveals and the glide, not every row drawn: ${restyled} elements in ${played.frames} frames`);
      assert.ok(played.started.glide >= played.revealed * 0.9, `the rows still glide each frame a row is revealed: ${played.started.glide} glides`);
      assert.ok(played.scroll.height <= played.scroll.list + 32, `the glide never grows the scroll past the list: ${JSON.stringify(played.scroll)}`);
    } finally { session.close(); }
  }, 120_000));
});

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

test('a live table bar moves to a new value drawn by the engine, only rising, with no CSS transition on it', async () => {
  await withNativePage('paint-glide', '/ui/src/charts/paintCost.dom.fixture.tsx', page => page.stage('glide', async () => {
    const played = await page.evaluate<{ before: number; samples: number[]; transition: string }>('window.paintCost.barGlide()');
    assert.ok(Math.abs(played.before - 0.2) < 1e-9, `the bar shows its value at rest: ${played.before}`);
    assert.ok(!played.transition.split(', ').some(name => ['transform', 'all'].includes(name)), `no transform transition: ${played.transition}`);
    assert.ok(played.samples.every((value, at) => at === 0 || value >= played.samples[at - 1] - 1e-9), 'it only rises toward the new value');
    assert.ok(Math.abs(played.samples.at(-1)! - 0.8) < 1e-9, `it ends at the new value: ${played.samples.at(-1)}`);
  }, 120_000));
});

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
