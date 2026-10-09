import { mix } from '../../shell/appearance/color.ts';

export interface HeatPart { id: string; label: string; color: string; share: number }
export interface HeatCell { parts: readonly HeatPart[]; unknown?: boolean; filtered?: boolean }
export interface HeatPeriod { firstHour: number; lastHour: number; readHour: number | null }
export interface HeatRow { id: string; label: string; cells: readonly HeatCell[] }
export type HeatLook = 'margin' | 'split';

/** Logical scroll rows, with two rows of drawing margin on either side. */
export function heatWindow(count: number, scrollTop: number, height: number, minimumRow: number, top: number, bottom: number) {
  const rowHeight = Math.max(minimumRow, (height - top - bottom) / Math.max(1, count));
  const total = Math.max(height, top + count * rowHeight + bottom);
  const scroll = Math.max(0, Math.min(total - height, scrollTop));
  const first = Math.max(0, Math.floor(scroll / rowHeight) - 2);
  const end = Math.min(count, Math.ceil((scroll + Math.max(0, height - top - bottom)) / rowHeight) + 2);
  return { first, end, rowHeight, total, offset: scroll - first * rowHeight,
    plotHeight: height };
}

export function heatRowAt(y: number, window: ReturnType<typeof heatWindow>, top: number, count: number) {
  return Math.max(0, Math.min(count - 1, window.first + Math.floor((y - top + window.offset) / window.rowHeight)));
}

/** Join only identical adjacent cells. Their tooltips still describe separate
 * reads; this avoids redrawing thousands of equal stripes across a long history. */
export function cellRuns(row: HeatRow, periods: readonly HeatPeriod[], until: number | null): { period: HeatPeriod; cell: HeatCell }[] {
  const result: { period: HeatPeriod; cell: HeatCell }[] = [];
  periods.forEach((period, column) => {
    const cell = row.cells[column];
    if (!cell || period.readHour == null || period.readHour > (until ?? Infinity)) return;
    const last = result.at(-1);
    if (last && last.cell.parts === cell.parts && !!last.cell.unknown === !!cell.unknown && !!last.cell.filtered === !!cell.filtered
      && last.period.lastHour + 1 === period.firstHour) {
      last.period = { ...last.period, lastHour: period.lastHour, readHour: period.readHour };
    } else result.push({ period, cell });
  });
  return result;
}

/** Same signed-off ramp as style-6-heat: ties are grey, a 50-point lead
 * reaches the side's full colour. Single-side cells have a zero runner-up. */
export function marginColor(parts: readonly HeatPart[], grey: string): string | null {
  let first: HeatPart | undefined, second = 0;
  for (const part of parts) {
    if (part.share <= 0) continue;
    if (!first || part.share > first.share || part.share === first.share && part.id.localeCompare(first.id) < 0) {
      second = first?.share ?? 0; first = part;
    } else second = Math.max(second, part.share);
  }
  if (!first) return null;
  return mix(grey, first.color, Math.max(0, Math.min(1, (first.share - second) / 0.5)));
}

/** Split cells have horizontal stripes, sized by share, in stable side order. */
export function splitStripes(parts: readonly HeatPart[], top: number, height: number): { color: string; top: number; height: number }[] {
  const sum = parts.reduce((total, part) => total + Math.max(0, part.share), 0);
  if (!sum) return [];
  let used = 0;
  return parts.filter(part => part.share > 0).map(part => {
    const stripe = { color: part.color, top: top + used / sum * height, height: part.share / sum * height };
    used += part.share; return stripe;
  });
}

export function periodAt(periods: readonly HeatPeriod[], hour: number): number | null {
  let lo = 0, hi = periods.length;
  while (lo < hi) { const mid = (lo + hi) >>> 1; if (periods[mid].firstHour <= hour) lo = mid + 1; else hi = mid; }
  const index = lo - 1;
  return index >= 0 && hour <= periods[index].lastHour ? index : null;
}

/** The props wired into TimeChart: both keys and pointer snapping use reads. */
export function heatNavigation(periods: readonly HeatPeriod[]) {
  return {
    keyHours: periods.flatMap(period => period.readHour == null ? [] : [period.readHour]),
    snap(hour: number): number {
      const index = periodAt(periods, hour);
      return index == null ? hour : periods[index].readHour ?? hour;
    },
  };
}

/** Exact rectangles: never round across a missing period or use a future read. */
export function cellRect(period: HeatPeriod, row: number, count: number, frame: { plotTop: number; plotHeight: number; plotLeft: number; plotWidth: number; toX: (hour: number) => number },
  from: number, to: number, until: number | null) {
  if (period.readHour != null && period.readHour > (until ?? Infinity) || period.lastHour < from || period.firstHour > to || !count) return null;
  const left = Math.max(frame.plotLeft, frame.toX(Math.max(from, period.firstHour)));
  const right = Math.min(frame.plotLeft + frame.plotWidth, frame.toX(Math.min(to, period.lastHour + 1, until == null ? Infinity : until + 1)));
  const rowHeight = frame.plotHeight / count;
  return right > left ? { left, top: frame.plotTop + row * rowHeight + 1, width: right - left, height: Math.max(1, rowHeight - 2) } : null;
}
