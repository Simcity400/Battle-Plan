import { drawableIcon } from '../../dashboard/artwork.ts';
import type { StockpileArtworkReply } from '../../generated/dashboard';

const text = (value: unknown) => value === null || typeof value === 'string';

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
