import { ellipsise } from './labels.ts';
import { formatDate, gameDate, monthNames } from '../dashboard/calendar.ts';
import type { GameDate } from '../dashboard/calendar.ts';
import { chartBottom } from './frame.ts';
import type { PlotFrame } from './frame.ts';
import { layoutEndLabels } from './labels.ts';
import { curveSegments, gapRegions, heldSegments } from './series.ts';
import type { PathCommand, Run } from './series.ts';
import { timeTicks } from './timeAxis.ts';
import type { ChartTokens } from './tokens.ts';
import { linearTicks, logTicks, thinLogTicks } from './valueAxis.ts';
import { type Canvas2D, type MeasureText, canvasMeasureText, textWidthRevision } from './textWidth.ts';

export const font = (tokens: ChartTokens, size: number, weight = 400) => `${weight} ${size}px ${tokens.font}`;
export const LABEL_WEIGHT = 500;
export const VALUE_WEIGHT = 600;
const crisp = (value: number) => Math.round(value) + 0.5;

export function tracePath(ctx: Canvas2D, commands: readonly PathCommand[]): void {
  for (const command of commands) {
    if (command[0] === 'M') ctx.moveTo(command[1], command[2]);
    else if (command[0] === 'L') ctx.lineTo(command[1], command[2]);
    else ctx.bezierCurveTo(command[1], command[2], command[3], command[4], command[5], command[6]);
  }
}

export function valueTicks(frame: PlotFrame, tokens: ChartTokens): number[] {
  if (frame.valueTicks) return frame.valueTicks.filter(value => value >= frame.y[0] && value <= frame.y[1]);
  return frame.log ? thinLogTicks(logTicks(frame.y[0], frame.y[1]), frame.toY, tokens.fontSize.small + 6)
    : linearTicks(frame.y[0], frame.y[1], Math.max(2, Math.round(frame.plotHeight / 48)));
}

export const VALUE_LABEL_GAP = 6;

export function measureValueLabelWidth(frame: PlotFrame, tokens: ChartTokens, formatValue: (value: number, ticks: readonly number[]) => string,
  measure: MeasureText): number {
  const labelFont = font(tokens, tokens.fontSize.small);
  const values = valueTicks(frame, tokens);
  return Math.max(0, ...values.map(value => measure(formatValue(value, values), labelFont)));
}

export function valueLabelWidth(ctx: Canvas2D | null, frame: PlotFrame, tokens: ChartTokens, formatValue: (value: number, ticks: readonly number[]) => string,
  measure?: MeasureText): number {
  if (measure) return measureValueLabelWidth(frame, tokens, formatValue, measure);
  const context = ctx!;
  context.save();
  const labelFont = font(tokens, tokens.fontSize.small);
  context.font = labelFont;
  const width = measureValueLabelWidth(frame, tokens, formatValue, canvasMeasureText(context));
  context.restore();
  return width;
}

const rowLabels = new WeakMap<Canvas2D, { font: string; width: number; measure: MeasureText; revision: number; labels: Map<string, string> }>();
export function drawAxes(ctx: Canvas2D, frame: PlotFrame, tokens: ChartTokens, formatValue: (value: number, ticks: readonly number[]) => string,
  rows?: readonly string[], measure?: MeasureText): void {
  const { plotLeft, plotTop, plotWidth, plotHeight } = frame;
  const measureText = measure ?? canvasMeasureText(ctx);
  ctx.save();
  ctx.lineWidth = tokens.lineWidth.grid;
  const smallFont = font(tokens, tokens.fontSize.small);
  ctx.font = smallFont;
  ctx.textAlign = 'right';
  ctx.textBaseline = 'middle';
  const values = rows ? [] : valueTicks(frame, tokens);
  for (const value of values) {
    const y = crisp(frame.toY(value));
    ctx.strokeStyle = tokens.grid;
    ctx.beginPath(); ctx.moveTo(plotLeft, y); ctx.lineTo(plotLeft + plotWidth, y); ctx.stroke();
    ctx.fillStyle = tokens.muted;
    ctx.fillText(formatValue(value, values), plotLeft - VALUE_LABEL_GAP, y);
  }
  if (rows) {
    const rowFont = font(tokens, tokens.fontSize.small, 500);
    ctx.font = rowFont;
    const width = plotLeft - VALUE_LABEL_GAP * 2;
    const revision = textWidthRevision();
    let cached = rowLabels.get(ctx);
    if (!cached || cached.font !== rowFont || cached.width !== width || cached.measure !== measureText || cached.revision !== revision) {
      cached = { font: rowFont, width, measure: measureText, revision, labels: new Map() };
      rowLabels.set(ctx, cached);
    }
    rows.forEach((label, index) => {
      let fitted = cached.labels.get(label);
      if (fitted === undefined) { fitted = ellipsise(label, width, text => measureText(text, rowFont)); cached.labels.set(label, fitted); }
      ctx.fillStyle = tokens.text;
      ctx.fillText(fitted, plotLeft - VALUE_LABEL_GAP, plotTop + (index + 0.5) / Math.max(1, rows.length) * plotHeight);
    });
  }
  const tickFont = rows ? font(tokens, tokens.fontSize.small, 500) : smallFont;
  const ticks = timeTicks(frame.x[0], frame.x[1], plotWidth, text => measureText(text, tickFont));
  const dates = frame.timeAxisTop ? plotTop - tokens.fontSize.small * 2 : chartBottom(frame);
  ctx.strokeStyle = tokens.grid;
  for (const hour of ticks.grid) {
    const x = crisp(frame.toX(hour));
    ctx.beginPath(); ctx.moveTo(x, frame.timeAxisTop ? plotTop : dates); ctx.lineTo(x, frame.timeAxisTop ? plotTop + plotHeight : dates + 4); ctx.stroke();
  }
  ctx.textAlign = 'center';
  ctx.textBaseline = 'top';
  ctx.fillStyle = tokens.muted;
  for (const label of ticks.labels) ctx.fillText(label.text, frame.toX(label.hour), dates + 6);
  ctx.strokeStyle = tokens.axis;
  ctx.beginPath(); ctx.moveTo(plotLeft, crisp(plotTop + plotHeight)); ctx.lineTo(plotLeft + plotWidth, crisp(plotTop + plotHeight)); ctx.stroke();
  ctx.restore();
}

export function clipToPlot(ctx: Canvas2D, frame: PlotFrame): void {
  ctx.beginPath();
  ctx.rect(frame.plotLeft, frame.plotTop - 2, frame.plotWidth + 2, frame.plotHeight + 4);
  ctx.clip();
}

export function drawLine(ctx: Canvas2D, frame: PlotFrame, tokens: ChartTokens, runs: readonly Run[],
  color: string, options: { area?: boolean; endDot?: boolean; until?: number; dash?: readonly number[]; held?: boolean } = {}): { x: number; y: number } | null {
  const until = Math.min(frame.x[1], options.until ?? Infinity);
  let last: { x: number; y: number } | null = null;
  const baseline = frame.toY(frame.log ? frame.y[0] : Math.max(frame.y[0], Math.min(0, frame.y[1])));
  const lineSegments = options.held ? heldSegments : curveSegments;
  for (const { points, path } of lineSegments(runs, frame.x[0], until, frame.toX, frame.toY)) {
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
    ctx.beginPath(); ctx.arc(last.x, last.y, tokens.lineWidth.series * 2, 0, Math.PI * 2);
    ctx.fillStyle = color; ctx.fill();
    ctx.lineWidth = tokens.lineWidth.series; ctx.strokeStyle = tokens.surface; ctx.stroke();
  }
  return last;
}

export function drawGapFill(ctx: Canvas2D, frame: PlotFrame, first: readonly Run[], second: readonly Run[],
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

export function drawBands(ctx: Canvas2D, frame: PlotFrame, tokens: ChartTokens, hours: readonly number[], bands: readonly { low: readonly number[]; high: readonly number[] }[], colors: readonly string[],
  ends: { start?: boolean; end?: boolean } = {}): void {
  bands.forEach((band, layer) => {
    if (band.high.every((value, index) => value === band.low[index])) return;
    if (hours.length === 1) {
      const top = frame.toY(band.high[0]);
      ctx.fillStyle = colors[layer];
      ctx.fillRect(frame.toX(hours[0]) - 1, top, 2, frame.toY(band.low[0]) - top);
      return;
    }
    ctx.beginPath();
    traceBandEdge(ctx, frame, hours, band.high);
    for (let index = hours.length - 1; index >= 0; index--) ctx.lineTo(frame.toX(hours[index]), frame.toY(band.low[index]));
    ctx.closePath();
    ctx.fillStyle = colors[layer]; ctx.fill();
    if (ends.start === false || ends.end === false) {
      ctx.beginPath();
      traceBandEdge(ctx, frame, hours, band.high);
      if (ends.end === false) ctx.moveTo(frame.toX(hours.at(-1)!), frame.toY(band.low.at(-1)!));
      else ctx.lineTo(frame.toX(hours.at(-1)!), frame.toY(band.low.at(-1)!));
      for (let index = hours.length - 2; index >= 0; index--) ctx.lineTo(frame.toX(hours[index]), frame.toY(band.low[index]));
      if (ends.start !== false) ctx.lineTo(frame.toX(hours[0]), frame.toY(band.high[0]));
    }
    ctx.lineWidth = 1; ctx.strokeStyle = tokens.surface; ctx.stroke();
  });
}

function traceBandEdge(ctx: Canvas2D, frame: PlotFrame, hours: readonly number[], values: readonly number[]): void {
  if (hours.length === 1) {
    const x = frame.toX(hours[0]);
    const y = frame.toY(values[0]);
    ctx.moveTo(x - 1, y); ctx.lineTo(x + 1, y);
    return;
  }
  hours.forEach((hour, index) => index ? ctx.lineTo(frame.toX(hour), frame.toY(values[index])) : ctx.moveTo(frame.toX(hour), frame.toY(values[index])));
}

export function drawPlayhead(ctx: Canvas2D, frame: PlotFrame, tokens: ChartTokens, hour: number): void {
  if (hour < frame.x[0] || hour > frame.x[1]) return;
  const x = crisp(frame.toX(hour));
  ctx.save();
  ctx.strokeStyle = tokens.accent; ctx.lineWidth = 1.5;
  ctx.beginPath(); ctx.moveTo(x, frame.plotTop); ctx.lineTo(x, chartBottom(frame)); ctx.stroke();
  ctx.restore();
}

export function drawCrosshair(ctx: Canvas2D, frame: PlotFrame, tokens: ChartTokens, hour: number): void {
  const x = crisp(frame.toX(hour));
  ctx.save();
  ctx.strokeStyle = tokens.muted; ctx.globalAlpha = 0.6; ctx.lineWidth = 1;
  ctx.beginPath(); ctx.moveTo(x, frame.plotTop); ctx.lineTo(x, chartBottom(frame)); ctx.stroke();
  ctx.restore();
}

export function drawSelection(ctx: Canvas2D, tokens: ChartTokens, box: { x0: number; y0: number; x1: number; y1: number }): void {
  const x = Math.min(box.x0, box.x1);
  const y = Math.min(box.y0, box.y1);
  ctx.save();
  ctx.fillStyle = tokens.accent; ctx.globalAlpha = 0.12;
  ctx.fillRect(x, y, Math.abs(box.x1 - box.x0), Math.abs(box.y1 - box.y0));
  ctx.globalAlpha = 0.8; ctx.strokeStyle = tokens.accent; ctx.lineWidth = 1;
  ctx.strokeRect(crisp(x), crisp(y), Math.abs(box.x1 - box.x0), Math.abs(box.y1 - box.y0));
  ctx.restore();
}

export function drawEndLabels(ctx: Canvas2D, frame: PlotFrame, tokens: ChartTokens, labels: readonly { text: string; suffix?: string; y: number; from: number; color: string; muted?: boolean }[],
  measure?: MeasureText): void {
  const measureText = measure ?? canvasMeasureText(ctx);
  const labelFont = font(tokens, tokens.fontSize.small, 600);
  ctx.save();
  ctx.font = labelFont;
  ctx.textAlign = 'left'; ctx.textBaseline = 'middle';
  const x = frame.plotLeft + frame.plotWidth + 10;
  for (const label of labels) {
    if (Math.abs(label.y - label.from) > 2) {
      ctx.strokeStyle = label.color; ctx.globalAlpha = 0.5; ctx.lineWidth = 1;
      ctx.beginPath(); ctx.moveTo(x - 8, label.from); ctx.lineTo(x - 2, label.y); ctx.stroke(); ctx.globalAlpha = 1;
    }
    ctx.fillStyle = label.muted ? tokens.muted : tokens.text;
    const room = frame.padding.right - 14, suffix = label.suffix ? ` ${label.suffix}` : '';
    const text = room > 0 ? ellipsise(label.text, room - measureText(suffix, labelFont), text => measureText(text, labelFont)) : label.text;
    ctx.fillText(`${text}${suffix}`.trimStart(), x, label.y);
  }
  ctx.restore();
}

export interface LineEnd { id: string; y: number; text: string; suffix?: string; color: string; muted?: boolean }
export const END_LABEL_HEIGHT = 14;

export function placeEndLabels(ctx: Canvas2D, frame: PlotFrame, tokens: ChartTokens, ends: readonly LineEnd[], gap = 2, measure?: MeasureText): void {
  const placed = layoutEndLabels(ends.map(end => ({ ...end, height: END_LABEL_HEIGHT })), frame.plotTop, frame.plotTop + frame.plotHeight, gap);
  drawEndLabels(ctx, frame, tokens, ends.map(end => ({ text: end.text, suffix: end.suffix, y: placed.get(end.id)!, from: end.y, color: end.color, muted: end.muted })), measure);
}

export function dayLabel(hour: number): string {
  const date = gameDate(Math.floor(hour));
  return date ? formatDate(date) : `Hour ${Math.floor(hour)}`;
}

export function spanLabel(first: number, last: number): string {
  const [from, to] = [gameDate(Math.floor(first)), gameDate(Math.floor(last))];
  if (!from || !to) return dayLabel(first) === dayLabel(last) ? dayLabel(first) : `${dayLabel(first)} – ${dayLabel(last)}`;
  if (from.year !== to.year) return `${formatDate(from)} – ${formatDate(to)}`;
  const month = (date: GameDate) => monthNames[date.month - 1].slice(0, 3);
  if (from.month !== to.month) return `${from.day} ${month(from)} – ${to.day} ${month(to)} ${to.year}`;
  return from.day === to.day ? formatDate(from) : `${from.day}–${to.day} ${month(to)} ${to.year}`;
}
