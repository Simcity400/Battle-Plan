import { formatDate, gameDate } from '../dashboard/calendar.ts';
import type { PlotFrame } from './frame.ts';
import { timeTicks } from './timeAxis.ts';
import type { ChartTokens } from './tokens.ts';
import { linearTicks, logTicks, thinLogTicks } from './valueAxis.ts';
import { type Canvas2D, type MeasureText, canvasMeasureText } from './textWidth.ts';

export const font = (tokens: ChartTokens, size: number, weight = 400) => `${weight} ${size}px ${tokens.font}`;
export const LABEL_WEIGHT = 500;
export const VALUE_WEIGHT = 600;
const crisp = (value: number) => Math.round(value) + 0.5;

export function valueTicks(frame: PlotFrame, tokens: ChartTokens): number[] {
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

export function drawAxes(ctx: Canvas2D, frame: PlotFrame, tokens: ChartTokens, formatValue: (value: number, ticks: readonly number[]) => string,
  measure?: MeasureText): void {
  const { plotLeft, plotTop, plotWidth, plotHeight } = frame;
  const measureText = measure ?? canvasMeasureText(ctx);
  ctx.save();
  ctx.lineWidth = tokens.lineWidth.grid;
  const smallFont = font(tokens, tokens.fontSize.small);
  ctx.font = smallFont;
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
  const ticks = timeTicks(frame.x[0], frame.x[1], plotWidth, text => measureText(text, smallFont));
  const dates = plotTop + plotHeight;
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

export function clipToPlot(ctx: Canvas2D, frame: PlotFrame): void {
  ctx.beginPath();
  ctx.rect(frame.plotLeft, frame.plotTop - 2, frame.plotWidth + 2, frame.plotHeight + 4);
  ctx.clip();
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
  ctx.beginPath(); ctx.moveTo(x, frame.plotTop); ctx.lineTo(x, frame.plotTop + frame.plotHeight); ctx.stroke();
  ctx.restore();
}

export function drawCrosshair(ctx: Canvas2D, frame: PlotFrame, tokens: ChartTokens, hour: number): void {
  const x = crisp(frame.toX(hour));
  ctx.save();
  ctx.strokeStyle = tokens.muted; ctx.globalAlpha = 0.6; ctx.lineWidth = 1;
  ctx.beginPath(); ctx.moveTo(x, frame.plotTop); ctx.lineTo(x, frame.plotTop + frame.plotHeight); ctx.stroke();
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

export function dayLabel(hour: number): string {
  const date = gameDate(Math.floor(hour));
  return date ? formatDate(date) : `Hour ${Math.floor(hour)}`;
}
