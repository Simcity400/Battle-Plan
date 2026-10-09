import { tooltipRowHeight } from './tooltip.ts';
import type { TooltipRow } from './tooltip';

/** Estimate unseen rows and retain actual measurements for rows already drawn. */
export function tooltipRowOffsets(rows: readonly TooltipRow[], measured: ReadonlyMap<number, number> = new Map()): number[] {
  const offsets = [0];
  for (let index = 0; index < rows.length; index++) offsets.push(offsets[index] + (measured.get(index) ?? tooltipRowHeight(rows[index])));
  return offsets;
}

export const tooltipScrollTop = (offsets: readonly number[], top: number, height: number, leading = 0, trailing = 0) =>
  Math.max(0, Math.min(top, leading + (offsets[offsets.length - 1] ?? 0) + trailing - height));

/** A measured viewport chooses the rows in view. The complete list stays in the
 * tooltip store; any row is reachable, with no chosen count or private limit. */
export function tooltipRowWindow(offsets: readonly number[], top: number, height: number, leading = 0, trailing = 0): { first: number; last: number } {
  if (!(height > 0) || offsets.length < 2) return { first: 0, last: 0 };
  const scrollTop = tooltipScrollTop(offsets, top, height, leading, trailing) - leading;
  const end = offsets[offsets.length - 1];
  if (scrollTop + height <= 0) return { first: 0, last: 0 };
  if (scrollTop >= end) return { first: offsets.length - 1, last: offsets.length - 1 };
  const bound = (value: number, after: boolean) => {
    let low = 0;
    let high = offsets.length;
    while (low < high) {
      const middle = (low + high) >>> 1;
      if (offsets[middle] < value || after && offsets[middle] === value) low = middle + 1; else high = middle;
    }
    return low;
  };
  return { first: Math.max(0, bound(Math.max(0, scrollTop), true) - 1), last: Math.min(offsets.length - 1, bound(Math.min(end, scrollTop + height), false)) };
}
