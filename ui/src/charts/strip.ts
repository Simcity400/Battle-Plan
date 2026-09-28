import { font } from './draw.ts';
import type { PlotFrame, StripBox } from './frame.ts';
import type { KeyTarget } from './hover.ts';
import type { Range } from './view.ts';
import type { ChartTokens } from './tokens.ts';

/** A bar strip under a time chart, on the same time axis (mock-up economy-1's fuel
 * gain and burn): each bar spans its hours with one amount up from a midline and
 * one down, on a scale shared by both halves. Bars too narrow to see apart share a
 * pixel column that keeps the largest of each, so a short peak is never lost; a
 * hover is about the bar whose amount is drawn where the pointer is. */

/** A band under the plot on the same time axis, above the dates, such as daily bars. */
export interface ChartStrip {
  /** Its height in layout pixels, for a chart this many pixels tall. */
  height(chartHeight: number): number;
  /** Draws the strip (`frame.strip`), unclipped so it can label itself in the value
   * axis' gutter; `state.playhead` is the last hour to draw (null: the whole range). */
  draw(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, state: { playhead: number | null }): void;
  /** The hour a hover at `point` (layout pixels) is about, such as the end of the bar
   * drawn there, even when a zoom cuts that end off: the tooltip, Jump to and pick
   * describe it while the crosshair stays at the pointer (`pointerTarget`). Null: the
   * hour under the pointer. `until` is the last hour drawn. */
  describe?(point: { x: number; y: number }, frame: PlotFrame, until: number | null): number | null;
  /** The keyboard's stops for what the strip draws in `view` by `until` (the last
   * hour drawn), resolved as `describe` resolves the pointer, so the keys reach
   * everything a pointer can hover. */
  keyTargets?(view: Range, until: number | null): readonly KeyTarget[];
}

export interface StripBar { firstHour: number; lastHour: number; up: number; down: number }
/** One drawn column: its pixels [`x`, `x` + `width`) and, per half, the amount drawn
 * and the index of the bar it comes from. */
export interface StripColumn { x: number; width: number; up: number; down: number; upBar: number; downBar: number }

/** Below this many pixels a bar shares its column with its neighbours. */
const MIN_BAR = 2;
/** From this many pixels a bar keeps a pixel of space before the next. */
const GAP_FROM = 4;

/** The pixel columns of the bars (in hour order) that have hours inside [from, to]:
 * a bar that only touches the range (ending at its start, say) has none there and is
 * not drawn. A shared column draws each half's largest amount, from the bar it names
 * (the later on a tie). */
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

/** The strip's midline in layout pixels: gain above it, burn below. */
export const stripMiddle = (strip: StripBox) => Math.round(strip.top + strip.height / 2) + 0.5;

/** The bars drawn by `until` (null: all): one ending later is not drawn at all, its
 * amounts being known only from that end. */
const drawnBars = (bars: readonly StripBar[], until: number | null | undefined) => until == null ? bars : bars.filter(bar => bar.lastHour <= until);

/** One filled rectangle of the strip, in layout pixels, with the bar (an index into
 * the bars drawn) whose amount it shows. */
export interface StripRect { x: number; y: number; width: number; height: number; half: 'up' | 'down'; bar: number }

/** The rectangles `drawStrip` fills for these columns, in the order it fills them
 * (each column's gain, then its burn), scaled to the largest amount in view. */
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

/** The bar whose amount is drawn at `point`, so a hover describes exactly what is
 * drawn there: the topmost rectangle containing it (the last filled, as columns may
 * overlap by a pixel). Over no rectangle but inside a column (above a short bar), the
 * topmost such column's bar for the half pointed at, so a short bar stays easy to
 * hover. Null where nothing is drawn at that pixel (between bars, the space after a
 * wide bar) or outside the strip. */
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

/** A keyboard stop for each bar drawn in `view` by `until`: at its end, where its
 * amounts are known, or where the view cuts it off when that end lies past the
 * view, describing its end either way, as a pointer hover over it does. A bar
 * that only touches the view is not drawn and has none. */
export function stripKeyTargets(bars: readonly StripBar[], view: Range, until: number | null): KeyTarget[] {
  return drawnBars(bars, until).flatMap(bar => {
    const first = Math.max(bar.firstHour, view[0]);
    const last = Math.min(bar.lastHour, view[1]);
    return last > first ? [{ at: Math.max(first, Math.floor(last)), describes: bar.lastHour }] : [];
  });
}

/** The strip's bars that end by `until` (an hour; null: all), scaled to the largest
 * amount in view, with the midline and a label for each half in the value axis'
 * gutter. */
export function drawStrip(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, bars: readonly StripBar[],
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

/** A time chart's strip of up and down bars: drawn with `drawStrip` in the palette's
 * first two colours, hovered through the same columns (`stripBarAt`), so a hover is
 * about the bar drawn under the pointer and ends at its last hour, and reached by
 * the keyboard at each bar drawn in view (`stripKeyTargets`). */
export function barStrip(bars: readonly StripBar[], look: { height: (chartHeight: number) => number; upLabel: string; downLabel: string }): ChartStrip {
  return {
    height: look.height,
    draw: (ctx, frame, tokens, state) => drawStrip(ctx, frame, tokens, bars,
      { up: tokens.categories[0], down: tokens.categories[1], upLabel: look.upLabel, downLabel: look.downLabel, until: state.playhead }),
    describe: (point, frame, until) => stripBarAt(bars, frame, until, point)?.lastHour ?? null,
    keyTargets: (view, until) => stripKeyTargets(bars, view, until),
  };
}
