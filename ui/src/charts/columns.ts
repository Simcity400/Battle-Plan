/** Stacked columns on a time chart: one column per period (such as a calendar
 * month), spanning its hours on the shared time axis, its bands stacked in
 * order. Values are signed: positive amounts stack up from zero and negative
 * ones down from it, so no recorded correction is hidden. An amount that is
 * not known never stacks as zero: the column's known bands draw, and a hatched
 * cap above them marks the part that cannot be told. A band whose amount is
 * known but whose attribution is not draws hatched over its colour. A period
 * that is not known at all is hatched across the plot rather than drawn. */
import type { ChartLayer } from './TimeChart';
import type { PlotFrame } from './frame';
import type { ChartTokens } from './tokens';
import type { Range } from './view';
import { fillHatched } from './hatch.ts';
import { stackEdges } from './stack.ts';
import type { Edge } from './stack';

export interface ColumnPeriod {
  firstHour: number;
  /** The period's last hour, inclusive. */
  lastHour: number;
  /** Each band's amount, in band order; null where it is not known. */
  values: readonly (number | null)[];
  /** Bands whose amount is known but not wholly attributed to them. */
  uncertain?: readonly boolean[];
  /** Nothing about the period is known: hatched across the plot, never zero, and
   * left out of the value axis. */
  unknown?: boolean;
}

/** How a chart's columns are drawn. */
export interface ColumnStyle {
  /** `pixel` (the default) keeps a pixel clear on each side once columns are wide
   * enough to tell apart; `proportional` keeps up to two pixels, a share of the
   * column's width in view, so narrow columns still read apart. */
  spacing?: 'pixel' | 'proportional';
  /** Draws a known nonzero band at least a pixel tall, so a small amount beside
   * large ones still shows; by default a band under a pixel draws nothing. */
  thinBands?: boolean;
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

/** The keyboard stops of the periods: each period's first hour. */
export const columnStops = (periods: readonly ColumnPeriod[]) => periods.map(period => period.firstHour);

/** Each known band's edges, in band order (null for an unknown band). */
export function columnEdges(period: ColumnPeriod): (Edge | null)[] {
  const known = period.values.map(value => value ?? 0);
  const edges = stackEdges(known);
  return edges.map((edge, band) => period.values[band] == null ? null : edge);
}

/** The values the known columns overlapping `x` reach, with zero, for fitting
 * the value axis, and room above for the unknown cap when a column has one;
 * null when no known column overlaps. */
export function columnsExtent(periods: readonly ColumnPeriod[], x: Range): Range | null {
  let low = 0, high = 0, any = false, unknown = false;
  for (const period of periods) {
    if (period.unknown || period.lastHour < x[0] || period.firstHour > x[1]) continue;
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

/** A column's left edge and width in pixels, or null when nothing of it shows:
 * its hours, less the gaps its `spacing` keeps on each side. */
export function columnBox(period: ColumnPeriod, frame: PlotFrame, spacing: ColumnStyle['spacing'] = 'pixel'): { left: number; width: number } | null {
  if (spacing === 'proportional') {
    const left = frame.toX(Math.max(period.firstHour, frame.x[0])), right = frame.toX(Math.min(period.lastHour + 1, frame.x[1]));
    const gap = Math.min(2, (right - left) * 0.15);
    const width = right - left - 2 * gap;
    return width > 0 ? { left: left + gap, width } : null;
  }
  const left = frame.toX(period.firstHour), right = frame.toX(period.lastHour + 1);
  const inset = right - left >= GAP_FROM ? 1 : 0;
  return { left: left + inset, width: Math.max(1, right - left - 2 * inset) };
}

/** A time-chart layer drawing `periods` with one colour per band. Columns
 * after the playhead are not drawn. */
export function columnsLayer(periods: readonly ColumnPeriod[], colours: readonly string[], style: ColumnStyle = {}): ChartLayer {
  return {
    draw(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, state: { playhead: number | null }) {
      const until = Math.min(frame.x[1], state.playhead ?? Infinity);
      const zero = frame.toY(0);
      const cap = tokens.hatchSpacing * CAP_TILES;
      const hatch = (left: number, top: number, width: number, height: number) =>
        fillHatched(ctx, left, top, width, height, tokens.muted, tokens.hatchSpacing, tokens.lineWidth.grid);
      for (const period of periods) {
        if (period.lastHour < frame.x[0] || period.firstHour > until) continue;
        const box = columnBox(period, frame, style.spacing);
        if (!box) continue;
        const { left, width } = box;
        if (period.unknown) {
          // Down to zero, or to the plot's edge when zero lies outside the value axis.
          hatch(left, frame.plotTop, width, frame.toY(Math.max(frame.y[0], Math.min(frame.y[1], 0))) - frame.plotTop);
          continue;
        }
        let top = zero;
        // A band known but uncertain that draws no height still shows.
        let marked = false;
        columnEdges(period).forEach((edge, band) => {
          if (!edge) return;
          const y0 = frame.toY(edge.high), y1 = frame.toY(edge.low);
          let height = y1 - y0;
          if (height < 1) {
            // A band is zero by its amount: one too small to move the stack's edges still draws.
            if (!style.thinBands || period.values[band] === 0) {
              marked ||= period.uncertain?.[band] === true;
              return;
            }
            height = 1;
          }
          ctx.fillStyle = colours[band % colours.length];
          ctx.fillRect(left, y0, width, height);
          if (period.uncertain?.[band]) hatch(left, y0, width, height);
          top = Math.min(top, y0);
        });
        if (marked) {
          const markTop = Math.max(frame.plotTop, top - tokens.hatchSpacing);
          hatch(left, markTop, width, top - markTop);
          top = markTop;
        }
        if (period.values.some(value => value == null)) {
          const capTop = Math.max(frame.plotTop, top - cap);
          hatch(left, capTop, width, top - capTop);
        }
      }
    },
    extent: x => columnsExtent(periods, x),
  };
}
