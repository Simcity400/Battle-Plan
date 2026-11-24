import type { ProductRequest } from '../../dashboard/client';
import { artworkEntry, drawableIcon } from '../../dashboard/artwork.ts';
import type { BoundTrack } from '../../dashboard/model';
import { retryDelay, transientError } from '../../dashboard/countryFacts.ts';
import { asFailure } from '../../dashboard/queryError.ts';
import { busyWait } from '../../shell/busy.ts';
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

export const MAX_ART_CALLS = 4;

interface ArtCall {
  readonly handles: readonly ArtHandle[];
  readonly run: () => Promise<ReadonlyMap<string, ArtResolution>>;
}

async function settleCalls(calls: readonly ArtCall[], signal: AbortSignal): Promise<ReadonlyMap<string, ArtResolution>> {
  const merged = new Map<string, ArtResolution>();
  let next = 0;
  const work = async () => {
    while (next < calls.length) {
      const call = calls[next++]!;
      try {
        for (const [id, resolution] of await call.run()) merged.set(id, resolution);
      } catch (error) {
        if (signal.aborted) throw error;
        const failed = failedArt(asFailure(error).message, transientError(error));
        for (const handle of call.handles) merged.set(artHandleKey(handle), failed);
      }
    }
  };
  await Promise.all(Array.from({ length: Math.min(MAX_ART_CALLS, calls.length) }, work));
  return merged;
}

export async function fetchArtHandles(
  load: (request: ProductRequest, signal: AbortSignal) => Promise<unknown>,
  track: Pick<BoundTrack, 'source' | 'branch'>,
  handles: readonly ArtHandle[],
  signal: AbortSignal,
): Promise<ReadonlyMap<string, ArtResolution>> {
  const sprites: SpriteFrame[] = [];
  const textures: { path: string; frame: number; frames: number }[] = [];
  const spriteHandles: ArtHandle[] = [];
  const portraits = new Map<number, { references: string[]; handles: ArtHandle[] }>();
  const technologies = new Map<string, Set<number>>();
  const technologyHandles: ArtHandle[] = [];
  for (const handle of handles) {
    if (handle.kind === 'sprite') {
      sprites.push({ key: handle.key, frame: handle.frame ?? 0, ...(handle.recorded !== undefined ? { recorded: handle.recorded } : {}), ...(handle.text ? { text: true } : {}) });
      spriteHandles.push(handle);
    } else if (handle.kind === 'path') {
      textures.push({ path: handle.path, frame: handle.frame ?? 0, frames: handle.frames ?? 1 });
      spriteHandles.push(handle);
    } else if (handle.kind === 'portrait') {
      const held = portraits.get(handle.hour);
      if (held) {
        held.references.push(handle.leader);
        held.handles.push(handle);
      } else portraits.set(handle.hour, { references: [handle.leader], handles: [handle] });
    } else if (handle.kind === 'technology') {
      const indices = technologies.get(handle.token) ?? new Set<number>();
      if (handle.databaseIndex !== null && handle.databaseIndex > 0) indices.add(handle.databaseIndex);
      technologies.set(handle.token, indices);
      technologyHandles.push(handle);
    }
  }
  const calls: ArtCall[] = [];
  if (sprites.length || textures.length) {
    calls.push({ handles: spriteHandles, run: () => load(
      productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'sprite-artwork', params: { sprites, ...(textures.length ? { textures } : {}) } }),
      signal,
    ).then(reply => new Map([...(sprites.length ? spriteResolutions(reply, sprites) : []), ...textureResolutions(reply, handles)])) });
  }
  for (const [hour, { references, handles: asked }] of portraits) {
    calls.push({ handles: asked, run: () => load(
      productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'unit-leader-portraits', params: { branchId: track.branch.id, hour: hour as GatedHour, references, edge: PORTRAIT_EDGE } }),
      signal,
    ).then(reply => portraitResolutions(reply as LeaderPortraitsReply, hour)) });
  }
  if (technologies.size) {
    const tokens = [...technologies.keys()].sort();
    const recorded = Object.fromEntries(tokens.filter(token => technologies.get(token)!.size).map(token => [token, [...technologies.get(token)!].sort((a, b) => a - b)]));
    calls.push({ handles: technologyHandles, run: () => load(
      productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'technology-artwork', params: Object.keys(recorded).length ? { tokens, recorded } : { tokens } }),
      signal,
    ).then(reply => technologyResolutions(reply as TechnologyArtworkReply, handles)) });
  }
  return settleCalls(calls, signal);
}

export function artCacheScope(track: Pick<BoundTrack, 'source' | 'branch'>, contentRevision: number): string {
  return JSON.stringify([track.source.id, track.source.revision, track.branch.id, contentRevision]);
}

export const SPRITE_BATCH = 64;
const MAX_SPRITE_CONCURRENCY = 2;

type ArtFetcher = (handles: readonly ArtHandle[], signal: AbortSignal) => Promise<ReadonlyMap<string, ArtResolution>>;

export interface ArtFailure {
  readonly reason: string;
  readonly retryable: boolean;
  readonly count: number;
  readonly at: number;
}

type ArtWatcher = (resolutions: ReadonlyMap<string, ArtResolution>, failures: ReadonlyMap<string, ArtFailure>) => void;

export interface ArtClock {
  now(): number;
  after(ms: number, run: () => void): () => void;
}

const ART_CLOCK: ArtClock = {
  now: () => (typeof performance !== 'undefined' ? performance.now() : Date.now()),
  after: (ms, run) => busyWait(run, ms),
};

interface ArtScope {
  users: number;
  resolutions: Map<string, ArtResolution>;
  failures: Map<string, ArtFailure>;
  failedHandles: Map<string, ArtHandle>;
  inFlight: Set<string>;
  queued: Map<string, ArtHandle>;
  watchers: Set<ArtWatcher>;
  controllers: Set<AbortController>;
  scheduled: boolean;
  running: number;
  fetch: ArtFetcher | null;
  retryTimer: (() => void) | null;
  version: number;
}

export class SpriteCaches {
  private scopes = new Map<string, ArtScope>();

  private readonly clock: ArtClock;

  constructor(clock: ArtClock = ART_CLOCK) {
    this.clock = clock;
  }

  private held(scope: string): ArtScope {
    let held = this.scopes.get(scope);
    if (!held) {
      held = { users: 0, resolutions: new Map(), failures: new Map(), failedHandles: new Map(), inFlight: new Set(), queued: new Map(), watchers: new Set(),
        controllers: new Set(), scheduled: false, running: 0, fetch: null, retryTimer: null, version: 0 };
      this.scopes.set(scope, held);
    }
    return held;
  }

  resolutions(scope: string): Map<string, ArtResolution> { return this.held(scope).resolutions; }

  failures(scope: string): Map<string, ArtFailure> { return this.held(scope).failures; }

  version(scope: string): number { return this.held(scope).version; }

  watch(scope: string, watcher: ArtWatcher): () => void {
    const held = this.held(scope);
    held.watchers.add(watcher);
    return () => { held.watchers.delete(watcher); };
  }

  request(scope: string, handles: readonly ArtHandle[], fetch: ArtFetcher): void {
    const held = this.held(scope);
    held.fetch = fetch;
    const now = this.clock.now();
    let added = false;
    for (const handle of handles) {
      const id = artHandleKey(handle);
      if (held.resolutions.has(id) || held.inFlight.has(id)) continue;
      const failed = held.failures.get(id);
      if (failed && failed.at > now) continue;
      held.inFlight.add(id);
      held.queued.set(id, handle);
      added = true;
    }
    if (added) this.schedule(scope, held);
  }

  private schedule(scope: string, held: ArtScope): void {
    if (held.scheduled) return;
    held.scheduled = true;
    queueMicrotask(() => {
      held.scheduled = false;
      if (this.scopes.get(scope) !== held || !held.fetch) return;
      this.pump(scope, held, held.fetch);
    });
  }

  private fail(held: ArtScope, id: string, handle: ArtHandle, reason: string, retryable: boolean): void {
    const count = (held.failures.get(id)?.count ?? 0) + 1;
    held.failures.set(id, { reason, retryable, count, at: retryable ? this.clock.now() + retryDelay(count) : Infinity });
    if (retryable) held.failedHandles.set(id, handle);
    else held.failedHandles.delete(id);
  }

  private armRetry(scope: string, held: ArtScope): void {
    if (held.retryTimer) return;
    let soonest = Infinity;
    for (const id of held.failedHandles.keys()) soonest = Math.min(soonest, held.failures.get(id)?.at ?? Infinity);
    if (soonest === Infinity) return;
    held.retryTimer = this.clock.after(Math.max(0, soonest - this.clock.now()), () => {
      held.retryTimer = null;
      if (this.scopes.get(scope) !== held) return;
      const now = this.clock.now();
      for (const [id, handle] of [...held.failedHandles]) {
        if ((held.failures.get(id)?.at ?? Infinity) > now || held.inFlight.has(id)) continue;
        held.failedHandles.delete(id);
        held.inFlight.add(id);
        held.queued.set(id, handle);
      }
      this.schedule(scope, held);
      this.armRetry(scope, held);
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
        for (const [id, handle] of entries) {
          held.inFlight.delete(id);
          const resolution = parsed.get(id);
          if (!resolution) this.fail(held, id, handle, 'The artwork reply left this handle out.', true);
          else if (resolution.status === 'failed') this.fail(held, id, handle, resolution.reason, resolution.retryable);
          else {
            held.failures.delete(id);
            held.failedHandles.delete(id);
            held.resolutions.set(id, resolution);
          }
        }
        this.armRetry(scope, held);
        held.version++;
        for (const watcher of [...held.watchers]) watcher(held.resolutions, held.failures);
      }).catch(error => {
        if (this.scopes.get(scope) !== held) return;
        if (controller.signal.aborted) {
          for (const [id] of entries) held.inFlight.delete(id);
          return;
        }
        const reason = asFailure(error).message;
        const retryable = transientError(error);
        for (const [id, handle] of entries) {
          held.inFlight.delete(id);
          if (!held.resolutions.has(id)) this.fail(held, id, handle, reason, retryable);
        }
        this.armRetry(scope, held);
        held.version++;
        for (const watcher of [...held.watchers]) watcher(held.resolutions, held.failures);
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
        held.failures.clear();
        held.failedHandles.clear();
        held.retryTimer?.();
        held.retryTimer = null;
        this.scopes.delete(scope);
      }
    };
  }
}
