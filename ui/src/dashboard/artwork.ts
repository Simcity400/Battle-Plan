import type { Artwork } from '../generated/dashboard';

/** Longest icon data URL a widget draws; the products scale content art well below it. */
export const MAX_ICON_URL = 128 * 1024;

/** An icon a widget can draw: a bounded PNG data URL from a product reply. Any
 * other value is missing art, which the widget shows with its placeholder, so one
 * oversized or malformed mod icon never refuses the whole reply. */
export function drawableIcon(icon: unknown): string | null {
  return typeof icon === 'string' && icon.length <= MAX_ICON_URL && /^data:image\/png;base64,[A-Za-z0-9+/]+=*$/.test(icon) ? icon : null;
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
