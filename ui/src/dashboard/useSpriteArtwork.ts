import { useEffect, useMemo, useRef, useState } from 'react';
import { useContentRevision } from './contentRevision';
import type { DashboardClient } from './client';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack } from './model';
import { loadTransientProduct } from './products';
import { SpriteCaches, spriteFrames, spriteId } from './spriteArtwork';
import type { SpriteFrame } from './spriteArtwork';
import { PerClient } from './prepare.ts';
import { productRequest } from './request.ts';
import { inFrame } from './frameUpdates.ts';
import { NO_ART, PENDING_ART, failedArt, type ArtResolution } from '../families/art/types.ts';

const held = new PerClient<SpriteCaches>();

function cachesOf(client: DashboardClient): SpriteCaches {
  let caches = held.get(client);
  if (!caches) {
    caches = new SpriteCaches();
    held.set(client, caches);
  }
  return caches;
}

export function useSpriteArtResolutions(
  runtime: WidgetRuntime,
  track: BoundTrack | undefined,
  sprites: Iterable<SpriteFrame | null | undefined>,
  active: boolean,
  paint?: (icons: ReadonlyMap<string, string | null>) => void,
): (sprite: SpriteFrame | null | undefined) => ArtResolution {
  const revision = useContentRevision(runtime.client);
  const scope = track ? JSON.stringify([track.source.id, track.source.revision, revision]) : '';
  const caches = cachesOf(runtime.client);
  const icons = caches.icons(scope);
  const resolutions = caches.resolutions(scope);
  const errors = caches.errors(scope);
  const [landed, setLanded] = useState(0);
  const paintRef = useRef(paint);
  paintRef.current = paint;
  const asked = spriteFrames(sprites).filter(sprite => {
    const id = spriteId(sprite);
    return !resolutions.has(id) && !errors.has(id);
  });
  const wantedIds = useMemo(() => new Set(asked.map(spriteId)), [asked.map(spriteId).join('\0')]);
  const wantedRef = useRef(wantedIds);
  wantedRef.current = wantedIds;

  useEffect(() => {
    const release = caches.hold(scope);
    let scheduled = false;
    const stop = caches.watch(scope, (nextIcons, nextErrors) => {
      if (scheduled) return;
      let relevant = false;
      for (const id of wantedRef.current) {
        if (nextIcons.has(id) || nextErrors.has(id)) { relevant = true; break; }
      }
      if (!relevant && wantedRef.current.size > 0) return;
      scheduled = true;
      inFrame(() => {
        scheduled = false;
        if (paintRef.current) paintRef.current(nextIcons);
        else setLanded(count => count + 1);
      });
    });
    if (caches.icons(scope) !== icons) setLanded(count => count + 1);
    return () => { stop(); release(); };
  }, [caches, scope]);

  const key = JSON.stringify([track?.source.id, track?.source.revision, revision, scope, asked]);
  useEffect(() => {
    if (!active || !track || !asked.length) return;
    caches.request(scope, asked, (batch, signal) => loadTransientProduct<unknown>(
      runtime.client,
      productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'sprite-artwork', params: { sprites: batch } }),
      signal,
    ));
  }, [runtime.client, caches, scope, key, active]);

  return useMemo(() => (sprite: SpriteFrame | null | undefined): ArtResolution => {
    if (!sprite) return NO_ART;
    const id = spriteId(sprite);
    const err = errors.get(id);
    if (err !== undefined) return failedArt(err, true);
    return resolutions.get(id) ?? PENDING_ART;
  }, [resolutions, errors, landed]);
}

export function useSpriteArtwork(
  runtime: WidgetRuntime,
  track: BoundTrack | undefined,
  sprites: Iterable<SpriteFrame | null | undefined>,
  active: boolean,
  paint?: (icons: ReadonlyMap<string, string | null>) => void,
): (sprite: SpriteFrame | null | undefined) => string | null {
  const resolve = useSpriteArtResolutions(runtime, track, sprites, active, paint);
  return sprite => {
    const art = resolve(sprite);
    return art.status === 'ready' ? art.url : null;
  };
}
