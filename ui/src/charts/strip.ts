import { font } from './draw.ts';
import type { PlotFrame } from './frame.ts';
import type { ChartTokens } from './tokens.ts';

/** A bar strip under a time chart, on the same time axis (mock-up economy-1's fuel
 * gain and burn): each bar spans its hours with one amount up from a midline and
 * one down, on a scale shared by both halves. Bars too narrow to see apart share a
 * pixel column that keeps the largest of each, so a short peak is never lost. */

export interface StripBar { firstHour: number; lastHour: number; up: number; down: number }
export interface StripColumn { x: number; width: number; up: number; down: number }

/** Below this many pixels a bar shares its column with its neighbours. */
const MIN_BAR = 2;
/** From this many pixels a bar keeps a pixel of space before the next. */
const GAP_FROM = 4;

/** The pixel columns of the bars (in hour order) inside [from, to]. */
export function stripColumns(bars: readonly StripBar[], toX: (hour: number) => number, from: number, to: number): StripColumn[] {
  const columns: (StripColumn & { shared: boolean })[] = [];
  for (const bar of bars) {
    if (bar.lastHour < from || bar.firstHour > to) continue;
    const left = toX(Math.max(bar.firstHour, from));
    const width = toX(Math.min(bar.lastHour, to)) - left;
    if (width >= MIN_BAR) {
      columns.push({ x: left, width: width >= GAP_FROM ? width - 1 : width, up: bar.up, down: bar.down, shared: false });
      continue;
    }
    const x = Math.floor(left);
    const previous = columns.at(-1);
    if (previous?.shared && previous.x === x) {
      previous.up = Math.max(previous.up, bar.up);
      previous.down = Math.max(previous.down, bar.down);
    } else columns.push({ x, width: 1, up: bar.up, down: bar.down, shared: true });
  }
  return columns.map(({ x, width, up, down }) => ({ x, width, up, down }));
}

/** The strip's bars that end by `until` (an hour; null: all), scaled to the largest
 * amount in view, with the midline and a label for each half in the value axis'
 * gutter. A bar ending after `until` is not drawn at all: its amounts are known
 * only from that end. */
export function drawStrip(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, bars: readonly StripBar[],
  options: { up: string; down: string; upLabel: string; downLabel: string; until?: number | null }): void {
  const strip = frame.strip;
  if (!strip) return;
  const until = options.until;
  const shown = until == null ? bars : bars.filter(bar => bar.lastHour <= until);
  const columns = stripColumns(shown, frame.toX, frame.x[0], frame.x[1]);
  const largest = columns.reduce((most, column) => Math.max(most, column.up, column.down), 0);
  const middle = Math.round(strip.top + strip.height / 2) + 0.5;
  const reach = Math.max(1, strip.height / 2 - 2);
  ctx.save();
  ctx.beginPath();
  ctx.rect(frame.plotLeft, strip.top, frame.plotWidth + 2, strip.height);
  ctx.clip();
  if (largest > 0) {
    for (const column of columns) {
      const up = column.up / largest * reach;
      const down = column.down / largest * reach;
      ctx.fillStyle = options.up;
      if (up > 0) ctx.fillRect(column.x, middle - 0.5 - up, column.width, up);
      ctx.fillStyle = options.down;
      if (down > 0) ctx.fillRect(column.x, middle + 0.5, column.width, down);
    }
  }
  ctx.restore();
  ctx.save();
  ctx.strokeStyle = tokens.axis; ctx.lineWidth = tokens.lineWidth.grid;
  ctx.beginPath(); ctx.moveTo(frame.plotLeft, middle); ctx.lineTo(frame.plotLeft + frame.plotWidth, middle); ctx.stroke();
  ctx.fillStyle = tokens.muted;
  ctx.font = font(tokens, tokens.fontSize.small);
  ctx.textAlign = 'right';
  ctx.textBaseline = 'top';
  ctx.fillText(options.upLabel, frame.plotLeft - 6, strip.top);
  ctx.textBaseline = 'bottom';
  ctx.fillText(options.downLabel, frame.plotLeft - 6, strip.top + strip.height);
  ctx.restore();
}
