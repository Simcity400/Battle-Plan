import type { ProductRequest } from '../../dashboard/client';
import { artworkEntry, drawableIcon } from '../../dashboard/artwork.ts';
import type { BoundTrack } from '../../dashboard/model';
import { asFailure } from '../../dashboard/queryError.ts';
import type { GatedHour } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request.ts';
import type { LeaderPortraitsReply, SpriteFrame, SpriteImage, TechnologyArtworkReply } from '../../generated/dashboard';
import type { ArtHandle } from '../plan.ts';
import { spriteImageToArt } from './adapters.ts';
import { artHandleKey, failedArt, productArt, readyArt, type ArtResolution, type ProductArtResolution } from './types.ts';

export type { SpriteFrame };

const PORTRAIT_EDGE = 96;

export const spriteHandle = (sprite: SpriteFrame): ArtHandle => ({ kind: 'sprite', ...sprite });

const spriteId = (sprite: SpriteFrame) => artHandleKey(spriteHandle(sprite));

const optionalKey = (value: unknown) => value === undefined || typeof value === 'string' && value.length > 0;

export function spriteFrames(sprites: Iterable<SpriteFrame | null | undefined>): SpriteFrame[] {
  const shown = new Map<string, SpriteFrame>();
  for (const sprite of sprites)
    if (sprite && typeof sprite.key === 'string' && sprite.key.length > 0 && Number.isSafeInteger(sprite.frame) && sprite.frame >= 0
      && optionalKey(sprite.recorded) && (sprite.text === undefined || typeof sprite.text === 'boolean'))
      shown.set(spriteId(sprite), { key: sprite.key, frame: sprite.frame, ...sprite.recorded === undefined ? {} : { recorded: sprite.recorded },
        ...sprite.text ? { text: true } : {} });
  return [...shown.entries()].sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0).map(([, sprite]) => sprite);
}

export function spriteResolutions(reply: unknown, asked: readonly SpriteFrame[]): Map<string, ArtResolution> {
  const value = reply as { items?: unknown; warnings?: unknown };
  const malformed = () => new Error('The sprite artwork reply is malformed.');
  if (!value || typeof value !== 'object' || !Array.isArray(value.items)
    || !Array.isArray(value.warnings) || !value.warnings.every(note => typeof note === 'string')) throw malformed();
  const ids = new Set(asked.map(spriteId));
  const out = new Map<string, ArtResolution>();
  for (const item of value.items as unknown[]) {
    const image = item as { key?: unknown; frame?: unknown; recorded?: unknown; text?: unknown; icon?: unknown; resolution?: unknown };
    if (!image || typeof image.key !== 'string' || typeof image.frame !== 'number' || !optionalKey(image.recorded) || !('icon' in image)
      || !(image.text === undefined || image.text === true)) throw malformed();
    const id = spriteId({ key: image.key, frame: image.frame, recorded: image.recorded as string | undefined, text: image.text === true });
    if (!ids.has(id) || out.has(id)) throw malformed();
    out.set(id, spriteImageToArt({
      key: image.key,
      frame: image.frame,
      ...(image.recorded !== undefined ? { recorded: image.recorded as string } : {}),
      ...(image.text === true ? { text: true } : {}),
      icon: drawableIcon(image.icon),
      ...(image.resolution !== undefined ? { resolution: image.resolution as ProductArtResolution } : {}),
    } as SpriteImage));
  }
  return out;
}

function textureResolutions(reply: unknown, asked: readonly ArtHandle[]): Map<string, ArtResolution> {
  const value = reply as { textures?: unknown };
  const out = new Map<string, ArtResolution>();
  if (value.textures === undefined) return out;
  if (!Array.isArray(value.textures)) throw new Error('The sprite artwork reply is malformed.');
  const ids = new Set(asked.map(artHandleKey));
  for (const item of value.textures as { path?: unknown; frame?: unknown; frames?: unknown; resolution?: unknown }[]) {
    if (!item || typeof item.path !== 'string' || typeof item.frame !== 'number' || typeof item.frames !== 'number' || !item.resolution) {
      throw new Error('The sprite artwork reply is malformed.');
    }
    const id = artHandleKey({ kind: 'path', path: item.path, frame: item.frame, frames: item.frames });
    if (ids.has(id)) out.set(id, productArt(item.resolution as ProductArtResolution));
  }
  return out;
}

function portraitResolutions(reply: LeaderPortraitsReply, hour: number): Map<string, ArtResolution> {
  if (!reply || !Array.isArray(reply.portraits)) throw new Error('The unit leader portraits reply is malformed.');
  return new Map(reply.portraits.map(portrait => [artHandleKey({ kind: 'portrait', leader: portrait.reference, hour }), productArt(portrait.resolution)]));
}

export function technologyResolutions(reply: TechnologyArtworkReply, handles: readonly ArtHandle[]): Map<string, ArtResolution> {
  if (!reply || !Array.isArray(reply.items) || !reply.items.every(artworkEntry)) throw new Error('The technology artwork reply is malformed.');
  const byToken = new Map(reply.items.map(item => [item.token, item]));
  const out = new Map<string, ArtResolution>();
  for (const handle of handles) {
    if (handle.kind !== 'technology') continue;
    const item = byToken.get(handle.token);
    const icon = item && drawableIcon(item.icon);
    out.set(artHandleKey(handle), !item
      ? failedArt('The technology artwork reply left this technology out', false)
      : icon ? readyArt(icon, { assetId: `technology:${handle.token}` })
        : failedArt('The technology artwork product drew no icon and does not say why', false));
  }
  return out;
}

export async function fetchArtHandles(
  load: (request: ProductRequest, signal: AbortSignal) => Promise<unknown>,
  track: Pick<BoundTrack, 'source' | 'branch'>,
  handles: readonly ArtHandle[],
  signal: AbortSignal,
): Promise<ReadonlyMap<string, ArtResolution>> {
  const sprites: SpriteFrame[] = [];
  const textures: { path: string; frame: number; frames: number }[] = [];
  const portraits = new Map<number, string[]>();
  const technologies = new Map<string, Set<number>>();
  for (const handle of handles) {
    if (handle.kind === 'sprite') {
      sprites.push({ key: handle.key, frame: handle.frame ?? 0, ...(handle.recorded !== undefined ? { recorded: handle.recorded } : {}), ...(handle.text ? { text: true } : {}) });
    } else if (handle.kind === 'path') {
      textures.push({ path: handle.path, frame: handle.frame ?? 0, frames: handle.frames ?? 1 });
    } else if (handle.kind === 'portrait') {
      const list = portraits.get(handle.hour);
      if (list) list.push(handle.leader);
      else portraits.set(handle.hour, [handle.leader]);
    } else if (handle.kind === 'technology') {
      const indices = technologies.get(handle.token) ?? new Set<number>();
      if (handle.databaseIndex !== null && handle.databaseIndex > 0) indices.add(handle.databaseIndex);
      technologies.set(handle.token, indices);
    }
  }
  const replies: Promise<ReadonlyMap<string, ArtResolution>>[] = [];
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
  if (technologies.size) {
    const tokens = [...technologies.keys()].sort();
    const recorded = Object.fromEntries(tokens.filter(token => technologies.get(token)!.size).map(token => [token, [...technologies.get(token)!].sort((a, b) => a - b)]));
    replies.push(load(
      productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'technology-artwork', params: Object.keys(recorded).length ? { tokens, recorded } : { tokens } }),
      signal,
    ).then(reply => technologyResolutions(reply as TechnologyArtworkReply, handles)));
  }
  const merged = new Map<string, ArtResolution>();
  for (const reply of await Promise.all(replies)) for (const [id, resolution] of reply) merged.set(id, resolution);
  return merged;
}

export function artCacheScope(track: Pick<BoundTrack, 'source' | 'branch'>, contentRevision: number): string {
  return JSON.stringify([track.source.id, track.source.revision, track.branch.id, contentRevision]);
}

export const SPRITE_BATCH = 64;
const MAX_SPRITE_CONCURRENCY = 2;

type ArtFetcher = (handles: readonly ArtHandle[], signal: AbortSignal) => Promise<ReadonlyMap<string, ArtResolution>>;

type ArtWatcher = (resolutions: ReadonlyMap<string, ArtResolution>, errors: ReadonlyMap<string, string>) => void;

interface ArtScope {
  users: number;
  resolutions: Map<string, ArtResolution>;
  errors: Map<string, string>;
  inFlight: Set<string>;
  queued: Map<string, ArtHandle>;
  watchers: Set<ArtWatcher>;
  controllers: Set<AbortController>;
  scheduled: boolean;
  running: number;
}

export class SpriteCaches {
  private scopes = new Map<string, ArtScope>();

  private held(scope: string): ArtScope {
    let held = this.scopes.get(scope);
    if (!held) {
      held = { users: 0, resolutions: new Map(), errors: new Map(), inFlight: new Set(), queued: new Map(), watchers: new Set(),
        controllers: new Set(), scheduled: false, running: 0 };
      this.scopes.set(scope, held);
    }
    return held;
  }

  resolutions(scope: string): Map<string, ArtResolution> { return this.held(scope).resolutions; }

  errors(scope: string): Map<string, string> { return this.held(scope).errors; }

  watch(scope: string, watcher: ArtWatcher): () => void {
    const held = this.held(scope);
    held.watchers.add(watcher);
    return () => { held.watchers.delete(watcher); };
  }

  request(scope: string, handles: readonly ArtHandle[], fetch: ArtFetcher): void {
    const held = this.held(scope);
    let added = false;
    for (const handle of handles) {
      const id = artHandleKey(handle);
      if (held.resolutions.has(id) || held.errors.has(id) || held.inFlight.has(id)) continue;
      held.inFlight.add(id);
      held.queued.set(id, handle);
      added = true;
    }
    if (!added || held.scheduled) return;
    held.scheduled = true;
    queueMicrotask(() => {
      held.scheduled = false;
      if (this.scopes.get(scope) !== held) return;
      this.pump(scope, held, fetch);
    });
  }

  private pump(scope: string, held: ArtScope, fetch: ArtFetcher): void {
    while (held.running < MAX_SPRITE_CONCURRENCY && held.queued.size > 0) {
      const entries = [...held.queued.entries()].slice(0, SPRITE_BATCH);
      for (const [id] of entries) held.queued.delete(id);
      const batch = entries.map(([, handle]) => handle);
      const controller = new AbortController();
      held.controllers.add(controller);
      held.running++;
      void fetch(batch, controller.signal).then(parsed => {
        if (controller.signal.aborted || this.scopes.get(scope) !== held) return;
        for (const [id] of entries) {
          held.inFlight.delete(id);
          const resolution = parsed.get(id);
          if (!resolution) {
            held.errors.set(id, 'The artwork reply left this handle out.');
            continue;
          }
          held.errors.delete(id);
          held.resolutions.set(id, resolution);
        }
        for (const watcher of [...held.watchers]) watcher(held.resolutions, held.errors);
      }).catch(error => {
        if (this.scopes.get(scope) !== held) return;
        const reason = controller.signal.aborted ? null : asFailure(error).message;
        for (const [id] of entries) {
          held.inFlight.delete(id);
          if (reason !== null && !held.resolutions.has(id)) held.errors.set(id, reason);
        }
        if (reason !== null) for (const watcher of [...held.watchers]) watcher(held.resolutions, held.errors);
      }).finally(() => {
        held.controllers.delete(controller);
        held.running--;
        if (this.scopes.get(scope) === held) this.pump(scope, held, fetch);
      });
    }
  }

  hold(scope: string): () => void {
    const held = this.held(scope);
    held.users++;
    return () => {
      if (--held.users === 0 && this.scopes.get(scope) === held) {
        for (const controller of held.controllers) controller.abort();
        held.controllers.clear();
        held.queued.clear();
        held.inFlight.clear();
        held.errors.clear();
        this.scopes.delete(scope);
      }
    };
  }
}
