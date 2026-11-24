import assert from 'node:assert/strict';
import test from 'node:test';
import { withNativePage } from '../../../tools/native-page.ts';





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




