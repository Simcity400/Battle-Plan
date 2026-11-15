import { drawableIcon } from '../../dashboard/artwork';
import { useHeadProduct } from '../../dashboard/headProduct.ts';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import type { StockpileArtworkReply } from '../../generated/dashboard';

const text = (value: unknown) => value === null || typeof value === 'string';

/** Checks a pictures reply (`stockpile-artwork`): each entry once, an archetype's (`type:<key>`) or a design's own
 * (`variant:<handle>`), with a drawable picture or none. */
export function validateStockpileArtwork(data: unknown): StockpileArtworkReply {
  const reply = data as StockpileArtworkReply;
  if (!reply || typeof reply.tag !== 'string' || !Array.isArray(reply.entries)
    || new Set(reply.entries.map(entry => entry?.key)).size !== reply.entries.length
    || !reply.entries.every(entry => entry != null && typeof entry.key === 'string' && /^\d{1,20}$/.test(entry.variant)
      && ((entry.key.startsWith('type:') && entry.key.length > 5) || entry.key === `variant:${entry.variant}`)
      && text(entry.name) && text(entry.reason))
    || !Array.isArray(reply.warnings) || !reply.warnings.every(note => typeof note === 'string')) throw new Error('The stockpile artwork reply is malformed.');
  for (const entry of reply.entries) entry.icon = drawableIcon(entry.icon);
  return reply;
}

/** An archetype's picture as the game's Logistics screen draws it (its country's best design), and the pictures of
 * `variants` by handle, for the hour `widget` shows: followed at the head as a pushed panel, which changes only when a
 * picture does, else asked for that hour. Other widgets show a design's art through this hook. */
export function useStockpileArtwork(runtime: WidgetRuntime, widget: WidgetSpec, tag: string | null, variants: readonly string[], active: boolean,
  hoverHour: number | null = null) {
  return useHeadProduct<StockpileArtworkReply>(runtime, widget, 'stockpile-artwork', tag ? { tag, variants: [...variants] } : null,
    { active, validate: validateStockpileArtwork, hoverHour });
}
