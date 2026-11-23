import type { PlotFrame } from './frame';
import { drawUnknownHatch } from './hatch.ts';
import type { Canvas2D } from './textWidth.ts';
import type { ChartTokens } from './tokens';
import type { Range } from './view';

export interface LostSpan { firstHour: number; lastHour: number }

export function lostAt(gaps: readonly LostSpan[], hour: number): LostSpan | null {
  for (const gap of gaps) {
    if (gap.firstHour > hour) return null;
    if (hour <= gap.lastHour) return gap;
  }
  return null;
}

export interface HourAxis { toX(hour: number): number; x: Range; left: number; right: number }

export const frameAxis = (frame: PlotFrame): HourAxis => ({ toX: hour => frame.toX(hour), x: frame.x, left: frame.plotLeft, right: frame.plotLeft + frame.plotWidth });

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

const FAR = 1e7;

export function clipColumns(ctx: Canvas2D, axis: HourAxis, gaps: readonly LostSpan[]): void {
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

export function hatchColumns(ctx: Canvas2D, tokens: ChartTokens, axis: HourAxis, gaps: readonly LostSpan[], top: number, height: number): void {
  for (const [from, to] of columns(axis, gaps)) drawUnknownHatch(ctx, tokens, { left: from, top, width: to - from, height });
}

export function clipGaps(ctx: Canvas2D, frame: PlotFrame, gaps: readonly LostSpan[]): void {
  clipColumns(ctx, frameAxis(frame), gaps);
}

export function drawGaps(ctx: Canvas2D, frame: PlotFrame, tokens: ChartTokens, gaps: readonly LostSpan[]): void {
  hatchColumns(ctx, tokens, frameAxis(frame), gaps, frame.plotTop, frame.plotHeight);
}

export function lostTooltip(gap: LostSpan, label: (hour: number) => string) {
  const span = gap.firstHour === gap.lastHour ? label(gap.firstHour) : `${label(gap.firstHour)} – ${label(gap.lastHour)}`;
  return { title: span, rows: [], note: 'Lost from the recording: no values are known here.' };
}
