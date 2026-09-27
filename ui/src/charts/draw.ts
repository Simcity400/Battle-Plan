import { formatDate, gameDate } from '../dashboard/calendar.ts';
import type { PlotFrame } from './frame.ts';
import { monotonePath, samplePoints, segments } from './series.ts';
import type { PathCommand, Run } from './series.ts';
import { timeTicks } from './timeAxis.ts';
import type { ChartTokens } from './tokens.ts';
import { linearTicks, logTicks, thinLogTicks } from './valueAxis.ts';

/** Canvas drawing shared by every chart. Everything takes its colours and sizes
 * from the tokens and works in layout pixels. */

export const font = (tokens: ChartTokens, size: number, weight = 400) => `${weight} ${size}px ${tokens.font}`;
const crisp = (value: number) => Math.round(value) + 0.5;

export function tracePath(ctx: CanvasRenderingContext2D, commands: readonly PathCommand[]): void {
  for (const command of commands) {
    if (command[0] === 'M') ctx.moveTo(command[1], command[2]);
    else if (command[0] === 'L') ctx.lineTo(command[1], command[2]);
    else ctx.bezierCurveTo(command[1], command[2], command[3], command[4], command[5], command[6]);
  }
}

/** Hairline value gridlines with their labels, and date labels under the plot. */
export function drawAxes(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, formatValue: (value: number) => string): void {
  const { plotLeft, plotTop, plotWidth, plotHeight } = frame;
  ctx.save();
  ctx.lineWidth = tokens.lineWidth.grid;
  ctx.font = font(tokens, tokens.fontSize.small);
  ctx.textAlign = 'right';
  ctx.textBaseline = 'middle';
  const values = frame.log ? thinLogTicks(logTicks(frame.y[0], frame.y[1]), frame.toY, tokens.fontSize.small + 6)
    : linearTicks(frame.y[0], frame.y[1], Math.max(2, Math.round(plotHeight / 48)));
  for (const value of values) {
    const y = crisp(frame.toY(value));
    ctx.strokeStyle = tokens.grid;
    ctx.beginPath(); ctx.moveTo(plotLeft, y); ctx.lineTo(plotLeft + plotWidth, y); ctx.stroke();
    ctx.fillStyle = tokens.muted;
    ctx.fillText(formatValue(value), plotLeft - 6, y);
  }
  const ticks = timeTicks(frame.x[0], frame.x[1], plotWidth, text => ctx.measureText(text).width);
  ctx.strokeStyle = tokens.grid;
  for (const hour of ticks.grid) {
    const x = crisp(frame.toX(hour));
    ctx.beginPath(); ctx.moveTo(x, plotTop + plotHeight); ctx.lineTo(x, plotTop + plotHeight + 4); ctx.stroke();
  }
  ctx.textAlign = 'center';
  ctx.textBaseline = 'top';
  ctx.fillStyle = tokens.muted;
  for (const label of ticks.labels) ctx.fillText(label.text, frame.toX(label.hour), plotTop + plotHeight + 6);
  ctx.strokeStyle = tokens.axis;
  ctx.beginPath(); ctx.moveTo(plotLeft, crisp(plotTop + plotHeight)); ctx.lineTo(plotLeft + plotWidth, crisp(plotTop + plotHeight)); ctx.stroke();
  ctx.restore();
}

/** Clips drawing to the plot so zoomed series never spill over the axes. */
export function clipToPlot(ctx: CanvasRenderingContext2D, frame: PlotFrame): void {
  ctx.beginPath();
  ctx.rect(frame.plotLeft, frame.plotTop - 2, frame.plotWidth + 2, frame.plotHeight + 4);
  ctx.clip();
}

/** A 2px line through recorded runs, broken at gaps, with an optional light area
 * fill, an optional dash pattern (for a second measure in the same colour) and a
 * ringed dot at its last recorded point. Returns that point. */
export function drawLine(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, runs: readonly Run[],
  color: string, options: { area?: boolean; endDot?: boolean; until?: number; dash?: readonly number[] } = {}): { x: number; y: number } | null {
  const until = Math.min(frame.x[1], options.until ?? Infinity);
  let last: { x: number; y: number } | null = null;
  const baseline = frame.toY(frame.log ? frame.y[0] : Math.max(frame.y[0], Math.min(0, frame.y[1])));
  for (const segment of segments(runs, frame.x[0], until)) {
    const points = samplePoints(segment, frame.toX, frame.toY);
    if (!points.length) continue;
    const path = monotonePath(points);
    if (options.area && points.length > 1) {
      ctx.beginPath(); tracePath(ctx, path);
      ctx.lineTo(points.at(-1)!.x, baseline); ctx.lineTo(points[0].x, baseline); ctx.closePath();
      ctx.globalAlpha = 0.12; ctx.fillStyle = color; ctx.fill(); ctx.globalAlpha = 1;
    }
    ctx.beginPath(); tracePath(ctx, path);
    if (points.length === 1) ctx.arc(points[0].x, points[0].y, tokens.lineWidth.series / 2, 0, Math.PI * 2);
    ctx.strokeStyle = color; ctx.lineWidth = tokens.lineWidth.series; ctx.lineJoin = 'round'; ctx.lineCap = options.dash ? 'butt' : 'round';
    ctx.setLineDash(options.dash ?? []); ctx.stroke(); ctx.setLineDash([]);
    last = points.at(-1)!;
  }
  if (last && options.endDot !== false) {
    ctx.beginPath(); ctx.arc(last.x, last.y, 4, 0, Math.PI * 2);
    ctx.fillStyle = color; ctx.fill();
    ctx.lineWidth = 2; ctx.strokeStyle = tokens.surface; ctx.stroke();
  }
  return last;
}

/** Stacked areas: `tops[k][i]` is the top of layer k at `hours[i]`, drawn with a
 * surface-coloured seam between layers. */
export function drawStack(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, hours: readonly number[], tops: readonly (readonly number[])[], colors: readonly string[]): void {
  let base = hours.map(() => (frame.log ? frame.y[0] : 0));
  tops.forEach((top, layer) => {
    ctx.beginPath();
    hours.forEach((hour, index) => index ? ctx.lineTo(frame.toX(hour), frame.toY(top[index])) : ctx.moveTo(frame.toX(hour), frame.toY(top[index])));
    for (let index = hours.length - 1; index >= 0; index--) ctx.lineTo(frame.toX(hours[index]), frame.toY(base[index]));
    ctx.closePath();
    ctx.fillStyle = colors[layer]; ctx.fill();
    ctx.lineWidth = 1; ctx.strokeStyle = tokens.surface; ctx.stroke();
    base = [...top];
  });
}

/** Grey shading over hours the recording is missing. */
export function drawGaps(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, gaps: readonly { firstHour: number; lastHour: number }[]): void {
  ctx.save();
  ctx.fillStyle = tokens.grid;
  ctx.globalAlpha = 0.35;
  for (const gap of gaps) {
    if (gap.lastHour < frame.x[0] || gap.firstHour > frame.x[1]) continue;
    const from = frame.toX(Math.max(gap.firstHour, frame.x[0]));
    const to = frame.toX(Math.min(gap.lastHour + 1, frame.x[1]));
    ctx.fillRect(from, frame.plotTop, Math.max(1, to - from), frame.plotHeight);
  }
  ctx.restore();
}

/** Short event ticks along the bottom of the plot. */
export function drawEventTicks(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, hours: readonly number[], color = tokens.muted): void {
  ctx.save();
  ctx.strokeStyle = color; ctx.lineWidth = 1.5;
  const bottom = frame.plotTop + frame.plotHeight;
  for (const hour of hours) {
    if (hour < frame.x[0] || hour > frame.x[1]) continue;
    const x = crisp(frame.toX(hour));
    ctx.beginPath(); ctx.moveTo(x, bottom); ctx.lineTo(x, bottom - 6); ctx.stroke();
  }
  ctx.restore();
}

/** The accent playhead. */
export function drawPlayhead(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, hour: number): void {
  if (hour < frame.x[0] || hour > frame.x[1]) return;
  const x = crisp(frame.toX(hour));
  ctx.save();
  ctx.strokeStyle = tokens.accent; ctx.lineWidth = 1.5;
  ctx.beginPath(); ctx.moveTo(x, frame.plotTop); ctx.lineTo(x, frame.plotTop + frame.plotHeight); ctx.stroke();
  ctx.restore();
}

/** The hover crosshair. */
export function drawCrosshair(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, hour: number): void {
  const x = crisp(frame.toX(hour));
  ctx.save();
  ctx.strokeStyle = tokens.muted; ctx.globalAlpha = 0.6; ctx.lineWidth = 1;
  ctx.beginPath(); ctx.moveTo(x, frame.plotTop); ctx.lineTo(x, frame.plotTop + frame.plotHeight); ctx.stroke();
  ctx.restore();
}

/** The box-zoom selection. */
export function drawSelection(ctx: CanvasRenderingContext2D, tokens: ChartTokens, box: { x0: number; y0: number; x1: number; y1: number }): void {
  const x = Math.min(box.x0, box.x1);
  const y = Math.min(box.y0, box.y1);
  ctx.save();
  ctx.fillStyle = tokens.accent; ctx.globalAlpha = 0.12;
  ctx.fillRect(x, y, Math.abs(box.x1 - box.x0), Math.abs(box.y1 - box.y0));
  ctx.globalAlpha = 0.8; ctx.strokeStyle = tokens.accent; ctx.lineWidth = 1;
  ctx.strokeRect(crisp(x), crisp(y), Math.abs(box.x1 - box.x0), Math.abs(box.y1 - box.y0));
  ctx.restore();
}

/** Text labels at the right of the plot, already placed apart by `layoutEndLabels`. */
export function drawEndLabels(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, labels: readonly { text: string; y: number; from: number; color: string }[]): void {
  ctx.save();
  ctx.font = font(tokens, tokens.fontSize.small, 600);
  ctx.textAlign = 'left'; ctx.textBaseline = 'middle';
  const x = frame.plotLeft + frame.plotWidth + 10;
  for (const label of labels) {
    if (Math.abs(label.y - label.from) > 2) {
      ctx.strokeStyle = label.color; ctx.globalAlpha = 0.5; ctx.lineWidth = 1;
      ctx.beginPath(); ctx.moveTo(x - 8, label.from); ctx.lineTo(x - 2, label.y); ctx.stroke(); ctx.globalAlpha = 1;
    }
    ctx.fillStyle = tokens.text;
    // A max width of zero or less draws nothing, so only limit it when there is room.
    const room = frame.padding.right - 14;
    if (room > 0) ctx.fillText(label.text, x, label.y, room); else ctx.fillText(label.text, x, label.y);
  }
  ctx.restore();
}

/** A day label for tooltips and menus, from the recording's own calendar. */
export function dayLabel(hour: number): string {
  const date = gameDate(Math.floor(hour));
  return date ? formatDate(date) : `Hour ${Math.floor(hour)}`;
}
