import { useEffect, useState } from 'react';
import { useContentRevision } from './contentRevision';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack } from './model';
import { loadTransientProduct } from './products';
import { spriteFrames, spriteIcons, spriteId } from './spriteArtwork';
import type { SpriteFrame } from './spriteArtwork';

const NONE: ReadonlyMap<string, string | null> = new Map();

/** Images for the exact sprite frames a product returned (doctrine icons, mission icons), loaded
 * on demand through the shared sprite route. The returned lookup takes a frame and gives its
 * image, null when it has no drawable art, or undefined while loading, so the widget draws its
 * text without a picture meanwhile. */
export function useSpriteArtwork(runtime: WidgetRuntime, track: BoundTrack | undefined, sprites: Iterable<SpriteFrame | null | undefined>,
  active: boolean): (sprite: SpriteFrame | null | undefined) => string | null {
  const revision = useContentRevision(runtime.client);
  const asked = spriteFrames(sprites);
  const request = track && asked.length ? { sourceId: track.source.id, revision: track.source.revision, product: 'sprite-artwork',
    params: { sprites: asked } } : null;
  const key = JSON.stringify([request, revision]);
  const [state, setState] = useState<{ client: WidgetRuntime['client']; key: string; icons: ReadonlyMap<string, string | null> }>();
  useEffect(() => {
    if (!active || !request) return;
    const controller = new AbortController();
    void loadTransientProduct<unknown>(runtime.client, request, controller.signal).then(reply => {
      if (!controller.signal.aborted) setState({ client: runtime.client, key, icons: spriteIcons(reply, asked) });
    }).catch(() => {
      // Art is decoration: a refused request leaves the names without pictures.
      if (!controller.signal.aborted) setState({ client: runtime.client, key, icons: NONE });
    });
    return () => controller.abort();
    // The key holds the source, the frames and the content revision.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runtime.client, key, active]);
  const icons = state?.client === runtime.client && state.key === key ? state.icons : NONE;
  return sprite => (sprite && icons.get(spriteId(sprite))) ?? null;
}
