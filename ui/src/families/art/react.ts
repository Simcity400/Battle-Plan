import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { DashboardClient } from '../../dashboard/client';
import { useContentRevision } from '../../dashboard/contentRevision.ts';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { inFrame } from '../../dashboard/frameUpdates.ts';
import type { BoundTrack } from '../../dashboard/model';
import { PerClient } from '../../dashboard/prepare.ts';
import { loadTransientProduct } from '../../dashboard/products.ts';
import { identityBrokerFor } from '../identity/broker.ts';
import { useIdentityScope } from '../identity/react.ts';
import type { ArtHandle } from '../plan.ts';
import { sharedDomArtCache, type DomArtCache } from './domCache.ts';
import { readArtDemands, requestDemandedFlags, type ArtResolutions } from './provider.ts';
import { SpriteCaches, artCacheScope, artCalls, type ArtFailure } from './transport.ts';
import { PENDING_ART, artHandleKey, failedArt, type ArtResolution, type ArtResult } from './types.ts';

const caches = new PerClient<SpriteCaches>();
const failureArts = new WeakMap<ArtFailure, ArtResolution>();

function failureArt(failure: ArtFailure): ArtResolution {
  let held = failureArts.get(failure);
  if (!held) failureArts.set(failure, held = failedArt(failure.reason, failure.retryable));
  return held;
}

function cachesOf(client: DashboardClient): SpriteCaches {
  let held = caches.get(client);
  if (!held) {
    held = new SpriteCaches();
    caches.set(client, held);
  }
  return held;
}

export function useArtResolutions(
  runtime: WidgetRuntime,
  track: BoundTrack | undefined,
  handles: readonly ArtHandle[],
  active: boolean,
): (handle: ArtHandle) => ArtResolution {
  const revision = useContentRevision(runtime.client);
  const scope = track ? artCacheScope(track, revision) : '';
  const held = cachesOf(runtime.client);
  const resolutions = held.resolutions(scope);
  const failures = held.failures(scope);
  const settled = held.version(scope);
  const [landed, setLanded] = useState(0);
  const asked = handles.filter(handle => {
    const id = artHandleKey(handle);
    return !resolutions.has(id) && failures.get(id)?.retryable !== false;
  });
  const askedIds = asked.map(artHandleKey);
  const askedKey = JSON.stringify(askedIds);
  const wantedIds = useMemo(() => new Set(askedIds), [askedKey]);
  const wantedRef = useRef(wantedIds);
  wantedRef.current = wantedIds;

  useEffect(() => {
    const release = held.hold(scope);
    let scheduled = false;
    const stop = held.watch(scope, (nextResolutions, nextFailures) => {
      if (scheduled) return;
      let relevant = false;
      for (const id of wantedRef.current) {
        if (nextResolutions.has(id) || nextFailures.has(id)) { relevant = true; break; }
      }
      if (!relevant) return;
      scheduled = true;
      inFrame(() => {
        scheduled = false;
        setLanded(count => count + 1);
      });
    });
    if (wantedRef.current.size && held.version(scope) !== settled) setLanded(count => count + 1);
    return () => { stop(); release(); };
  }, [held, scope]);

  const key = JSON.stringify([scope, askedKey]);
  useEffect(() => {
    if (!active || !track || !asked.length) return;
    held.request(scope, asked, (batch, signal) => artCalls((request, sent) => loadTransientProduct<unknown>(runtime.client, request, sent), track, batch, signal));
  }, [runtime.client, held, scope, key, active, landed]);

  return useMemo(() => (handle: ArtHandle): ArtResolution => {
    const id = artHandleKey(handle);
    const failed = failures.get(id);
    if (failed !== undefined) return failureArt(failed);
    return resolutions.get(id) ?? PENDING_ART;
  }, [resolutions, failures, landed]);
}

export interface LaneArtResolutions {
  readonly resolutions: ArtResolutions | undefined;
  subscribe(changed: () => void): () => void;
}

export function useArtHandleResolutions(
  runtime: WidgetRuntime,
  track: BoundTrack | undefined,
  active: boolean,
  forward?: (handles: readonly ArtHandle[]) => void,
) {
  const client = runtime.client;
  const scope = useIdentityScope(client, track?.source, track?.branch.id, active && !!track);
  const revision = useContentRevision(client);
  const demandScope = track ? artCacheScope(track, revision) : '';
  const cache = cachesOf(client);
  const lane = useMemo(() => {
    let handles: readonly ArtHandle[] = [];
    let resolutions: ArtResolutions | undefined;
    const listeners = new Set<() => void>();
    return {
      get resolutions() { return resolutions; },
      subscribe(changed: () => void) {
        listeners.add(changed);
        return () => { listeners.delete(changed); };
      },
      demand(read: readonly ArtHandle[]) { handles = readArtDemands(handles, read); },
      refresh(current: BoundTrack | undefined, enabled: boolean) {
        if (!handles.length && !resolutions) return;
        const settled = cache.resolutions(demandScope);
        const failures = cache.failures(demandScope);
        if (enabled && current && handles.length) {
          cache.request(demandScope, handles, (batch, signal) => artCalls((request, sent) => loadTransientProduct<unknown>(client, request, sent), current, batch, signal));
        }
        const next: Record<string, ArtResolution> = {};
        let changed = handles.length !== Object.keys(resolutions ?? {}).length;
        for (const handle of handles) {
          const id = artHandleKey(handle);
          const failed = failures.get(id);
          next[id] = failed ? failureArt(failed) : settled.get(id) ?? PENDING_ART;
          if (next[id] !== resolutions?.[id]) changed = true;
        }
        if (changed) {
          resolutions = handles.length ? next : undefined;
          for (const listener of listeners) listener();
        }
      },
    };
  }, [client, cache, demandScope]);
  useEffect(() => cache.hold(demandScope), [cache, demandScope]);
  useEffect(() => {
    const refresh = () => lane.refresh(track, active);
    const stop = cache.watch(demandScope, refresh);
    refresh();
    return stop;
  }, [cache, demandScope, lane, track, active]);
  const onArtDemands = useCallback((handles: readonly ArtHandle[]) => {
    forward?.(handles);
    if (scope) requestDemandedFlags(identityBrokerFor(client), scope, handles, track?.source);
    lane.demand(handles);
    lane.refresh(track, active);
  }, [forward, scope, client, track, active, lane]);
  return useMemo(() => ({
    get resolutions() { return lane.resolutions; },
    subscribe: lane.subscribe,
    onArtDemands,
  }), [lane, onArtDemands]);
}

export function useDomArt(
  resolution: ArtResolution,
  contentRevision = 0,
  cache: DomArtCache<HTMLImageElement> = sharedDomArtCache,
): ArtResult<HTMLImageElement> {
  const [, setTick] = useState(0);
  const url = resolution.status === 'ready' ? resolution.url : null;
  const shown = useRef<string>('');

  useEffect(() => {
    cache.setContentRevision(contentRevision);
  }, [cache, contentRevision]);

  useEffect(() => {
    if (!url) return;
    const stop = cache.subscribe(url, () => setTick(value => value + 1));
    if (cache.stateOfUrl(url) !== shown.current) setTick(value => value + 1);
    return stop;
  }, [cache, url]);

  const state = cache.resolve(resolution);
  shown.current = state.status;
  const retry = useCallback(() => {
    if (state.status === 'failed') state.retry?.();
  }, [state]);

  return state.status === 'failed' ? { ...state, retry } : state;
}
