import { drawableIcon } from './artwork.ts';

export interface SpriteFrame { key: string; frame: number; recorded?: string }

export const spriteId = (sprite: SpriteFrame) => JSON.stringify([sprite.frame, sprite.key, sprite.recorded ?? null]);

export const optionalKey = (value: unknown) => value === undefined || typeof value === 'string' && value.length > 0;

export function spriteFrames(sprites: Iterable<SpriteFrame | null | undefined>): SpriteFrame[] {
  const shown = new Map<string, SpriteFrame>();
  for (const sprite of sprites)
    if (sprite && typeof sprite.key === 'string' && sprite.key.length > 0 && Number.isSafeInteger(sprite.frame) && sprite.frame >= 0
      && optionalKey(sprite.recorded))
      shown.set(spriteId(sprite), sprite.recorded === undefined ? { key: sprite.key, frame: sprite.frame }
        : { key: sprite.key, frame: sprite.frame, recorded: sprite.recorded });
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
    const image = item as { key?: unknown; frame?: unknown; recorded?: unknown; icon?: unknown };
    if (!image || typeof image.key !== 'string' || typeof image.frame !== 'number' || !optionalKey(image.recorded) || !('icon' in image))
      throw malformed();
    const id = spriteId({ key: image.key, frame: image.frame, recorded: image.recorded as string | undefined });
    if (!ids.has(id) || icons.has(id)) throw malformed();
    icons.set(id, drawableIcon(image.icon));
  }
  return icons;
}
