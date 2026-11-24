import type { ChartLayer } from './TimeChart';
import type { ChartTokens } from './tokens';
import type { PlotFrame } from './frame';

export interface Span { firstHour: number; lastHour: number }

export const APPROXIMATE_ALPHA = 0.55;
export const APPROXIMATE_DASH: readonly number[] = [4, 4];
export const PENDING_ALPHA = 0.12;

export function spanRanges(spans: readonly Span[] | undefined, from: number, to: number, until: number | null = null): [number, number][] {
  const end = Math.min(to, until == null ? Infinity : until + 1);
  const ranges: [number, number][] = [];
  for (const span of spans ?? []) {
    const first = Math.max(from, span.firstHour), last = Math.min(end, span.lastHour + 1);
    if (first < last) ranges.push([first, last]);
  }
  return ranges;
}

export function fillPendingBand(ctx: CanvasRenderingContext2D, tokens: ChartTokens, from: number, to: number, top: number, height: number): void {
  if (!(to > from) || !(height > 0)) return;
  ctx.save(); ctx.fillStyle = tokens.muted; ctx.globalAlpha = PENDING_ALPHA;
  ctx.fillRect(from, top, to - from, height); ctx.restore();
}

export function pendingLayer(pending: readonly Span[] | undefined): ChartLayer {
  return { draw(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, state: { playhead: number | null }) {
    for (const [from, to] of spanRanges(pending, frame.x[0], frame.x[1], state.playhead))
      fillPendingBand(ctx, tokens, frame.toX(from), frame.toX(to), frame.plotTop, frame.plotHeight);
  } };
}
