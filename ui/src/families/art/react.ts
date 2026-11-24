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
import { mergeArtDemands, needsArtResolution, requestDemandedFlags, type ArtResolutions } from './provider.ts';
import { SpriteCaches, artCacheScope, fetchArtHandles } from './transport.ts';
import { PENDING_ART, artHandleKey, failedArt, type ArtResolution, type ArtResult } from './types.ts';

const NO_HANDLES: readonly ArtHandle[] = [];
const caches = new PerClient<SpriteCaches>();

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
  const errors = held.errors(scope);
  const [landed, setLanded] = useState(0);
  const asked = handles.filter(handle => {
    const id = artHandleKey(handle);
    return !resolutions.has(id) && !errors.has(id);
  });
  const askedKey = asked.map(artHandleKey).join('\0');
  const wantedIds = useMemo(() => new Set(asked.map(artHandleKey)), [askedKey]);
  const wantedRef = useRef(wantedIds);
  wantedRef.current = wantedIds;

  useEffect(() => {
    const release = held.hold(scope);
    let scheduled = false;
    const stop = held.watch(scope, (nextResolutions, nextErrors) => {
      if (scheduled) return;
      let relevant = false;
      for (const id of wantedRef.current) {
        if (nextResolutions.has(id) || nextErrors.has(id)) { relevant = true; break; }
      }
      if (!relevant && wantedRef.current.size > 0) return;
      scheduled = true;
      inFrame(() => {
        scheduled = false;
        setLanded(count => count + 1);
      });
    });
    if (held.resolutions(scope) !== resolutions) setLanded(count => count + 1);
    return () => { stop(); release(); };
  }, [held, scope]);

  const key = JSON.stringify([scope, askedKey]);
  useEffect(() => {
    if (!active || !track || !asked.length) return;
    held.request(scope, asked, (batch, signal) => fetchArtHandles((request, sent) => loadTransientProduct<unknown>(runtime.client, request, sent), track, batch, signal));
  }, [runtime.client, held, scope, key, active]);

  return useMemo(() => (handle: ArtHandle): ArtResolution => {
    const id = artHandleKey(handle);
    const err = errors.get(id);
    if (err !== undefined) return failedArt(err, true);
    return resolutions.get(id) ?? PENDING_ART;
  }, [resolutions, errors, landed]);
}

export function useArtHandleResolutions(
  runtime: WidgetRuntime,
  track: BoundTrack | undefined,
  active: boolean,
  forward?: (handles: readonly ArtHandle[]) => void,
): {
  readonly resolutions: ArtResolutions | undefined;
  readonly onArtDemands: (handles: readonly ArtHandle[]) => void;
} {
  const [demanded, setDemanded] = useState<readonly ArtHandle[]>(NO_HANDLES);
  const scope = useIdentityScope(runtime.client, track?.source, track?.branch.id, active && !!track);
  const onArtDemands = useCallback((handles: readonly ArtHandle[]) => {
    forward?.(handles);
    if (scope) requestDemandedFlags(identityBrokerFor(runtime.client), scope, handles, track?.source);
    setDemanded(held => mergeArtDemands(held, handles.filter(needsArtResolution)));
  }, [forward, scope, runtime.client, track?.source]);
  const resolve = useArtResolutions(runtime, track, demanded, active && demanded.length > 0);
  const resolutions = useMemo(() => {
    if (!demanded.length) return undefined;
    const out: Record<string, ArtResolution> = {};
    for (const handle of demanded) out[artHandleKey(handle)] = resolve(handle);
    return out;
  }, [demanded, resolve]);
  return { resolutions, onArtDemands };
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
