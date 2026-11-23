import { useEffect, useState } from 'react';
import { useContentRevision } from './contentRevision';
import type { DashboardClient } from './client';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack } from './model';
import { loadTransientProduct } from './products';
import { SpriteCaches, spriteFrames, spriteIcons, spriteId } from './spriteArtwork';
import type { SpriteFrame } from './spriteArtwork';
import { PerClient } from './prepare.ts';
import { productRequest } from './request.ts';

const held = new PerClient<SpriteCaches>();

function cachesOf(client: DashboardClient): SpriteCaches {
  let caches = held.get(client);
  if (!caches) { caches = new SpriteCaches(); held.set(client, caches); }
  return caches;
}

export function useSpriteArtwork(runtime: WidgetRuntime, track: BoundTrack | undefined, sprites: Iterable<SpriteFrame | null | undefined>,
  active: boolean, paint?: (icons: ReadonlyMap<string, string | null>) => void): (sprite: SpriteFrame | null | undefined) => string | null {
  const revision = useContentRevision(runtime.client);
  const scope = track ? JSON.stringify([track.source.id, track.source.revision, revision]) : '';
  const caches = cachesOf(runtime.client);
  const icons = caches.icons(scope);
  const [, setLanded] = useState(0);
  useEffect(() => {
    const release = caches.hold(scope);
    if (caches.icons(scope) !== icons) setLanded(count => count + 1);
    return release;
  }, [caches, scope]);
  const asked = spriteFrames(sprites).filter(sprite => !icons.has(spriteId(sprite)));
  const request = track && asked.length ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'sprite-artwork',
    params: { sprites: asked } }) : null;
  const key = JSON.stringify([request, revision, scope]);
  useEffect(() => {
    if (!active || !request) return;
    const controller = new AbortController();
    void loadTransientProduct<unknown>(runtime.client, request, controller.signal).then(reply => {
      if (controller.signal.aborted) return;
      const held = caches.icons(scope);
      for (const [id, icon] of spriteIcons(reply, asked)) held.set(id, icon);
      if (paint) paint(held);
      else setLanded(count => count + 1);
    }).catch(() => {
    });
    return () => controller.abort();
  }, [runtime.client, key, active, paint]);
  return sprite => (sprite && icons.get(spriteId(sprite))) ?? null;
}
