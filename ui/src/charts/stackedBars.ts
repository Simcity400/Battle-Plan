import type { ChartLayer } from './TimeChart';
import type { PlotFrame } from './frame';
import { fillHatched } from './hatch.ts';
import type { ChartTokens } from './tokens';
import type { Range } from './view';

/** One period's bar on the time axis (such as a calendar month), stacked by part.
 * Parts keep their given order from the axis outwards: positive amounts stack up
 * from zero and negative ones down, so a signed correction is never hidden.
 * An `unknown` period is hatched across the plot rather than drawn as zero, and a
 * known period with no parts is an empty bar. */
export interface StackedPeriod {
  key: string;
  firstHour: number;
  /** The period's last hour, inclusive. */
  lastHour: number;
  parts: readonly { key: string; value: number }[];
  unknown?: boolean;
}

/** The period holding an hour, by binary search over periods in time order. */
export function periodAt<T extends { firstHour: number; lastHour: number }>(periods: readonly T[], hour: number): T | null {
  let low = 0, high = periods.length - 1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    const period = periods[middle];
    if (hour < period.firstHour) high = middle - 1;
    else if (hour > period.lastHour) low = middle + 1;
    else return period;
  }
  return null;
}

/** The stacked extremes of the known periods overlapping a time range, from zero. */
export function stackedExtent(periods: readonly StackedPeriod[], x: Range, until: number | null = null): Range | null {
  let low = 0, high = 0, any = false;
  for (const period of periods) {
    if (period.unknown || period.lastHour < x[0] || period.firstHour > x[1] || (until !== null && period.firstHour > until)) continue;
    let up = 0, down = 0;
    for (const part of period.parts) if (part.value >= 0) up += part.value; else down += part.value;
    any = true; high = Math.max(high, up); low = Math.min(low, down);
  }
  return any ? [low, high] : null;
}

/** The keyboard stops of the periods: each period's first hour. */
export const periodStops = (periods: readonly StackedPeriod[]) => periods.map(period => period.firstHour);

/** A gap either side of each bar, in layout pixels, so neighbouring periods read apart. */
const barGap = (width: number) => Math.min(2, width * 0.15);

/** Draws each period up to the playhead, its parts stacked in their colours. */
export function stackedBarLayer(periods: readonly StackedPeriod[], colour: (part: string, tokens: ChartTokens) => string): ChartLayer {
  return {
    draw(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, state: { playhead: number | null }) {
      const zero = frame.toY(Math.max(frame.y[0], Math.min(frame.y[1], 0)));
      for (const period of periods) {
        if (period.lastHour < frame.x[0] || period.firstHour > frame.x[1]) continue;
        if (state.playhead !== null && period.firstHour > state.playhead) continue;
        const left = frame.toX(Math.max(period.firstHour, frame.x[0]));
        const right = frame.toX(Math.min(period.lastHour + 1, frame.x[1]));
        const gap = barGap(right - left);
        const x = left + gap, width = right - left - 2 * gap;
        if (width <= 0) continue;
        if (period.unknown) {
          fillHatched(ctx, x, frame.plotTop, width, zero - frame.plotTop, tokens.muted, tokens.hatchSpacing, tokens.lineWidth.grid);
          continue;
        }
        let up = 0, down = 0;
        for (const part of period.parts) {
          if (part.value === 0) continue;
          const from = part.value > 0 ? up : down;
          const to = from + part.value;
          if (part.value > 0) up = to; else down = to;
          const top = frame.toY(Math.max(from, to)), bottom = frame.toY(Math.min(from, to));
          ctx.fillStyle = colour(part.key, tokens);
          ctx.fillRect(x, top, width, Math.max(1, bottom - top));
        }
      }
    },
    extent: x => stackedExtent(periods, x),
  };
}
