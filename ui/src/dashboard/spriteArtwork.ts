import type { ArtResolution, SpriteImage } from '../generated/dashboard';
import { spriteImageToArt } from '../families/art/adapters.ts';
import { artHandleKey } from '../families/art/types.ts';
import { drawableIcon } from './artwork.ts';
import { asFailure } from './queryError.ts';

export interface SpriteFrame { key: string; frame: number; recorded?: string; text?: boolean }

export const spriteId = (sprite: SpriteFrame) => artHandleKey({ kind: 'sprite', ...sprite });

export const optionalKey = (value: unknown) => value === undefined || typeof value === 'string' && value.length > 0;

export function spriteFrames(sprites: Iterable<SpriteFrame | null | undefined>): SpriteFrame[] {
  const shown = new Map<string, SpriteFrame>();
  for (const sprite of sprites)
    if (sprite && typeof sprite.key === 'string' && sprite.key.length > 0 && Number.isSafeInteger(sprite.frame) && sprite.frame >= 0
      && optionalKey(sprite.recorded) && (sprite.text === undefined || typeof sprite.text === 'boolean'))
      shown.set(spriteId(sprite), { key: sprite.key, frame: sprite.frame, ...sprite.recorded === undefined ? {} : { recorded: sprite.recorded },
        ...sprite.text ? { text: true } : {} });
  return [...shown.entries()].sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0).map(([, sprite]) => sprite);
}

function parseSpriteItems(reply: unknown, asked: readonly SpriteFrame[]): Map<string, { readonly icon: string | null; readonly resolution: ArtResolution }> {
  const value = reply as { items?: unknown; warnings?: unknown };
  const malformed = () => new Error('The sprite artwork reply is malformed.');
  if (!value || typeof value !== 'object' || !Array.isArray(value.items)
    || !Array.isArray(value.warnings) || !value.warnings.every(note => typeof note === 'string')) throw malformed();
  const ids = new Set(asked.map(spriteId));
  const out = new Map<string, { readonly icon: string | null; readonly resolution: ArtResolution }>();
  for (const item of value.items as unknown[]) {
    const image = item as { key?: unknown; frame?: unknown; recorded?: unknown; text?: unknown; icon?: unknown; resolution?: unknown };
    if (!image || typeof image.key !== 'string' || typeof image.frame !== 'number' || !optionalKey(image.recorded) || !('icon' in image)
      || !(image.text === undefined || image.text === true)) throw malformed();
    const id = spriteId({ key: image.key, frame: image.frame, recorded: image.recorded as string | undefined, text: image.text === true });
    if (!ids.has(id) || out.has(id)) throw malformed();
    const icon = drawableIcon(image.icon);
    const resolution = spriteImageToArt({
      key: image.key,
      frame: image.frame,
      ...(image.recorded !== undefined ? { recorded: image.recorded as string } : {}),
      ...(image.text === true ? { text: true } : {}),
      icon,
      ...(image.resolution !== undefined ? { resolution: image.resolution as ArtResolution } : {}),
    } as SpriteImage);
    out.set(id, { icon, resolution });
  }
  return out;
}

export function spriteIcons(reply: unknown, asked: readonly SpriteFrame[]): Map<string, string | null> {
  const parsed = parseSpriteItems(reply, asked);
  const icons = new Map<string, string | null>();
  for (const [id, entry] of parsed) icons.set(id, entry.icon);
  return icons;
}

export function spriteResolutions(reply: unknown, asked: readonly SpriteFrame[]): Map<string, ArtResolution> {
  const parsed = parseSpriteItems(reply, asked);
  const resolutions = new Map<string, ArtResolution>();
  for (const [id, entry] of parsed) resolutions.set(id, entry.resolution);
  return resolutions;
}

export const SPRITE_BATCH = 64;
export const MAX_SPRITE_CONCURRENCY = 2;

export type SpriteFetcher = (sprites: readonly SpriteFrame[], signal: AbortSignal) => Promise<unknown>;

type SpriteWatcher = (icons: ReadonlyMap<string, string | null>, errors: ReadonlyMap<string, string>) => void;

interface SpriteScope {
  users: number;
  icons: Map<string, string | null>;
  resolutions: Map<string, ArtResolution>;
  errors: Map<string, string>;
  inFlight: Set<string>;
  queued: Map<string, SpriteFrame>;
  watchers: Set<SpriteWatcher>;
  controllers: Set<AbortController>;
  scheduled: boolean;
  running: number;
}

export class SpriteCaches {
  private scopes = new Map<string, SpriteScope>();

  private held(scope: string): SpriteScope {
    let held = this.scopes.get(scope);
    if (!held) {
      held = { users: 0, icons: new Map(), resolutions: new Map(), errors: new Map(), inFlight: new Set(), queued: new Map(), watchers: new Set(),
        controllers: new Set(), scheduled: false, running: 0 };
      this.scopes.set(scope, held);
    }
    return held;
  }

  icons(scope: string): Map<string, string | null> { return this.held(scope).icons; }

  resolutions(scope: string): Map<string, ArtResolution> { return this.held(scope).resolutions; }

  errors(scope: string): Map<string, string> { return this.held(scope).errors; }

  watch(scope: string, watcher: SpriteWatcher): () => void {
    const held = this.held(scope);
    held.watchers.add(watcher);
    return () => { held.watchers.delete(watcher); };
  }

  request(scope: string, sprites: readonly SpriteFrame[], fetch: SpriteFetcher): void {
    const held = this.held(scope);
    let added = false;
    for (const sprite of sprites) {
      const id = spriteId(sprite);
      if (held.resolutions.has(id) || held.errors.has(id) || held.inFlight.has(id)) continue;
      held.inFlight.add(id);
      held.queued.set(id, sprite);
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

  private pump(scope: string, held: SpriteScope, fetch: SpriteFetcher): void {
    while (held.running < MAX_SPRITE_CONCURRENCY && held.queued.size > 0) {
      const entries = [...held.queued.entries()].slice(0, SPRITE_BATCH);
      for (const [id] of entries) held.queued.delete(id);
      const batch = entries.map(([, sprite]) => sprite);
      const controller = new AbortController();
      held.controllers.add(controller);
      held.running++;
      void fetch(batch, controller.signal).then(reply => {
        if (controller.signal.aborted || this.scopes.get(scope) !== held) return;
        const parsed = parseSpriteItems(reply, batch);
        for (const [id] of entries) {
          held.inFlight.delete(id);
          const entry = parsed.get(id);
          if (!entry) {
            held.errors.set(id, 'The sprite artwork reply left this sprite out.');
            continue;
          }
          held.errors.delete(id);
          held.icons.set(id, entry.icon);
          held.resolutions.set(id, entry.resolution);
        }
        for (const watcher of [...held.watchers]) watcher(held.icons, held.errors);
      }).catch(error => {
        if (this.scopes.get(scope) !== held) return;
        const reason = controller.signal.aborted ? null : asFailure(error).message;
        for (const [id] of entries) {
          held.inFlight.delete(id);
          if (reason !== null && !held.resolutions.has(id)) held.errors.set(id, reason);
        }
        if (reason !== null) for (const watcher of [...held.watchers]) watcher(held.icons, held.errors);
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
