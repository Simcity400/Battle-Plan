import { drawableIcon } from './artwork.ts';
import { asFailure } from './queryError.ts';

export interface SpriteFrame { key: string; frame: number; recorded?: string; text?: boolean }

export const spriteId = (sprite: SpriteFrame) => JSON.stringify([sprite.frame, sprite.key, sprite.recorded ?? null, sprite.text === true]);

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

export function spriteIcons(reply: unknown, asked: readonly SpriteFrame[]): Map<string, string | null> {
  const value = reply as { items?: unknown; warnings?: unknown };
  const malformed = () => new Error('The sprite artwork reply is malformed.');
  if (!value || typeof value !== 'object' || !Array.isArray(value.items)
    || !Array.isArray(value.warnings) || !value.warnings.every(note => typeof note === 'string')) throw malformed();
  const ids = new Set(asked.map(spriteId));
  const icons = new Map<string, string | null>();
  for (const item of value.items as unknown[]) {
    const image = item as { key?: unknown; frame?: unknown; recorded?: unknown; text?: unknown; icon?: unknown };
    if (!image || typeof image.key !== 'string' || typeof image.frame !== 'number' || !optionalKey(image.recorded) || !('icon' in image)
      || !(image.text === undefined || image.text === true)) throw malformed();
    const id = spriteId({ key: image.key, frame: image.frame, recorded: image.recorded as string | undefined, text: image.text === true });
    if (!ids.has(id) || icons.has(id)) throw malformed();
    icons.set(id, drawableIcon(image.icon));
  }
  return icons;
}

export const SPRITE_BATCH = 64;
export const MAX_SPRITE_CONCURRENCY = 2;

export type SpriteFetcher = (sprites: readonly SpriteFrame[], signal: AbortSignal) => Promise<unknown>;

interface SpriteScope {
  users: number;
  icons: Map<string, string | null>;
  errors: Map<string, string>;
  inFlight: Set<string>;
  queued: Map<string, SpriteFrame>;
  watchers: Set<(icons: ReadonlyMap<string, string | null>, errors: ReadonlyMap<string, string>) => void>;
  controllers: Set<AbortController>;
  scheduled: boolean;
  running: number;
}

export class SpriteCaches {
  private scopes = new Map<string, SpriteScope>();

  private held(scope: string): SpriteScope {
    let held = this.scopes.get(scope);
    if (!held) {
      held = { users: 0, icons: new Map(), errors: new Map(), inFlight: new Set(), queued: new Map(), watchers: new Set(), controllers: new Set(), scheduled: false, running: 0 };
      this.scopes.set(scope, held);
    }
    return held;
  }

  icons(scope: string): Map<string, string | null> { return this.held(scope).icons; }

  errors(scope: string): Map<string, string> { return this.held(scope).errors; }

  watch(scope: string, watcher: (icons: ReadonlyMap<string, string | null>, errors: ReadonlyMap<string, string>) => void): () => void {
    const held = this.held(scope);
    held.watchers.add(watcher);
    return () => { held.watchers.delete(watcher); };
  }

  request(scope: string, sprites: readonly SpriteFrame[], fetch: SpriteFetcher): void {
    const held = this.held(scope);
    let added = false;
    for (const sprite of sprites) {
      const id = spriteId(sprite);
      if (held.icons.has(id) || held.errors.has(id) || held.inFlight.has(id)) continue;
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
      const ids = entries.map(([id]) => id);
      const controller = new AbortController();
      held.controllers.add(controller);
      held.running++;
      void fetch(batch, controller.signal).then(reply => {
        if (controller.signal.aborted || this.scopes.get(scope) !== held) return;
        const resolved = spriteIcons(reply, batch);
        for (const id of ids) {
          held.inFlight.delete(id);
          held.errors.delete(id);
          held.icons.set(id, resolved.get(id) ?? null);
        }
        for (const watcher of [...held.watchers]) watcher(held.icons, held.errors);
      }).catch(error => {
        if (this.scopes.get(scope) !== held) return;
        const reason = controller.signal.aborted ? null : asFailure(error).message;
        for (const id of ids) {
          held.inFlight.delete(id);
          if (reason !== null && !held.icons.has(id)) held.errors.set(id, reason);
        }
        if (reason !== null) {
          for (const watcher of [...held.watchers]) watcher(held.icons, held.errors);
        }
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
