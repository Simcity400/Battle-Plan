import { summaryStack } from './chartSummary.ts';
import type { ChartSummary } from './chartSummary';
import { drawBands } from './draw.ts';
import { forEachCompactStackStretch, stackEdges } from './stack.ts';
import type { ChartLayer } from './TimeChart';

/** Stacked bands of lines read as the chart draws them (`read`: each band's summary bottom to top, null until every
 * band has one), never through React: a tile landing repaints, the head moves without a render. A tiled line's bins
 * run past the head, so the bands stop at the head's bin and that bin is drawn from its first hour to the head at the
 * values at the head (`valueAt`, the followed values while following); an untiled view is drawn to the head. The
 * stack is built once per set of summaries. */
export function tiledStackLayer(read: () => readonly ChartSummary[] | null, colors: () => readonly string[], extent: () => [number, number] | null = () => null): ChartLayer {
  let built: { from: readonly ChartSummary[]; stack: ReturnType<typeof summaryStack> } | null = null;
  const stackOf = (summaries: readonly ChartSummary[]) => {
    if (!built || built.from.length !== summaries.length || built.from.some((summary, at) => summary !== summaries[at])) {
      built = { from: summaries, stack: summaryStack(summaries) };
    }
    return built.stack;
  };
  return {
    draw: (ctx, frame, tokens, state) => {
      const summaries = read();
      if (!summaries?.length) return;
      const fill = colors();
      const until = Math.min(frame.x[1], state.playhead ?? frame.x[1]);
      const hoursPerPixel = (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth);
      const tiles = summaries[0].tiles;
      const cut = tiles && state.playhead !== null ? Math.floor(until / tiles.binHours) * tiles.binHours : null;
      const { model, sample } = stackOf(summaries);
      forEachCompactStackStretch(model, frame.x[0], cut === null ? until : Math.min(until, cut - 1),
        hoursPerPixel / 2, segment => drawBands(ctx, frame, tokens, segment.hours, segment.bands, fill, segment), sample);
      if (cut === null || cut > until) return;
      const head = headValues(summaries, until);
      if (!head) return;
      const edges = stackEdges(head);
      const from = Math.max(frame.x[0], cut - 1);
      drawBands(ctx, frame, tokens, [from, until], edges.map(edge => ({ low: [edge.low, edge.low], high: [edge.high, edge.high] })), fill);
    },
    extent: () => extent(),
  };
}

/** Each band's value at `hour` as its tiles give it at the head; null unless every band has one. */
export function headValues(summaries: readonly ChartSummary[], hour: number): number[] | null {
  const values: number[] = [];
  for (const summary of summaries) {
    const value = summary.tiles?.valueAt(hour) ?? null;
    if (value === null || !Number.isFinite(value)) return null;
    values.push(value);
  }
  return values;
}
