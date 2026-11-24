import type { LogItem } from './logData';

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

function through(sorted: Float64Array, hour: number): number {
  let low = 0, high = sorted.length;
  while (low < high) {
    const middle = (low + high) >> 1;
    if (sorted[middle] <= hour) low = middle + 1; else high = middle;
  }
  return low;
}

export function logCut(spans: LogSpans, hour: number): number {
  return through(spans.reveals, hour) * (spans.gones.length + 1) + through(spans.gones, hour);
}

export function logAllShown(spans: LogSpans, hour: number): boolean {
  return through(spans.reveals, hour) === spans.reveals.length && through(spans.gones, hour) === 0;
}

export function logItemState(spans: LogSpans, slot: number, hour: number): 'shown' | 'ahead' | 'gone' {
  return spans.reveal[slot] > hour ? 'ahead' : spans.gone[slot] <= hour ? 'gone' : 'shown';
}

export function logStateAt(spans: LogSpans, slots: readonly number[], hour: number): { slot: number; state: 'shown' | 'ahead' | 'gone' } {
  let first = slots[0], last = -1;
  for (const slot of slots) {
    if (spans.reveal[slot] > hour) { if (spans.reveal[slot] < spans.reveal[first] || spans.reveal[first] <= hour) first = slot; continue; }
    if (spans.gone[slot] > hour) return { slot, state: 'shown' };
    if (last < 0 || spans.gone[slot] > spans.gone[last]) last = slot;
  }
  return last < 0 ? { slot: first, state: 'ahead' } : { slot: last, state: 'gone' };
}

export function logShownOrdinals(items: readonly LogItem<unknown>[], spans: LogSpans, hour: number): number[] {
  const ordinals = [0];
  for (let at = 0; at < items.length; at++) {
    const item = items[at];
    ordinals.push(ordinals[at] + (item.kind === 'month' ? 0 : item.kind === 'page' ? item.page.count : logItemState(spans, at, hour) === 'shown' ? 1 : 0));
  }
  return ordinals;
}

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

