import { drawableIcon } from './artwork.ts';

/** One frame of an authored sprite, as a product returned it; frame is zero-based. */
export interface SpriteFrame { key: string; frame: number }

/** The lookup id of one sprite frame. */
export const spriteId = (sprite: SpriteFrame) => `${sprite.frame}:${sprite.key}`;

/** The distinct sprite frames a widget draws, in a stable order, so one request serves them all. */
export function spriteFrames(sprites: Iterable<SpriteFrame | null | undefined>): SpriteFrame[] {
  const shown = new Map<string, SpriteFrame>();
  for (const sprite of sprites)
    if (sprite && typeof sprite.key === 'string' && sprite.key.length > 0 && Number.isSafeInteger(sprite.frame) && sprite.frame >= 0)
      shown.set(spriteId(sprite), { key: sprite.key, frame: sprite.frame });
  return [...shown.entries()].sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0).map(([, sprite]) => sprite);
}

/** Each asked frame's drawable image from a `sprite-artwork` reply, by `spriteId`: null where
 * the art is refused or cannot be drawn. A reply answering frames it was not asked for is malformed. */
export function spriteIcons(reply: unknown, asked: readonly SpriteFrame[]): Map<string, string | null> {
  const value = reply as { items?: unknown; warnings?: unknown };
  const malformed = () => new Error('The sprite artwork reply is malformed.');
  if (!value || typeof value !== 'object' || !Array.isArray(value.items)
    || !Array.isArray(value.warnings) || !value.warnings.every(note => typeof note === 'string')) throw malformed();
  const ids = new Set(asked.map(spriteId));
  const icons = new Map<string, string | null>();
  for (const item of value.items as unknown[]) {
    const image = item as { key?: unknown; frame?: unknown; icon?: unknown };
    if (!image || typeof image.key !== 'string' || typeof image.frame !== 'number' || !('icon' in image)) throw malformed();
    const id = spriteId({ key: image.key, frame: image.frame });
    if (!ids.has(id) || icons.has(id)) throw malformed();
    icons.set(id, drawableIcon(image.icon));
  }
  return icons;
}
