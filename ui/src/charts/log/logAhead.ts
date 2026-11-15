import type { LogItem } from './logData';

/** When each item of a log held ahead shows: from `reveal` (a row's revealed hour; a month's first row's) to before
 * `gone` (the hour a row's successor replaces it; a month's once all its rows have gone). An unloaded window's
 * placeholder always shows. `reveals` and `gones` are the same hours ascending, the finite gones only, so the cut at an
 * hour is two binary searches. */
export interface LogSpans {
  readonly reveal: Float64Array;
  readonly gone: Float64Array;
  readonly reveals: Float64Array;
  readonly gones: Float64Array;
}

const revealOf = (item: LogItem<unknown>) => item.kind === 'row' ? Math.max(item.row.hour, item.row.revealedHour ?? item.row.hour) : -Infinity;
const goneOf = (item: LogItem<unknown>) => item.kind === 'row' ? item.row.leftHour ?? Infinity : Infinity;

export function logSpans(items: readonly LogItem<unknown>[]): LogSpans {
  const reveal = new Float64Array(items.length), gone = new Float64Array(items.length);
  // A month's divider heads the rows after it, up to the next divider or window. A section's heading always shows, and
  // its note until the first of the section's rows or windows does.
  let first = Infinity, last = -Infinity, rows = 0, section = Infinity;
  for (let at = items.length - 1; at >= 0; at--) {
    const item = items[at];
    if (item.kind === 'month' && item.part) {
      reveal[at] = -Infinity;
      gone[at] = item.part === 'note' ? section : Infinity;
      first = Infinity; last = -Infinity; rows = 0;
      if (item.part === 'heading') section = Infinity;
      continue;
    }
    if (item.kind === 'month') {
      reveal[at] = rows ? first : -Infinity;
      gone[at] = rows ? last : Infinity;
      first = Infinity; last = -Infinity; rows = 0;
      continue;
    }
    reveal[at] = revealOf(item);
    gone[at] = goneOf(item);
    section = Math.min(section, reveal[at]);
    if (item.kind === 'page') { first = Infinity; last = -Infinity; rows = 0; continue; }
    first = Math.min(first, reveal[at]); last = Math.max(last, gone[at]); rows++;
  }
  const reveals = Float64Array.from(reveal).sort();
  const gones = Float64Array.from(gone.filter(Number.isFinite)).sort();
  return { reveal, gone, reveals, gones };
}

/** How many of `sorted` are at or before `hour`. */
function through(sorted: Float64Array, hour: number): number {
  let low = 0, high = sorted.length;
  while (low < high) {
    const middle = (low + high) >> 1;
    if (sorted[middle] <= hour) low = middle + 1; else high = middle;
  }
  return low;
}

/** Which items show at `hour`, as one number: the same number, the same items. */
export function logCut(spans: LogSpans, hour: number): number {
  return through(spans.reveals, hour) * (spans.gones.length + 1) + through(spans.gones, hour);
}

/** Whether every item shows at `hour`. */
export function logAllShown(spans: LogSpans, hour: number): boolean {
  return through(spans.reveals, hour) === spans.reveals.length && through(spans.gones, hour) === 0;
}

/** Whether the item at `slot` shows at `hour`; `'ahead'` before its reveal, `'gone'` once its successor replaced it. */
export function logItemState(spans: LogSpans, slot: number, hour: number): 'shown' | 'ahead' | 'gone' {
  return spans.reveal[slot] > hour ? 'ahead' : spans.gone[slot] <= hour ? 'gone' : 'shown';
}

/** Which of one row's states (the items at `slots`) it shows at `hour`, and how: the state shown; before its first, its
 * first, held `'ahead'`; else the last it showed, `'gone'` (after its last, or until it comes back). */
export function logStateAt(spans: LogSpans, slots: readonly number[], hour: number): { slot: number; state: 'shown' | 'ahead' | 'gone' } {
  let first = slots[0], last = -1;
  for (const slot of slots) {
    if (spans.reveal[slot] > hour) { if (spans.reveal[slot] < spans.reveal[first] || spans.reveal[first] <= hour) first = slot; continue; }
    if (spans.gone[slot] > hour) return { slot, state: 'shown' };
    if (last < 0 || spans.gone[slot] > spans.gone[last]) last = slot;
  }
  return last < 0 ? { slot: first, state: 'ahead' } : { slot: last, state: 'gone' };
}

/** The keyboard's row ordinals at `hour` (`logRowOrdinals` counting only the items shown): keys step over the rows the
 * clock has revealed, never onto a row or state held ahead or gone. */
export function logShownOrdinals(items: readonly LogItem<unknown>[], spans: LogSpans, hour: number): number[] {
  const ordinals = [0];
  for (let at = 0; at < items.length; at++) {
    const item = items[at];
    ordinals.push(ordinals[at] + (item.kind === 'month' ? 0 : item.kind === 'page' ? item.page.count : logItemState(spans, at, hour) === 'shown' ? 1 : 0));
  }
  return ordinals;
}

/** The items' offsets at `hour`: an item not shown takes no height, so those after it close up over it. `offsets` itself
 * when every item shows. */
export function logShownOffsets(offsets: readonly number[], spans: LogSpans, hour: number): readonly number[] {
  if (logAllShown(spans, hour)) return offsets;
  const shown = new Array<number>(offsets.length);
  shown[0] = 0;
  for (let at = 0; at < offsets.length - 1; at++) {
    const height = spans.reveal[at] > hour || spans.gone[at] <= hour ? 0 : offsets[at + 1] - offsets[at];
    shown[at + 1] = shown[at] + height;
  }
  return shown;
}

/** The items to draw `[start, end)` while rows are held ahead (`offsets` laid at `hour`, `full` at full height), so that
 * no frame before the next render reaches an item not drawn: a row shown now and never leaving keeps its height through
 * the block, while a row held ahead or leaving may take none, so only the first fill the view. `anchored` (or at the
 * top), the item at `top` stays there and the view runs on from it; otherwise the view stays at its pixels, and the
 * rows above may push others down into it. At the top, every item before the first shown is drawn: rows held ahead
 * there (a battle beginning, an event arriving, newest first) are revealed into the view at the top, however many there
 * are, with no month divider shown above them. `overscan` items each way. */
export function logHeldRange(offsets: readonly number[], full: readonly number[], spans: LogSpans, hour: number, top: number, height: number,
  overscan: number, anchored: boolean): [number, number] {
  const count = offsets.length - 1;
  if (count <= 0) return [0, 0];
  const size = (at: number) => full[at + 1] - full[at];
  const steady = (at: number) => spans.reveal[at] <= hour && spans.gone[at] === Infinity;
  let start = 0, end = 0;
  if (anchored || top <= 0) {
    let high = count - 1;
    while (start < high) {
      const middle = (start + high + 1) >> 1;
      if (offsets[middle] <= top) start = middle; else high = middle - 1;
    }
    end = start;
    for (let filled = offsets[start] - top; end < count && filled < height; end++) if (steady(end)) filled += size(end);
    if (top <= 0) return [0, Math.min(count, end + overscan)];
  } else {
    for (let latest = 0; start < count; start++) {
      const next = latest + (spans.gone[start] > hour ? size(start) : 0);
      if (next > top) break;
      latest = next;
    }
    for (let earliest = 0; end < count && earliest < top + height; end++) if (steady(end)) earliest += size(end);
  }
  return [Math.max(0, start - overscan), Math.min(count, end + overscan)];
}

/** The items to draw `[start, end)` widened over the items next to them that take no height: rows held ahead sit where
 * the row they will push down starts, and are drawn with it so revealing them needs no render. */
export function widenOverHidden(offsets: readonly number[], start: number, end: number): [number, number] {
  const count = offsets.length - 1;
  while (start > 0 && offsets[start - 1] === offsets[start]) start--;
  while (end < count && offsets[end] === offsets[end + 1]) end++;
  return [start, end];
}
