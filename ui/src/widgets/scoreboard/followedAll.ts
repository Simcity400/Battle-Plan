/** Several queries followed together while their widget is on screen, as `useFollow` follows one: the widget renders
 * only when what it renders for changes, never as answers land. */
import { useCallback, useEffect, useMemo, useRef, useSyncExternalStore } from 'react';
import type { RefObject } from 'react';
import { perfLog } from '../../shell/perf.ts';
import { measuredWidget } from '../../dashboard/widgetPerf.ts';
import { canonical } from '../../dashboard/follow/store.ts';
import type { FollowEntry, FollowHandle, FollowQuery, FollowStore } from '../../dashboard/follow/store';
import { observeOnScreen } from '../../dashboard/follow/useFollow.ts';
import type { OnScreen } from '../../dashboard/follow/useFollow';
import type { Live } from '../../charts/live';

export interface FollowedAll {
  /** Each query's entry as rendered, index for index; null for a query not asked, and every one while off screen. */
  entries: readonly (FollowEntry | null)[];
  /** Why one is not followed. */
  reason?: string;
  /** Each query's entry now, read where values are painted: the set the queries asked last, or null while it is not
   * held (off screen, or the queries just changed). */
  current: (at: number) => FollowEntry | null;
  /** Told when the entries `current` gives change. */
  subscribe: (changed: () => void) => () => void;
}

/** `renders`: `entries`, the widget renders when an entry is followed or let go (it reads `entries`); `availability`,
 * only when a query is refused or no longer is (it reads its entries by `current`, so a new set of queries, such as a
 * top's new members, costs no render of its own). Neither renders as a follow settles or answers. */
export function useFollowedAll(store: FollowStore | null, queries: readonly (FollowQuery | null)[], target: RefObject<Element | null>,
  onScreen: OnScreen = observeOnScreen, renders: 'entries' | 'availability' = 'entries'): FollowedAll {
  const widget = measuredWidget();
  const asked = useRef(queries); asked.current = queries;
  const key = JSON.stringify(queries.map(query => query ? canonical(query) : null));
  const box = useRef<{ key: string | null; entries: (FollowEntry | null)[]; listeners: Set<() => void>; sets: Set<() => void> }>(null);
  box.current ??= { key: null, entries: [], listeners: new Set(), sets: new Set() };
  const held = box.current;
  const subscribe = useCallback((changed: () => void) => { held.listeners.add(changed); return () => { held.listeners.delete(changed); }; }, [held]);
  const snapshot = useSyncExternalStore(subscribe, () => held.entries.map(entry => !entry ? ''
    : `${renders === 'entries' ? entry.id : ''}\u0001${entry.status === 'unavailable' ? entry.reason ?? 'unavailable' : ''}`).join('\u0000'), () => '');
  // A new set of queries while on screen is taken in place: the new ones are acquired before the old are let go, so the
  // entries never empty between them (run 13: a render as the old set left and another as the new one was seen).
  const keyed = useRef(key); keyed.current = key;
  const swap = useRef<(() => void) | null>(null);
  useEffect(() => {
    const element = target.current;
    if (!store || !element) return;
    let handles: (FollowHandle | null)[] | null = null;
    let unwatch: (() => void)[] = [];
    const tell = () => { for (const changed of held.listeners) changed(); };
    const moved = () => { for (const changed of [...held.sets]) changed(); tell(); };
    const release = (gone: (FollowHandle | null)[] | null, stops: (() => void)[]) => {
      for (const stop of stops) stop();
      for (const handle of gone ?? []) handle?.release();
    };
    const take = () => {
      const gone = handles, stops = unwatch;
      handles = asked.current.map(query => query ? store.acquire(query) : null);
      unwatch = [];
      held.key = keyed.current;
      held.entries = handles.map(handle => handle?.entry ?? null);
      for (const handle of handles) {
        if (!handle) continue;
        if (widget !== null) perfLog?.followWidget(widget, handle.entry.id);
        unwatch.push(store.watch(handle.entry, tell));
      }
      release(gone, stops);
      moved();
    };
    const leave = () => {
      release(handles, unwatch);
      handles = null; unwatch = [];
      if (held.key !== null) { held.key = null; held.entries = []; moved(); }
    };
    swap.current = () => { if (handles && held.key !== keyed.current) take(); };
    const stop = onScreen(element, visible => {
      if (visible && !handles) take();
      else if (!visible && handles) leave();
    });
    return () => { swap.current = null; stop(); leave(); };
  }, [store, target, onScreen, held, widget]);
  useEffect(() => { swap.current?.(); }, [key]);
  const current = useCallback((at: number) => held.key === key ? held.entries[at] ?? null : null, [held, key]);
  const watchSets = useCallback((changed: () => void) => { held.sets.add(changed); return () => { held.sets.delete(changed); }; }, [held]);
  return useMemo(() => {
    const entries = held.key === key ? held.entries : queries.map(() => null);
    const unavailable = entries.find((entry, at) => queries[at] && entry?.status === 'unavailable');
    return { entries, reason: unavailable?.reason, current, subscribe: watchSets };
  }, [snapshot, held, key, current, watchSets]); // eslint-disable-line react-hooks/exhaustive-deps
}

/** What a slot was last painted from: an entry that had answered, and its live source. */
export type HandedSources<T> = Map<string, { entry: FollowEntry; live: Live<T> }>;

/** Query `at` of a set followed with `useFollowedAll` as a live source through a change of the set (a top's new
 * members asked under new queries): the current entry's values once it has answered; until then the last answered
 * entry's of the same `slot` in `held`, so the values never blank for the step a new follow waits (its rows are named,
 * so a row it lacks is missing, never another's); with neither, `waiting`. `entry()` is the entry the values are read
 * from, for the hour they are of. `held` belongs to one store and timeline. */
export function handedLive<T>(store: Pick<FollowStore, 'watch'>, all: () => FollowedAll, at: number, held: HandedSources<T>, slot: string,
  make: (entry: FollowEntry) => Live<T>, waiting: (entry: FollowEntry | null) => T): Live<T> & { entry: () => FollowEntry | null } {
  const made = new WeakMap<FollowEntry, Live<T>>();
  const liveOf = (entry: FollowEntry) => { let live = made.get(entry); if (!live) made.set(entry, live = make(entry)); return live; };
  const source = () => {
    const entry = all().current(at);
    if (entry?.status === 'unavailable') { held.delete(slot); return { entry, live: liveOf(entry) }; }
    if (entry && entry.answer !== undefined) {
      const live = liveOf(entry);
      if (held.get(slot)?.entry !== entry) held.set(slot, { entry, live });
      return { entry, live };
    }
    return held.get(slot) ?? { entry, live: null };
  };
  return {
    read: () => { const { entry, live } = source(); return live ? live.read() : waiting(entry); },
    entry: () => source().entry,
    subscribe: changed => {
      let watched: FollowEntry | null = null, stop = () => {};
      const rewatch = () => {
        const entry = all().current(at);
        if (entry === watched) return;
        stop(); watched = entry;
        stop = entry ? store.watch(entry, changed) : () => {};
      };
      rewatch();
      const stopSets = all().subscribe(() => { rewatch(); changed(); });
      return () => { stopSets(); stop(); };
    },
  };
}
