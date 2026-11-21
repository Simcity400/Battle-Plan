import type { Artwork } from '../generated/dashboard';
import { artworkUrlValue } from './payload.ts';

export function drawableIcon(icon: unknown): string | null {
  return artworkUrlValue(icon) ? icon : null;
}

export function artworkEntry(value: unknown): value is Artwork {
  if (typeof value !== 'object' || value === null) return false;
  const art = value as Record<string, unknown>;
  const text = (field: unknown) => field === null || typeof field === 'string';
  return typeof art.token === 'string' && art.token.length > 0 && text(art.name) && text(art.description)
    && (art.baseDurationDays === null || (typeof art.baseDurationDays === 'number' && Number.isFinite(art.baseDurationDays) && art.baseDurationDays >= 0))
    && 'icon' in art;
}

export function keepDrawableIcons(artwork: Artwork[]): void {
  for (const art of artwork) art.icon = drawableIcon(art.icon);
}
