import { useEffect, useState } from 'react';
import { useContentRevision } from './contentRevision';
import type { DashboardClient } from './client';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack } from './model';
import { loadTransientProduct } from './products';
import { spriteFrames, spriteIcons, spriteId } from './spriteArtwork';
import type { SpriteFrame } from './spriteArtwork';
import { PerClient } from './prepare.ts';
import { productRequest } from './request.ts';

const held = new PerClient<{ scope: string; icons: Map<string, string | null> }>();

function heldIcons(client: DashboardClient, scope: string): Map<string, string | null> {
  const now = held.get(client);
  if (now?.scope === scope) return now.icons;
  const icons = new Map<string, string | null>();
  held.set(client, { scope, icons });
  return icons;
}

export function useSpriteArtwork(runtime: WidgetRuntime, track: BoundTrack | undefined, sprites: Iterable<SpriteFrame | null | undefined>,
  active: boolean, paint?: (icons: ReadonlyMap<string, string | null>) => void): (sprite: SpriteFrame | null | undefined) => string | null {
  const revision = useContentRevision(runtime.client);
  const scope = track ? JSON.stringify([track.source.id, track.source.revision, revision]) : '';
  const icons = heldIcons(runtime.client, scope);
  const asked = spriteFrames(sprites).filter(sprite => !icons.has(spriteId(sprite)));
  const request = track && asked.length ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'sprite-artwork',
    params: { sprites: asked } }) : null;
  const key = JSON.stringify([request, revision]);
  const [, setLanded] = useState(0);
  useEffect(() => {
    if (!active || !request) return;
    const controller = new AbortController();
    void loadTransientProduct<unknown>(runtime.client, request, controller.signal).then(reply => {
      if (controller.signal.aborted) return;
      for (const [id, icon] of spriteIcons(reply, asked)) icons.set(id, icon);
      if (paint) paint(icons);
      else setLanded(count => count + 1);
    }).catch(() => {
    });
    return () => controller.abort();
  }, [runtime.client, key, active, paint]);
  return sprite => (sprite && icons.get(spriteId(sprite))) ?? null;
}
