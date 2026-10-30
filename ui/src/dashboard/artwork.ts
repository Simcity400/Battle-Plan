import type { Artwork } from '../generated/dashboard';
import { artworkUrlValue } from './payload.ts';

/** An icon a widget can draw: the artwork URL of a product reply's image. Any
 * other value is missing art, which the widget shows with its placeholder, so one
 * malformed icon never refuses the whole reply. */
export function drawableIcon(icon: unknown): string | null {
  return artworkUrlValue(icon) ? icon : null;
}

/** Whether a reply's artwork entry has the product's shape (token, name,
 * description, duration). The icon is not judged here: a well-formed entry whose
 * icon cannot be drawn keeps its text and shows the placeholder
 * ([`keepDrawableIcons`]); a malformed entry refuses the reply. */
export function artworkEntry(value: unknown): value is Artwork {
  if (typeof value !== 'object' || value === null) return false;
  const art = value as Record<string, unknown>;
  const text = (field: unknown) => field === null || typeof field === 'string';
  return typeof art.token === 'string' && art.token.length > 0 && text(art.name) && text(art.description)
    && (art.baseDurationDays === null || (typeof art.baseDurationDays === 'number' && Number.isFinite(art.baseDurationDays) && art.baseDurationDays >= 0))
    && 'icon' in art;
}

/** Sets every icon of already validated artwork that the widget cannot draw to
 * missing art, in place, so the reply keeps its identity. */
export function keepDrawableIcons(artwork: Artwork[]): void {
  for (const art of artwork) art.icon = drawableIcon(art.icon);
}
