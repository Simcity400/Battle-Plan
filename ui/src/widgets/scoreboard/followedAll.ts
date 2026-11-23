import { useCallback, useContext, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { RefObject } from 'react';
import { canonical } from '../../dashboard/follow/canonical.ts';
import { FollowedCard, installedEntry, type FollowEntry, type FollowQuery, type FollowStore } from '../../dashboard/follow/store.ts';
import { observeOnScreen, useWidgetKey } from '../../dashboard/follow/useFollow.ts';
import type { OnScreen } from '../../dashboard/follow/useFollow';
import { branchBounds, useElement, useFollowedWidget, useInstalled, useOnScreen, usePipeline } from '../../dashboard/follow/useFollowed.ts';
import { restartPipeline } from '../../dashboard/follow/pipeline.ts';
import { asFailure, type Failure } from '../../dashboard/queryError.ts';
import { PreparingPage } from '../../dashboard/preparing.ts';
import type { Live } from '../../charts/live';

export interface FollowedAll {
  entries: readonly (FollowEntry | null)[];
  reason?: string;
  failure?: string;
  error?: Failure;
  outside?: boolean;
  retry: () => void;
  current: (at: number) => FollowEntry | null;
  subscribe: (changed: () => void) => () => void;
}

export function useFollowedAll(store: FollowStore | null, queries: readonly (FollowQuery | null)[], target: RefObject<Element | null>,
  onScreen: OnScreen = observeOnScreen): FollowedAll {
  const prepared = useContext(PreparingPage);
  const widgetKey = useWidgetKey();
  const pipeline = usePipeline(store?.clock ?? null, store?.client ?? null);
  const element = useElement(target), card = useContext(FollowedCard);
  const wanted = !prepared && !!store && queries.some(Boolean);
  const visible = useOnScreen(wanted ? card ?? element : null, onScreen);
  const first = queries.find(Boolean), branch = first ? first.branchId ?? (first.params as { branchId?: unknown } | undefined)?.branchId : undefined;
  const pipe = useFollowedWidget(pipeline, wanted && store ? `${widgetKey}|${store.offset}` : null, { offset: store?.offset ?? 0, pin: null, bounds: store ? branchBounds(store.source, branch) : undefined });
  const key = JSON.stringify(queries.map(query => query ? canonical(query) : null));
  const asked = useRef(queries); asked.current = queries;
  const [subs, setSubs] = useState<{ key: string; subs: (number | null)[] }>({ key: '', subs: [] });
  useLayoutEffect(() => {
    if (!pipeline || pipe === null || !store || !visible) { setSubs({ key: '', subs: [] }); return; }
    const tokens = asked.current.map(() => ({}));
    const made = asked.current.map((query, at) => {
      if (!query) return null;
      const branch = query.branchId ?? (query.params as { branchId?: unknown } | undefined)?.branchId;
      if (typeof branch !== 'string') return null;
      return pipeline.subscribe(pipe, tokens[at]!, store.source, branch, store.offset, query, typeof query.panel === 'string' ? 'panel' : 'followed',
        typeof query.panel === 'string' ? query.panel : undefined) ?? null;
    });
    setSubs({ key, subs: made });
    return () => { for (const token of tokens) pipeline.unsubscribe(pipe, token); };
  }, [pipeline, pipe, store, visible, key]);
  const view = useInstalled(pipeline, pipe);
  const entries = useMemo(() => queries.map((_, at) => {
    const sub = subs.key === key ? subs.subs[at] : null;
    return pipeline && pipe !== null && sub !== null && sub !== undefined ? installedEntry(pipeline, pipe, sub) : null;
  }), [pipeline, pipe, subs, key]);
  const current = useCallback((at: number) => entries[at] ?? null, [entries]);
  const subscribe = useCallback((changed: () => void) => pipeline && pipe !== null ? pipeline.onInstall(pipe, changed) : () => {}, [pipeline, pipe]);
  const outcome = useMemo(() => {
    const unavailable = entries.find((entry, at) => queries[at] && entry?.status === 'unavailable');
    const failed = entries.filter((entry, at): entry is FollowEntry => !!queries[at] && entry?.status === 'failed');
    const cause = failed.find(entry => entry.error !== undefined) ?? failed[0];
    const failure = cause ? cause.reason ?? 'The followed stream failed' : undefined;
    const error = cause ? cause.error ?? asFailure(new Error(failure!)) : undefined;
    return { reason: unavailable?.reason, failure, error };
  }, [entries, view]);
  const retryable = outcome.error !== undefined;
  const retry = useCallback(() => { if (retryable && pipeline) restartPipeline(pipeline); }, [retryable, pipeline]);
  return useMemo(() => ({
    entries, reason: outcome.reason, ...(outcome.failure && { failure: outcome.failure }), ...(outcome.error && { error: outcome.error }),
    outside: view?.mode === 'outside', retry, current, subscribe,
  }), [entries, outcome, view, retry, current, subscribe]);
}

export function handedLive<T>(all: () => FollowedAll, at: number,
  make: (entry: FollowEntry) => Live<T>, waiting: (entry: FollowEntry | null) => T): Live<T> & { entry: () => FollowEntry | null } {
  const made = new WeakMap<FollowEntry, Live<T>>();
  const liveOf = (entry: FollowEntry) => { let live = made.get(entry); if (!live) made.set(entry, live = make(entry)); return live; };
  const source = () => {
    const entry = all().current(at);
    return { entry, live: entry && (entry.answer !== undefined || entry.status === 'unavailable' || entry.status === 'failed') ? liveOf(entry) : null };
  };
  return {
    read: () => { const { entry, live } = source(); return live ? live.read() : waiting(entry); },
    entry: () => source().entry,
    subscribe: changed => all().subscribe(changed),
  };
}
