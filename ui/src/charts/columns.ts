/** Stacked columns on a time chart: one column per period (such as a calendar
 * month), spanning its hours on the shared time axis, its bands stacked in
 * order. Values are signed: positive amounts stack up from zero and negative
 * ones down from it, so no recorded correction is hidden. An amount that is
 * not known never stacks as zero: the column's known bands draw, and a hatched
 * cap above them marks the part that cannot be told. A band whose amount is
 * known but whose attribution is not draws hatched over its colour. */
import type { ChartLayer } from './TimeChart';
import type { PlotFrame } from './frame';
import type { ChartTokens } from './tokens';
import type { Range } from './view';
import { fillHatched } from './hatch.ts';
import { stackEdges } from './stack.ts';
import type { Edge } from './stack';

export interface ColumnPeriod {
  firstHour: number; lastHour: number;
  /** Each band's amount, in band order; null where it is not known. */
  values: readonly (number | null)[];
  /** Bands whose amount is known but not wholly attributed to them. */
  uncertain?: readonly boolean[];
}

/** Columns narrower than this many pixels draw with no gap between them. */
const GAP_FROM = 6;
/** Height of the hatched cap marking a column's unknown amounts, in hatch tiles. */
const CAP_TILES = 3;
/** Value-axis room kept above the stacks when a column has an unknown or
 * uncertain band, as a share of the whole signed span drawn, so its cap stays visible
 * however the axis fits, negative stacks included. */
export const CAP_HEADROOM = 0.1;

/** The period holding `hour`, by binary search over periods in hour order. */
export function columnAt(periods: readonly ColumnPeriod[], hour: number): number {
  let low = 0, high = periods.length - 1;
  while (low <= high) {
    const mid = (low + high) >> 1;
    if (hour < periods[mid].firstHour) high = mid - 1;
    else if (hour > periods[mid].lastHour) low = mid + 1;
    else return mid;
  }
  return -1;
}

/** Each known band's edges, in band order (null for an unknown band). */
export function columnEdges(period: ColumnPeriod): (Edge | null)[] {
  const known = period.values.map(value => value ?? 0);
  const edges = stackEdges(known);
  return edges.map((edge, band) => period.values[band] == null ? null : edge);
}

/** The values the columns overlapping `x` reach, with zero, for fitting the
 * value axis, and room above for the unknown cap when a column has one; null
 * when none overlaps. */
export function columnsExtent(periods: readonly ColumnPeriod[], x: Range): Range | null {
  let low = 0, high = 0, any = false, unknown = false;
  for (const period of periods) {
    if (period.lastHour < x[0] || period.firstHour > x[1]) continue;
    any = true;
    unknown ||= period.values.some(value => value == null) || period.uncertain?.some(Boolean) === true;
    for (const edge of columnEdges(period)) {
      if (!edge) continue;
      low = Math.min(low, edge.low); high = Math.max(high, edge.high);
    }
  }
  if (!any) return null;
  if (!unknown) return [low, high];
  // A column of nothing but unknown amounts still needs room for its cap.
  const span = high - low;
  return [low, span > 0 ? high + span * CAP_HEADROOM : high + 1];
}

/** A column's left edge and width in pixels: its hours, with a pixel kept
 * clear on each side once columns are wide enough to tell apart. */
export function columnBox(period: ColumnPeriod, frame: PlotFrame): { left: number; width: number } {
  const left = frame.toX(period.firstHour), right = frame.toX(period.lastHour + 1);
  const inset = right - left >= GAP_FROM ? 1 : 0;
  return { left: left + inset, width: Math.max(1, right - left - 2 * inset) };
}

/** A time-chart layer drawing `periods` with one colour per band. Columns
 * after the playhead are not drawn. */
export function columnsLayer(periods: readonly ColumnPeriod[], colours: readonly string[]): ChartLayer {
  return {
    draw(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, state: { playhead: number | null }) {
      const until = Math.min(frame.x[1], state.playhead ?? Infinity);
      const zero = frame.toY(0);
      const cap = tokens.hatchSpacing * CAP_TILES;
      for (const period of periods) {
        if (period.lastHour < frame.x[0] || period.firstHour > until) continue;
        const { left, width } = columnBox(period, frame);
        let top = zero;
        // A band known but uncertain that draws no height still shows.
        let marked = false;
        columnEdges(period).forEach((edge, band) => {
          if (!edge) return;
          const y0 = frame.toY(edge.high), y1 = frame.toY(edge.low);
          const height = y1 - y0;
          if (height < 1) {
            marked ||= period.uncertain?.[band] === true;
            return;
          }
          ctx.fillStyle = colours[band % colours.length];
          ctx.fillRect(left, y0, width, height);
          if (period.uncertain?.[band]) fillHatched(ctx, left, y0, width, height, tokens.muted, tokens.hatchSpacing, tokens.lineWidth.grid);
          top = Math.min(top, y0);
        });
        if (marked) {
          const markTop = Math.max(frame.plotTop, top - tokens.hatchSpacing);
          fillHatched(ctx, left, markTop, width, top - markTop, tokens.muted, tokens.hatchSpacing, tokens.lineWidth.grid);
          top = markTop;
        }
        if (period.values.some(value => value == null)) {
          const capTop = Math.max(frame.plotTop, top - cap);
          fillHatched(ctx, left, capTop, width, top - capTop, tokens.muted, tokens.hatchSpacing, tokens.lineWidth.grid);
        }
      }
    },
    extent: x => columnsExtent(periods, x),
  };
}
