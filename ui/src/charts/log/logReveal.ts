import type { LogItem } from './logData';

export interface LogRevealHistory {
  playhead?: number | null;
  from: number | null;
  loaded: ReadonlySet<string>;
  /** Stable server windows already presented at this playhead. */
  presented: ReadonlyMap<string, number>;
}

export const EMPTY_LOG_REVEALS: LogRevealHistory = { from: null, loaded: new Set(), presented: new Map() };

/** Loading old rows is not an event. Keep the most recent advancement interval
 * across asynchronous loads at the same hour, so genuinely new live rows enter. */
export function logReveals(previous: LogRevealHistory, items: readonly LogItem<unknown>[], playhead?: number | null): {
  history: LogRevealHistory;
  entering: ReadonlySet<string>;
} {
  const from = typeof playhead === 'number' && typeof previous.playhead === 'number'
    ? playhead > previous.playhead ? previous.playhead : playhead === previous.playhead ? previous.from : null
    : null;
  const loaded = new Set<string>();
  const presented = typeof playhead === 'number' && playhead === previous.playhead ? new Map(previous.presented) : new Map<string, number>();
  const entering = new Set<string>();
  for (const item of items) {
    if (item.kind !== 'row') continue;
    const { pageKey, row } = item;
    if (pageKey != null) loaded.add(pageKey);
    if (playhead === undefined) {
      if (pageKey == null || previous.loaded.has(pageKey)) entering.add(item.key);
    } else if (typeof playhead === 'number' && from != null) {
      const revealed = Math.max(row.hour, row.revealedHour ?? row.hour);
      if (revealed > from && revealed <= playhead && (pageKey == null || presented.get(pageKey) !== playhead)) entering.add(item.key);
    }
  }
  if (typeof playhead === 'number') for (const key of loaded) presented.set(key, playhead);
  return { history: { playhead, from, loaded, presented }, entering };
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
