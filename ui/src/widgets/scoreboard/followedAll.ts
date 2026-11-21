import { useCallback, useEffect, useMemo, useRef, useSyncExternalStore } from 'react';
import type { RefObject } from 'react';
import { perfLog } from '../../shell/perf.ts';
import { measuredWidget } from '../../dashboard/widgetPerf.ts';
import { canonical } from '../../dashboard/follow/canonical.ts';
import type { FollowEntry, FollowHandle, FollowQuery, FollowStore } from '../../dashboard/follow/store';
import { observeOnScreen } from '../../dashboard/follow/useFollow.ts';
import type { OnScreen } from '../../dashboard/follow/useFollow';
import type { Live } from '../../charts/live';

export interface FollowedAll {
  entries: readonly (FollowEntry | null)[];
  reason?: string;
  current: (at: number) => FollowEntry | null;
  subscribe: (changed: () => void) => () => void;
}

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
  }, [snapshot, held, key, current, watchSets]);
}

export type HandedSources<T> = Map<string, { entry: FollowEntry; live: Live<T> }>;

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
