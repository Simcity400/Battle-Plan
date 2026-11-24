import { useEffect, useMemo, useState } from 'react';
import { useContentRevision } from './contentRevision';
import type { DashboardClient } from './client';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack } from './model';
import { loadTransientProduct } from './products';
import { asFailure } from './queryError.ts';
import { SpriteCaches, spriteFrames, spriteIcons, spriteId } from './spriteArtwork';
import type { SpriteFrame } from './spriteArtwork';
import { PerClient } from './prepare.ts';
import { productRequest } from './request.ts';
import { CURRENT_INSTALL_PROVENANCE, NO_ART, PENDING_ART, failedArt, missingArt, readyArt, type ArtResolution } from '../families/art/types.ts';

interface SpriteState {
  readonly caches: SpriteCaches;
  readonly failed: Map<string, Map<string, string>>;
}

const held = new PerClient<SpriteState>();

function stateOf(client: DashboardClient): SpriteState {
  let entry = held.get(client);
  if (!entry) {
    entry = { caches: new SpriteCaches(), failed: new Map() };
    held.set(client, entry);
  }
  return entry;
}

function failedFor(state: SpriteState, scope: string): Map<string, string> {
  let map = state.failed.get(scope);
  if (!map) {
    map = new Map();
    state.failed.set(scope, map);
  }
  return map;
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
  const state = stateOf(runtime.client);
  const icons = state.caches.icons(scope);
  const errors = failedFor(state, scope);
  const [landed, setLanded] = useState(0);

  useEffect(() => {
    const release = state.caches.hold(scope);
    if (state.caches.icons(scope) !== icons) setLanded(count => count + 1);
    return () => {
      release();
      if (!state.caches.icons(scope).size) state.failed.delete(scope);
    };
  }, [state, scope]);

  const asked = spriteFrames(sprites).filter(sprite => {
    const id = spriteId(sprite);
    return !icons.has(id) && !errors.has(id);
  });
  const request = track && asked.length ? productRequest({
    sourceId: track.source.id,
    revision: track.source.revision,
    product: 'sprite-artwork',
    params: { sprites: asked },
  }) : null;
  const key = JSON.stringify([request, revision, scope]);

  useEffect(() => {
    if (!active || !request) return;
    const controller = new AbortController();
    void loadTransientProduct<unknown>(runtime.client, request, controller.signal).then(reply => {
      if (controller.signal.aborted) return;
      const heldIcons = state.caches.icons(scope);
      const heldErrors = failedFor(state, scope);
      for (const [id, icon] of spriteIcons(reply, asked)) {
        heldIcons.set(id, icon);
        heldErrors.delete(id);
      }
      if (paint) paint(heldIcons);
      else setLanded(count => count + 1);
    }).catch(error => {
      if (controller.signal.aborted) return;
      const reason = asFailure(error).message;
      const heldErrors = failedFor(state, scope);
      for (const sprite of asked) heldErrors.set(spriteId(sprite), reason);
      setLanded(count => count + 1);
    });
    return () => controller.abort();
  }, [runtime.client, key, active, paint]);

  return useMemo(() => (sprite: SpriteFrame | null | undefined): ArtResolution => {
    if (!sprite) return NO_ART;
    const id = spriteId(sprite);
    const err = errors.get(id);
    if (err !== undefined) return failedArt(err, true);
    if (!icons.has(id)) return PENDING_ART;
    const url = icons.get(id);
    return url ? readyArt(url, { assetId: id, provenance: CURRENT_INSTALL_PROVENANCE }) : missingArt('Sprite art not found in current installation');
  }, [icons, errors, landed]);
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
