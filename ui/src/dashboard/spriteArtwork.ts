import { drawableIcon } from './artwork.ts';

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

export class SpriteCaches {
  private scopes = new Map<string, { users: number; icons: Map<string, string | null> }>();

  private held(scope: string) {
    let held = this.scopes.get(scope);
    if (!held) { held = { users: 0, icons: new Map() }; this.scopes.set(scope, held); }
    return held;
  }

  icons(scope: string): Map<string, string | null> { return this.held(scope).icons; }

  hold(scope: string): () => void {
    const held = this.held(scope);
    held.users++;
    return () => { if (--held.users === 0 && this.scopes.get(scope) === held) this.scopes.delete(scope); };
  }
}
