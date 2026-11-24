import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { useArtResolutions } from '../../dashboard/useSpriteArtwork.ts';
import type { ArtHandle } from '../plan.ts';
import { sharedDomArtCache, type DomArtCache } from './domCache.ts';
import { mergeArtDemands, needsArtResolution, type ArtResolutions } from './provider.ts';
import { artHandleKey, type ArtResolution, type ArtResult } from './types.ts';

const NO_HANDLES: readonly ArtHandle[] = [];

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
  const onArtDemands = useCallback((handles: readonly ArtHandle[]) => {
    forward?.(handles);
    setDemanded(held => mergeArtDemands(held, handles.filter(needsArtResolution)));
  }, [forward]);
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
