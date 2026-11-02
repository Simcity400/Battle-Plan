import type { LogItem } from './logData';

export interface LogRevealHistory {
  readonly playhead?: number | null;
  readonly from: number | null;
  readonly loaded: ReadonlySet<string>;
  /** The rows of each stable server window already presented at this playhead. */
  readonly presented: ReadonlyMap<string, ReadonlySet<string>>;
  readonly reveals?: LogReveals;
}

export const EMPTY_LOG_REVEALS: LogRevealHistory = { from: null, loaded: new Set(), presented: new Map() };

interface RevealIndex {
  readonly items: ReadonlyArray<LogItem<unknown>>;
  readonly boundaries: ReadonlyArray<Readonly<{ hour: number; key: string; slot: number }>>;
  readonly loaded: ReadonlySet<string>;
  readonly shown: ReadonlyMap<string, ReadonlySet<string>>;
}

export interface LogReveals {
  readonly entering: ReadonlySet<string>;
  readonly playhead?: number | null;
  readonly index: RevealIndex;
  readonly next: number;
  readonly presented: ReadonlyMap<string, ReadonlySet<string>>;
}

const revealFrom = (previous: LogRevealHistory, playhead?: number | null) =>
  typeof playhead === 'number' && typeof previous.playhead === 'number'
    ? playhead > previous.playhead ? previous.playhead : playhead === previous.playhead ? previous.from : null
    : null;

function revealIndex(items: readonly LogItem<unknown>[]): RevealIndex {
  const loaded = new Set<string>();
  const shown = new Map<string, Set<string>>();
  const boundaries: { hour: number; key: string; slot: number }[] = [];
  for (let slot = 0; slot < items.length; slot++) {
    const item = items[slot];
    if (item.kind !== 'row') continue;
    if (item.pageKey != null) {
      loaded.add(item.pageKey);
      shown.set(item.pageKey, (shown.get(item.pageKey) ?? new Set<string>()).add(item.key));
    }
    boundaries.push({ hour: Math.max(item.row.hour, item.row.revealedHour ?? item.row.hour), key: item.key, slot });
  }
  boundaries.sort((a, b) => a.hour - b.hour || a.slot - b.slot);
  return { items, boundaries, loaded, shown };
}

function nextReveal(index: RevealIndex, hour: number): number {
  let first = 0, last = index.boundaries.length;
  while (first < last) {
    const middle = (first + last) >>> 1;
    if (index.boundaries[middle].hour <= hour) first = middle + 1;
    else last = middle;
  }
  return first;
}

export function logReveals(previous: LogRevealHistory, items: readonly LogItem<unknown>[], playhead?: number | null): LogReveals {
  const held = previous.reveals;
  const same = held !== undefined && held.index.items === items;
  if (same && playhead === previous.playhead) return held;
  if (same && typeof playhead === 'number' && typeof previous.playhead === 'number' && playhead > previous.playhead) {
    if (held.next === held.index.boundaries.length || held.index.boundaries[held.next].hour > playhead) return held;
    const next = nextReveal(held.index, playhead);
    const entering = new Set<string>();
    for (let at = held.next; at < next; at++) entering.add(held.index.boundaries[at].key);
    return { entering, playhead, index: held.index, next, presented: held.index.shown };
  }
  const index = same ? held.index : revealIndex(items);
  const from = revealFrom(previous, playhead);
  const presented = typeof playhead === 'number' && playhead === previous.playhead
    ? new Map(previous.presented) : new Map<string, ReadonlySet<string>>();
  const entering = new Set<string>();
  for (const item of items) {
    if (item.kind !== 'row') continue;
    const { pageKey, row } = item;
    if (playhead === undefined) {
      if (pageKey == null || previous.loaded.has(pageKey)) entering.add(item.key);
    } else if (typeof playhead === 'number' && from != null) {
      const revealed = Math.max(row.hour, row.revealedHour ?? row.hour);
      if (revealed > from && revealed <= playhead && (pageKey == null || !presented.get(pageKey)?.has(item.key))) entering.add(item.key);
    }
  }
  if (typeof playhead === 'number') for (const [key, rows] of index.shown) presented.set(key, new Set([...presented.get(key) ?? [], ...rows]));
  return { entering, playhead, index, next: typeof playhead === 'number' ? nextReveal(index, playhead) : 0, presented };
}

// Commit clock history separately so unchanged eligibility retains its identity.
export function logRevealHistory(previous: LogRevealHistory, reveals: LogReveals, playhead?: number | null): LogRevealHistory {
  return { playhead, from: revealFrom(previous, playhead), loaded: reveals.index.loaded,
    presented: typeof playhead === 'number'
      ? playhead !== previous.playhead ? reveals.index.shown : reveals === previous.reveals ? previous.presented : reveals.presented
      : reveals.presented, reveals };
}

/** A row remounted by scrolling is not another arrival. A continuously mounted
 * entry keeps its CSS animation through forward ticks and metadata updates.
 * Remember entries while their payload is held; released pages are protected
 * by `logReveals` above. */
export function logVisibleEntries(items: readonly LogItem<unknown>[], start: number, end: number,
  eligible: ReadonlySet<string>, previous: ReadonlySet<string>, mounted: ReadonlySet<string> = new Set(),
  continueMotion = false): { entering: ReadonlySet<string>; seen: ReadonlySet<string> } {
  const seen = new Set(items.filter(item => item.kind === 'row' && previous.has(item.key)).map(item => item.key));
  const entering = new Set<string>();
  for (let at = start; at < end; at++) {
    const item = items[at];
    if (item.kind === 'row' && ((continueMotion && mounted.has(item.key)) || (eligible.has(item.key) && !seen.has(item.key)))) {
      entering.add(item.key);
      seen.add(item.key);
    }
  }
  return { entering, seen };
}
