import { useEffect, useMemo, useRef, useState } from 'react';
import { useContentRevision } from './contentRevision.ts';
import type { DashboardClient, ProductRequest } from './client';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack } from './model';
import { loadTransientProduct } from './products.ts';
import { SpriteCaches, portraitResolutions, spriteFrames, spriteHandle, spriteResolutions, textureResolutions } from './spriteArtwork.ts';
import type { SpriteFrame } from './spriteArtwork.ts';
import { PerClient } from './prepare.ts';
import { productRequest } from './request.ts';
import { inFrame } from './frameUpdates.ts';
import type { GatedHour } from './regionTime';
import type { ArtResolution as GeneratedArtResolution, LeaderPortraitsReply } from '../generated/dashboard';
import { NO_ART, PENDING_ART, artHandleKey, failedArt, type ArtResolution } from '../families/art/types.ts';
import type { ArtHandle } from '../families/plan.ts';

const held = new PerClient<SpriteCaches>();
const PORTRAIT_EDGE = 96;

function cachesOf(client: DashboardClient): SpriteCaches {
  let caches = held.get(client);
  if (!caches) {
    caches = new SpriteCaches();
    held.set(client, caches);
  }
  return caches;
}

export type ArtLoad = (request: ProductRequest, signal: AbortSignal) => Promise<unknown>;

export async function fetchArtHandles(load: ArtLoad, track: Pick<BoundTrack, 'source' | 'branch'>, handles: readonly ArtHandle[], signal: AbortSignal): Promise<ReadonlyMap<string, GeneratedArtResolution>> {
  const sprites: SpriteFrame[] = [];
  const textures: { path: string; frame: number; frames: number }[] = [];
  const portraits = new Map<number, string[]>();
  for (const handle of handles) {
    if (handle.kind === 'sprite') {
      sprites.push({ key: handle.key, frame: handle.frame ?? 0, ...(handle.recorded !== undefined ? { recorded: handle.recorded } : {}), ...(handle.text ? { text: true } : {}) });
    } else if (handle.kind === 'path') {
      textures.push({ path: handle.path, frame: handle.frame ?? 0, frames: handle.frames ?? 1 });
    } else if (handle.kind === 'portrait') {
      const list = portraits.get(handle.hour);
      if (list) list.push(handle.leader);
      else portraits.set(handle.hour, [handle.leader]);
    }
  }
  const replies: Promise<ReadonlyMap<string, GeneratedArtResolution>>[] = [];
  if (sprites.length || textures.length) {
    replies.push(load(
      productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'sprite-artwork', params: { sprites, ...(textures.length ? { textures } : {}) } }),
      signal,
    ).then(reply => new Map([...(sprites.length ? spriteResolutions(reply, sprites) : []), ...textureResolutions(reply, handles)])));
  }
  for (const [hour, references] of portraits) {
    replies.push(load(
      productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'unit-leader-portraits', params: { branchId: track.branch.id, hour: hour as GatedHour, references, edge: PORTRAIT_EDGE } }),
      signal,
    ).then(reply => portraitResolutions(reply as LeaderPortraitsReply, hour)));
  }
  const merged = new Map<string, GeneratedArtResolution>();
  for (const reply of await Promise.all(replies)) for (const [id, resolution] of reply) merged.set(id, resolution);
  return merged;
}

export function useArtResolutions(
  runtime: WidgetRuntime,
  track: BoundTrack | undefined,
  handles: readonly ArtHandle[],
  active: boolean,
  paint?: (icons: ReadonlyMap<string, string | null>) => void,
): (handle: ArtHandle) => ArtResolution {
  const revision = useContentRevision(runtime.client);
  const scope = track ? JSON.stringify([track.source.id, track.source.revision, revision]) : '';
  const caches = cachesOf(runtime.client);
  const icons = caches.icons(scope);
  const resolutions = caches.resolutions(scope);
  const errors = caches.errors(scope);
  const [landed, setLanded] = useState(0);
  const paintRef = useRef(paint);
  paintRef.current = paint;
  const asked = handles.filter(handle => {
    const id = artHandleKey(handle);
    return !resolutions.has(id) && !errors.has(id);
  });
  const askedKey = asked.map(artHandleKey).join('\0');
  const wantedIds = useMemo(() => new Set(asked.map(artHandleKey)), [askedKey]);
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

  const key = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, revision, scope, askedKey]);
  useEffect(() => {
    if (!active || !track || !asked.length) return;
    caches.request(scope, asked, (batch, signal) => fetchArtHandles((request, held) => loadTransientProduct<unknown>(runtime.client, request, held), track, batch, signal));
  }, [runtime.client, caches, scope, key, active]);

  return useMemo(() => (handle: ArtHandle): ArtResolution => {
    const id = artHandleKey(handle);
    const err = errors.get(id);
    if (err !== undefined) return failedArt(err, true);
    return resolutions.get(id) ?? PENDING_ART;
  }, [resolutions, errors, landed]);
}

export function useSpriteArtResolutions(
  runtime: WidgetRuntime,
  track: BoundTrack | undefined,
  sprites: Iterable<SpriteFrame | null | undefined>,
  active: boolean,
  paint?: (icons: ReadonlyMap<string, string | null>) => void,
): (sprite: SpriteFrame | null | undefined) => ArtResolution {
  const resolve = useArtResolutions(runtime, track, spriteFrames(sprites).map(spriteHandle), active, paint);
  return useMemo(() => (sprite: SpriteFrame | null | undefined): ArtResolution => (sprite ? resolve(spriteHandle(sprite)) : NO_ART), [resolve]);
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
