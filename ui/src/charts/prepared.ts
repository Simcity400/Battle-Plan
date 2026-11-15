import type { ChartLayer } from './TimeChart';
import type { ChartTokens } from './tokens';
import type { PlotFrame } from './frame';

/** Inclusive recorded hours, as replies carry `approximate` and `pending` spans. */
export interface Span { firstHour: number; lastHour: number }

/** Fades approximate geometry as the line kit's dashed strokes do. */
export const APPROXIMATE_ALPHA = 0.55;
export const APPROXIMATE_DASH: readonly number[] = [4, 4];
/** The light band behind hours still being prepared. */
export const PENDING_ALPHA = 0.12;

/** The parts of `spans` inside the half-open hour range `from` to `to`, as
 * half-open hour ranges for drawing, in order; a playhead `until` also ends
 * them after its hour. */
export function spanRanges(spans: readonly Span[] | undefined, from: number, to: number, until: number | null = null): [number, number][] {
  const end = Math.min(to, until == null ? Infinity : until + 1);
  const ranges: [number, number][] = [];
  for (const span of spans ?? []) {
    const first = Math.max(from, span.firstHour), last = Math.min(end, span.lastHour + 1);
    if (first < last) ranges.push([first, last]);
  }
  return ranges;
}

/** Whether any span holds an hour from `first` through `last`. */
export const overlapsSpans = (spans: readonly Span[] | undefined, first: number, last: number) =>
  !!spans?.some(span => span.firstHour <= last && span.lastHour >= first);

/** What a chart's key names: the approximate style when it draws approximate
 * spans, and the pending band when it draws hours still being prepared. */
export function preparedKeyItems(approximate: readonly Span[] | undefined, pending: readonly Span[] | undefined,
  loading?: readonly Span[]): ('approximate' | 'pending' | 'loading')[] {
  return [...(approximate?.length ? ['approximate' as const] : []), ...(pending?.length ? ['pending' as const] : []),
    ...(loading?.length ? ['loading' as const] : [])];
}

/** Paints the light band of hours still being prepared from `from` to `to` pixels. */
export function fillPendingBand(ctx: CanvasRenderingContext2D, tokens: ChartTokens, from: number, to: number, top: number, height: number): void {
  if (!(to > from) || !(height > 0)) return;
  ctx.save(); ctx.fillStyle = tokens.muted; ctx.globalAlpha = PENDING_ALPHA;
  ctx.fillRect(from, top, to - from, height); ctx.restore();
}

/** Fades drawn geometry inside a rectangle and gives it the dashed approximate
 * edge: the rectangle keeps its fill, seen through the surface. */
export function markApproximate(ctx: CanvasRenderingContext2D, tokens: ChartTokens, left: number, top: number, width: number, height: number): void {
  if (!(width > 0) || !(height > 0)) return;
  ctx.save();
  ctx.fillStyle = tokens.surface; ctx.globalAlpha = 1 - APPROXIMATE_ALPHA;
  ctx.fillRect(left, top, width, height);
  ctx.globalAlpha = 1; ctx.strokeStyle = tokens.muted; ctx.lineWidth = 1; ctx.setLineDash([...APPROXIMATE_DASH]);
  ctx.strokeRect(left + 0.5, top + 0.5, Math.max(0, width - 1), Math.max(0, height - 1));
  ctx.restore();
}

/** Bands the hours a reply names pending (its root `pending`) on a time chart,
 * through the playhead: there is no geometry there yet. */
export function pendingLayer(pending: readonly Span[] | undefined): ChartLayer {
  return { draw(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, state: { playhead: number | null }) {
    for (const [from, to] of spanRanges(pending, frame.x[0], frame.x[1], state.playhead))
      fillPendingBand(ctx, tokens, frame.toX(from), frame.toX(to), frame.plotTop, frame.plotHeight);
  } };
}
