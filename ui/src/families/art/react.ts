import { useCallback, useEffect, useState } from 'react';
import { sharedDomArtCache, type DomArtCache } from './domCache.ts';
import type { ArtResolution, ArtResult } from './types.ts';

export function useDomArt(
  resolution: ArtResolution,
  contentRevision = 0,
  cache: DomArtCache<HTMLImageElement> = sharedDomArtCache,
): ArtResult<HTMLImageElement> {
  const [, setTick] = useState(0);
  const url = resolution.status === 'ready' ? resolution.url : null;

  useEffect(() => {
    cache.setContentRevision(contentRevision);
  }, [cache, contentRevision]);

  useEffect(() => {
    if (!url) return;
    return cache.subscribe(url, () => setTick(value => value + 1));
  }, [cache, url]);

  const state = cache.resolve(resolution);
  const retry = useCallback(() => {
    if (state.status === 'failed') state.retry?.();
  }, [state]);

  return state.status === 'failed' ? { ...state, retry } : state;
}
