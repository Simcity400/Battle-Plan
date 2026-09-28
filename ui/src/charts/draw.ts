import { formatDate, gameDate } from '../dashboard/calendar.ts';
import { chartBottom } from './frame.ts';
import type { PlotFrame } from './frame.ts';
import { layoutEndLabels } from './labels.ts';
import { curveSegments, gapRegions } from './series.ts';
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

/** The value ticks a frame's axis is drawn with. They depend only on the value range
 * and the plot's height, so a chart can measure their labels before it settles its
 * left padding. */
export function valueTicks(frame: PlotFrame, tokens: ChartTokens): number[] {
  return frame.log ? thinLogTicks(logTicks(frame.y[0], frame.y[1]), frame.toY, tokens.fontSize.small + 6)
    : linearTicks(frame.y[0], frame.y[1], Math.max(2, Math.round(frame.plotHeight / 48)));
}

/** The gap between a value label and the plot, in pixels. */
export const VALUE_LABEL_GAP = 6;

/** The widest value label of a frame's axis, in pixels. */
export function valueLabelWidth(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, formatValue: (value: number, ticks: readonly number[]) => string): number {
  ctx.save();
  ctx.font = font(tokens, tokens.fontSize.small);
  const values = valueTicks(frame, tokens);
  const width = Math.max(0, ...values.map(value => ctx.measureText(formatValue(value, values)).width));
  ctx.restore();
  return width;
}

/** Hairline value gridlines with their labels, and date labels under the plot (and
 * any strip under it).
 * `formatValue` also gets every tick drawn, so it can keep neighbouring labels apart. */
export function drawAxes(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, formatValue: (value: number, ticks: readonly number[]) => string): void {
  const { plotLeft, plotTop, plotWidth, plotHeight } = frame;
  ctx.save();
  ctx.lineWidth = tokens.lineWidth.grid;
  ctx.font = font(tokens, tokens.fontSize.small);
  ctx.textAlign = 'right';
  ctx.textBaseline = 'middle';
  const values = valueTicks(frame, tokens);
  for (const value of values) {
    const y = crisp(frame.toY(value));
    ctx.strokeStyle = tokens.grid;
    ctx.beginPath(); ctx.moveTo(plotLeft, y); ctx.lineTo(plotLeft + plotWidth, y); ctx.stroke();
    ctx.fillStyle = tokens.muted;
    ctx.fillText(formatValue(value, values), plotLeft - VALUE_LABEL_GAP, y);
  }
  const ticks = timeTicks(frame.x[0], frame.x[1], plotWidth, text => ctx.measureText(text).width);
  const dates = chartBottom(frame);
  ctx.strokeStyle = tokens.grid;
  for (const hour of ticks.grid) {
    const x = crisp(frame.toX(hour));
    ctx.beginPath(); ctx.moveTo(x, dates); ctx.lineTo(x, dates + 4); ctx.stroke();
  }
  ctx.textAlign = 'center';
  ctx.textBaseline = 'top';
  ctx.fillStyle = tokens.muted;
  for (const label of ticks.labels) ctx.fillText(label.text, frame.toX(label.hour), dates + 6);
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
  for (const { points, path } of curveSegments(runs, frame.x[0], until, frame.toX, frame.toY)) {
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

/** A light fill between two lines drawn with `drawLine` (to the same last hour),
 * bounded by exactly the drawn curves: `aboveColor` where the first line is higher,
 * `belowColor` where the second is, split wherever they cross. */
export function drawGapFill(ctx: CanvasRenderingContext2D, frame: PlotFrame, first: readonly Run[], second: readonly Run[],
  aboveColor: string, belowColor: string, options: { until?: number; alpha?: number } = {}): void {
  const until = Math.min(frame.x[1], options.until ?? Infinity);
  const regions = gapRegions(curveSegments(first, frame.x[0], until, frame.toX, frame.toY), curveSegments(second, frame.x[0], until, frame.toX, frame.toY),
    frame.plotTop, frame.plotTop + frame.plotHeight);
  const paint = (list: readonly { clip: PathCommand[]; fill: PathCommand[] }[], color: string) => {
    for (const region of list) {
      ctx.save();
      ctx.beginPath(); tracePath(ctx, region.clip); ctx.closePath(); ctx.clip();
      ctx.beginPath(); tracePath(ctx, region.fill); ctx.closePath();
      ctx.globalAlpha = options.alpha ?? 0.16; ctx.fillStyle = color; ctx.fill();
      ctx.restore();
    }
  };
  paint(regions.above, aboveColor);
  paint(regions.below, belowColor);
}

/** Stacked areas: `tops[k][i]` is the top of layer k at `hours[i]`, drawn with a
 * surface-coloured seam between layers. */
/** Filled bands between explicit lower and upper edges (a diverging stack), each
 * with a thin surface outline so neighbouring bands stay apart. */
export function drawBands(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, hours: readonly number[], bands: readonly { low: readonly number[]; high: readonly number[] }[], colors: readonly string[]): void {
  bands.forEach((band, layer) => {
    if (band.high.every((value, index) => value === band.low[index])) return;
    // A single sample (an hour between gaps) has no width: it is drawn as a thin bar.
    if (hours.length === 1) {
      const top = frame.toY(band.high[0]);
      ctx.fillStyle = colors[layer];
      ctx.fillRect(frame.toX(hours[0]) - 1, top, 2, frame.toY(band.low[0]) - top);
      return;
    }
    ctx.beginPath();
    hours.forEach((hour, index) => index ? ctx.lineTo(frame.toX(hour), frame.toY(band.high[index])) : ctx.moveTo(frame.toX(hour), frame.toY(band.high[index])));
    for (let index = hours.length - 1; index >= 0; index--) ctx.lineTo(frame.toX(hours[index]), frame.toY(band.low[index]));
    ctx.closePath();
    ctx.fillStyle = colors[layer]; ctx.fill();
    ctx.lineWidth = 1; ctx.strokeStyle = tokens.surface; ctx.stroke();
  });
}

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

/** The accent playhead, through the plot and any strip under it. */
export function drawPlayhead(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, hour: number): void {
  if (hour < frame.x[0] || hour > frame.x[1]) return;
  const x = crisp(frame.toX(hour));
  ctx.save();
  ctx.strokeStyle = tokens.accent; ctx.lineWidth = 1.5;
  ctx.beginPath(); ctx.moveTo(x, frame.plotTop); ctx.lineTo(x, chartBottom(frame)); ctx.stroke();
  ctx.restore();
}

/** The hover crosshair, through the plot and any strip under it. */
export function drawCrosshair(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, hour: number): void {
  const x = crisp(frame.toX(hour));
  ctx.save();
  ctx.strokeStyle = tokens.muted; ctx.globalAlpha = 0.6; ctx.lineWidth = 1;
  ctx.beginPath(); ctx.moveTo(x, frame.plotTop); ctx.lineTo(x, chartBottom(frame)); ctx.stroke();
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

/** Text labels at the right of the plot, already placed apart by `layoutEndLabels`;
 * a `muted` label names a secondary line, such as a ceiling. */
export function drawEndLabels(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, labels: readonly { text: string; y: number; from: number; color: string; muted?: boolean }[]): void {
  ctx.save();
  ctx.font = font(tokens, tokens.fontSize.small, 600);
  ctx.textAlign = 'left'; ctx.textBaseline = 'middle';
  const x = frame.plotLeft + frame.plotWidth + 10;
  for (const label of labels) {
    if (Math.abs(label.y - label.from) > 2) {
      ctx.strokeStyle = label.color; ctx.globalAlpha = 0.5; ctx.lineWidth = 1;
      ctx.beginPath(); ctx.moveTo(x - 8, label.from); ctx.lineTo(x - 2, label.y); ctx.stroke(); ctx.globalAlpha = 1;
    }
    ctx.fillStyle = label.muted ? tokens.muted : tokens.text;
    // A max width of zero or less draws nothing, so only limit it when there is room.
    const room = frame.padding.right - 14;
    if (room > 0) ctx.fillText(label.text, x, label.y, room); else ctx.fillText(label.text, x, label.y);
  }
  ctx.restore();
}

/** One line's end label: where its line ends (`y`, in pixels), its text and colour. */
export interface LineEnd { id: string; y: number; text: string; color: string; muted?: boolean }
/** An end label's height, and the least gap kept between two. */
export const END_LABEL_HEIGHT = 14;

/** End labels for the lines' ends, placed apart inside the plot (`layoutEndLabels`)
 * and drawn with a leader line where a label had to move from its line's end. */
export function placeEndLabels(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, ends: readonly LineEnd[], gap = 2): void {
  const placed = layoutEndLabels(ends.map(end => ({ ...end, height: END_LABEL_HEIGHT })), frame.plotTop, frame.plotTop + frame.plotHeight, gap);
  drawEndLabels(ctx, frame, tokens, ends.map(end => ({ text: end.text, y: placed.get(end.id)!, from: end.y, color: end.color, muted: end.muted })));
}

/** A day label for tooltips and menus, from the recording's own calendar. */
export function dayLabel(hour: number): string {
  const date = gameDate(Math.floor(hour));
  return date ? formatDate(date) : `Hour ${Math.floor(hour)}`;
}
