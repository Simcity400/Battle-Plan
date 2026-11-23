import { useCallback, useContext, useId, useLayoutEffect, useMemo, useRef, useSyncExternalStore } from 'react';
import type { RefObject } from 'react';
import type { PlaybackController } from '../playback';
import { PreparingPage } from '../preparing.ts';
import type { FollowedAnswer } from './message.ts';
import { FollowedCard, FollowedWidgetKey, installedEntry, type FollowEntry, type FollowQuery, type FollowStore } from './store.ts';
import { observeOnScreen, type OnScreen } from './onScreen.ts';
import { branchBounds, useElement, useFollowedSub, useFollowedWidget, useInstalled, useOnScreen, usePipeline, componentReading, datedReading, datedStore, onInstalled } from './useFollowed.ts';
import type { Reading } from './useFollowed';

export { observeOnScreen, screenHolder, NEAR_SCREEN, DASHBOARD_SCROLL } from './onScreen.ts';
export type { OnScreen } from './onScreen.ts';

const noStructure = () => '';

export const panelStructure = (_answer: FollowedAnswer | undefined, entry: FollowEntry) => entry.panel ? String(entry.version) : '';

export interface Followed {
  status: 'off' | FollowEntry['status'];
  reason?: string;
  structure: string;
  entry: FollowEntry | null;
  hidden: boolean;
  reading: Reading | null;
}

export const followsHead = (followed: Followed) => followed.status === 'followed';

export function followRefusal(...followed: readonly Followed[]): string | null {
  for (const each of followed) if (each.status === 'unavailable' || each.status === 'failed') return each.reason ?? 'The followed reading stopped without a reason';
  return null;
}

export function usePlaybackHour(clock: PlaybackController, on = true): number {
  const subscribe = useCallback((changed: () => void) => on ? clock.subscribe(changed) : () => {}, [clock, on]);
  const read = useCallback(() => on ? clock.getHour() : null, [on, clock]);
  return useSyncExternalStore(subscribe, read, read) ?? clock.getHour();
}

export function useWidgetKey(): string {
  const own = useId();
  return useContext(FollowedWidgetKey) ?? own;
}

export function useFollow(store: FollowStore | null, asked: FollowQuery | null, target: RefObject<Element | null>, paint: ((entry: FollowEntry) => unknown) | null,
  structureOf: (answer: FollowedAnswer | undefined, entry: FollowEntry) => string = noStructure, onScreen: OnScreen = observeOnScreen): Followed {
  const prepared = useContext(PreparingPage);
  const widgetKey = useWidgetKey();
  const pipeline = usePipeline(store?.clock ?? null, store?.client ?? null);
  const query = prepared || !store ? null : asked;
  const element = useElement(target), card = useContext(FollowedCard);
  const visible = useOnScreen(query ? card ?? element : null, onScreen);
  const branch = query ? (query.branchId ?? (query.params as { branchId?: unknown } | undefined)?.branchId) : undefined;
  const pipe = useFollowedWidget(pipeline, store && query ? `${widgetKey}|${store.offset}` : null, { offset: store?.offset ?? 0, pin: null, bounds: store ? branchBounds(store.source, branch) : undefined });
  const follow = useMemo(() => store && typeof branch === 'string' ? { source: store.source, branch, offset: store.offset } : null, [store, branch]);
  const channel = query && typeof query.panel === 'string' ? 'panel' : 'followed';
  const sub = useFollowedSub(pipeline, pipe, follow, query && visible ? query : null, channel, typeof query?.panel === 'string' ? query.panel : undefined);
  const view = useInstalled(pipeline, pipe);
  const entry = useMemo(() => pipeline && pipe !== null && sub !== undefined ? installedEntry(pipeline, pipe, sub) : null, [pipeline, pipe, sub]);
  const painting = useRef(paint);
  painting.current = paint;
  useLayoutEffect(() => entry && pipeline && pipe !== null ? onInstalled(pipeline, pipe, () => painting.current?.(entry)) : undefined, [entry, pipeline, pipe]);
  const status: Followed['status'] = !query ? 'off' : entry ? entry.status : 'pending';
  return {
    status, ...entry?.reason !== undefined ? { reason: entry.reason } : {},
    structure: entry ? structureOf(entry.answer, entry) : '',
    entry, hidden: !!query && !visible,
    reading: query ? datedReading(componentReading(pipeline && pipe !== null ? pipeline.view(pipe) : view, sub), datedStore(pipeline, pipe)) : null,
  };
}
