import type { ArtResolution, SpriteImage } from '../generated/dashboard';
import { spriteImageToArt } from '../families/art/adapters.ts';
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

export class SpriteCaches {
  private scopes = new Map<string, { users: number; icons: Map<string, string | null>; resolutions: Map<string, ArtResolution> }>();

  private held(scope: string) {
    let held = this.scopes.get(scope);
    if (!held) { held = { users: 0, icons: new Map(), resolutions: new Map() }; this.scopes.set(scope, held); }
    return held;
  }

  icons(scope: string): Map<string, string | null> { return this.held(scope).icons; }

  resolutions(scope: string): Map<string, ArtResolution> { return this.held(scope).resolutions; }

  hold(scope: string): () => void {
    const held = this.held(scope);
    held.users++;
    return () => { if (--held.users === 0 && this.scopes.get(scope) === held) this.scopes.delete(scope); };
  }
}
