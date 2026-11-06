import type { PlotFrame } from './frame';
import { drawUnknownHatch } from './hatch.ts';
import type { ChartTokens } from './tokens';
import type { Range } from './view';

/** Hours the recording lost that the families of a reply went on past, inclusive. Nothing
 * drawn from that reply crosses them: a line ends at their first hour and goes on after their
 * last. A chart takes the spans of the replies it draws, never another product's. */
export interface LostSpan { firstHour: number; lastHour: number }

/** The lost span holding `hour`, if any. */
export function lostAt(gaps: readonly LostSpan[], hour: number): LostSpan | null {
  for (const gap of gaps) {
    if (gap.firstHour > hour) return null;
    if (hour <= gap.lastHour) return gap;
  }
  return null;
}

/** How a chart places hours across its plot: from `left` to `right`, showing hours `x`. */
export interface HourAxis { toX(hour: number): number; x: Range; left: number; right: number }

export const frameAxis = (frame: PlotFrame): HourAxis => ({ toX: hour => frame.toX(hour), x: frame.x, left: frame.plotLeft, right: frame.plotLeft + frame.plotWidth });

/** The plot's horizontal pixels of each span in view: from its first hour to the hour after its last. */
function columns(axis: HourAxis, gaps: readonly LostSpan[]): Array<[number, number]> {
  const shown: Array<[number, number]> = [];
  for (const gap of gaps) {
    if (gap.lastHour + 1 < axis.x[0] || gap.firstHour > axis.x[1]) continue;
    const from = Math.max(axis.left, axis.toX(gap.firstHour));
    const to = Math.min(axis.right, axis.toX(gap.lastHour + 1));
    if (to > from) shown.push([from, to]);
  }
  return shown;
}

// Columns reach far past the canvas, so a scrolled or translated context keeps them whole.
const FAR = 1e7;

/** Narrows the clip to every column outside `gaps`: whatever is drawn after, a line, a held step
 * or a bar, stops at a span's first hour and resumes after its last. */
export function clipColumns(ctx: CanvasRenderingContext2D, axis: HourAxis, gaps: readonly LostSpan[]): void {
  const cut = columns(axis, gaps);
  if (!cut.length) return;
  ctx.beginPath();
  let left = -FAR;
  for (const [from, to] of cut) {
    if (from > left) ctx.rect(left, -FAR, from - left, 2 * FAR);
    left = Math.max(left, to);
  }
  ctx.rect(left, -FAR, FAR - left, 2 * FAR);
  ctx.clip();
}

/** Hatches each span in view from `top`, `height` high, as hours with no value: never a zero. */
export function hatchColumns(ctx: CanvasRenderingContext2D, tokens: ChartTokens, axis: HourAxis, gaps: readonly LostSpan[], top: number, height: number): void {
  for (const [from, to] of columns(axis, gaps)) drawUnknownHatch(ctx, tokens, { left: from, top, width: to - from, height });
}

/** [`clipColumns`] over a plot frame's canvas. */
export function clipGaps(ctx: CanvasRenderingContext2D, frame: PlotFrame, gaps: readonly LostSpan[]): void {
  clipColumns(ctx, frameAxis(frame), gaps);
}

/** [`hatchColumns`] across a plot frame's plot. */
export function drawGaps(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, gaps: readonly LostSpan[]): void {
  hatchColumns(ctx, tokens, frameAxis(frame), gaps, frame.plotTop, frame.plotHeight);
}

/** What a chart's tooltip says at an hour the recording lost. */
export function lostTooltip(gap: LostSpan, label: (hour: number) => string) {
  const span = gap.firstHour === gap.lastHour ? label(gap.firstHour) : `${label(gap.firstHour)} – ${label(gap.lastHour)}`;
  return { title: span, rows: [], note: 'Lost from the recording: no values are known here.' };
}
