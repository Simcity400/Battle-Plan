import { useCallback, useEffect, useMemo, useState } from 'react';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { SpriteFrame } from '../../dashboard/spriteArtwork.ts';
import { useSpriteArtResolutions } from '../../dashboard/useSpriteArtwork.ts';
import type { ArtHandle } from '../plan.ts';
import { sharedDomArtCache, type DomArtCache } from './domCache.ts';
import { mergeArtDemands } from './provider.ts';
import { artHandleKey, type ArtResolution, type ArtResult } from './types.ts';

const NO_HANDLES: readonly ArtHandle[] = [];

function spriteFrameOf(handle: Extract<ArtHandle, { readonly kind: 'sprite' }>): SpriteFrame {
  return {
    key: handle.key,
    frame: handle.frame ?? 0,
    ...(handle.recorded !== undefined ? { recorded: handle.recorded } : {}),
    ...(handle.text ? { text: true } : {}),
  };
}

export function useArtHandleResolutions(
  runtime: WidgetRuntime,
  track: BoundTrack | undefined,
  active: boolean,
  forward?: (handles: readonly ArtHandle[]) => void,
): {
  readonly resolutions: Readonly<Record<string, ArtResolution>> | undefined;
  readonly onArtDemands: (handles: readonly ArtHandle[]) => void;
} {
  const [demanded, setDemanded] = useState<readonly ArtHandle[]>(NO_HANDLES);
  const onArtDemands = useCallback((handles: readonly ArtHandle[]) => {
    forward?.(handles);
    setDemanded(held => mergeArtDemands(held, handles.filter(handle => handle.kind === 'sprite')));
  }, [forward]);
  const sprites = useMemo(
    () => demanded.flatMap(handle => (handle.kind === 'sprite' ? [spriteFrameOf(handle)] : [])),
    [demanded],
  );
  const resolveSprite = useSpriteArtResolutions(runtime, track, sprites, active && sprites.length > 0);
  const resolutions = useMemo(() => {
    if (!demanded.length) return undefined;
    const out: Record<string, ArtResolution> = {};
    for (const handle of demanded) {
      if (handle.kind === 'sprite') out[artHandleKey(handle)] = resolveSprite(spriteFrameOf(handle));
    }
    return out;
  }, [demanded, resolveSprite]);
  return { resolutions, onArtDemands };
}

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
