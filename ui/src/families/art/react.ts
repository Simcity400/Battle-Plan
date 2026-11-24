import { useCallback, useEffect, useState } from 'react';
import { sharedDomArtCache, type DomArtCache } from './domCache.ts';
import type { ArtResolution, DecodedArtState } from './types.ts';

export function useDomArt(
  resolution: ArtResolution,
  contentRevision = 0,
  cache: DomArtCache<HTMLImageElement> = sharedDomArtCache,
): DecodedArtState<HTMLImageElement> {
  const [, setTick] = useState(0);
  const asset = resolution.kind === 'ready' ? resolution.asset : null;

  useEffect(() => {
    cache.setContentRevision(contentRevision);
  }, [cache, contentRevision]);

  useEffect(() => {
    if (!asset) return;
    return cache.subscribe(asset, () => setTick(value => value + 1));
  }, [cache, asset]);

  const state = cache.resolve(resolution);
  const retry = useCallback(() => {
    if (state.kind === 'failed') state.retry();
  }, [state]);

  return state.kind === 'failed' ? { ...state, retry } : state;
}
