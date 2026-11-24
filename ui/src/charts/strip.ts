import { font } from './draw.ts';
import type { PlotFrame, StripBox } from './frame.ts';
import type { KeyTarget } from './hover.ts';
import type { Canvas2D } from './textWidth.ts';
import type { Range } from './view.ts';
import type { ChartTokens } from './tokens.ts';


export interface ChartStrip {
  height(chartHeight: number): number;
  draw(ctx: Canvas2D, frame: PlotFrame, tokens: ChartTokens, state: { playhead: number | null }): void;
  describe?(point: { x: number; y: number }, frame: PlotFrame, until: number | null): number | null;
  keyTargets?(view: Range, until: number | null): readonly KeyTarget[];
}

export interface StripBar { firstHour: number; lastHour: number; up: number; down: number; span?: true }
export interface StripColumn { x: number; width: number; up: number; down: number; upBar: number; downBar: number }

const MIN_BAR = 2;
const GAP_FROM = 4;

export function stripColumns(bars: readonly StripBar[], toX: (hour: number) => number, from: number, to: number): StripColumn[] {
  const columns: (StripColumn & { shared: boolean })[] = [];
  bars.forEach((bar, index) => {
    const first = Math.max(bar.firstHour, from);
    const last = Math.min(bar.lastHour, to);
    if (last <= first) return;
    const left = toX(first);
    const width = toX(last) - left;
    if (width >= MIN_BAR) {
      columns.push({ x: left, width: width >= GAP_FROM ? width - 1 : width, up: bar.up, down: bar.down, upBar: index, downBar: index, shared: false });
      return;
    }
    const x = Math.floor(left);
    const previous = columns.at(-1);
    if (previous?.shared && previous.x === x) {
      if (bar.up >= previous.up) { previous.up = bar.up; previous.upBar = index; }
      if (bar.down >= previous.down) { previous.down = bar.down; previous.downBar = index; }
    } else columns.push({ x, width: 1, up: bar.up, down: bar.down, upBar: index, downBar: index, shared: true });
  });
  return columns.map(({ shared: _shared, ...column }) => column);
}

export const stripMiddle = (strip: StripBox) => Math.round(strip.top + strip.height / 2) + 0.5;

const drawnBars = (bars: readonly StripBar[], until: number | null | undefined) => until == null ? bars : bars.filter(bar => bar.lastHour <= until);

export interface StripRect { x: number; y: number; width: number; height: number; half: 'up' | 'down'; bar: number }

export function stripRects(columns: readonly StripColumn[], strip: StripBox): StripRect[] {
  const largest = columns.reduce((most, column) => Math.max(most, column.up, column.down), 0);
  if (!(largest > 0)) return [];
  const middle = stripMiddle(strip);
  const reach = Math.max(1, strip.height / 2 - 2);
  return columns.flatMap(column => {
    const up = column.up / largest * reach;
    const down = column.down / largest * reach;
    return [
      ...(up > 0 ? [{ x: column.x, y: middle - 0.5 - up, width: column.width, height: up, half: 'up' as const, bar: column.upBar }] : []),
      ...(down > 0 ? [{ x: column.x, y: middle + 0.5, width: column.width, height: down, half: 'down' as const, bar: column.downBar }] : []),
    ];
  });
}

export function stripBarAt(bars: readonly StripBar[], frame: PlotFrame, until: number | null, point: { x: number; y: number }): StripBar | null {
  const strip = frame.strip;
  if (!strip || point.y < frame.plotTop + frame.plotHeight || point.y > strip.top + strip.height) return null;
  const shown = drawnBars(bars, until);
  const columns = stripColumns(shown, frame.toX, frame.x[0], frame.x[1]);
  const inX = (item: { x: number; width: number }) => point.x >= item.x && point.x < item.x + item.width;
  const rects = stripRects(columns, strip);
  for (let index = rects.length - 1; index >= 0; index--) {
    const rect = rects[index];
    if (inX(rect) && point.y >= rect.y && point.y < rect.y + rect.height) return shown[rect.bar];
  }
  for (let index = columns.length - 1; index >= 0; index--) {
    if (inX(columns[index])) return shown[point.y < stripMiddle(strip) ? columns[index].upBar : columns[index].downBar];
  }
  return null;
}

export function stripKeyTargets(bars: readonly StripBar[], view: Range, until: number | null): KeyTarget[] {
  return drawnBars(bars, until).flatMap(bar => {
    if (bar.span) return [];
    const first = Math.max(bar.firstHour, view[0]);
    const last = Math.min(bar.lastHour, view[1]);
    return last > first ? [{ at: Math.max(first, Math.floor(last)), describes: bar.lastHour }] : [];
  });
}

export function drawStrip(ctx: Canvas2D, frame: PlotFrame, tokens: ChartTokens, bars: readonly StripBar[],
  options: { up: string; down: string; upLabel: string; downLabel: string; until?: number | null }): void {
  const strip = frame.strip;
  if (!strip) return;
  const columns = stripColumns(drawnBars(bars, options.until), frame.toX, frame.x[0], frame.x[1]);
  const middle = stripMiddle(strip);
  ctx.save();
  ctx.beginPath();
  ctx.rect(frame.plotLeft, strip.top, frame.plotWidth + 2, strip.height);
  ctx.clip();
  for (const rect of stripRects(columns, strip)) {
    ctx.fillStyle = rect.half === 'up' ? options.up : options.down;
    ctx.fillRect(rect.x, rect.y, rect.width, rect.height);
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

export function barStrip(bars: readonly StripBar[], look: { height: (chartHeight: number) => number; upLabel: string; downLabel: string }): ChartStrip {
  return {
    height: look.height,
    draw: (ctx, frame, tokens, state) => drawStrip(ctx, frame, tokens, bars,
      { up: tokens.categories[0], down: tokens.categories[1], upLabel: look.upLabel, downLabel: look.downLabel, until: state.playhead }),
    describe: (point, frame, until) => {
      const bar = stripBarAt(bars, frame, until, point);
      return bar && !bar.span ? bar.lastHour : null;
    },
    keyTargets: (view, until) => stripKeyTargets(bars, view, until),
  };
}
