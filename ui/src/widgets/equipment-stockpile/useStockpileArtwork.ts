import { countryFromKey } from '../../dashboard/countryKey.ts';
import { drawableIcon } from '../../dashboard/artwork';
import { useHeadProduct } from '../../dashboard/headProduct.ts';
import { useGate } from '../../dashboard/regionGate';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import type { StockpileArtworkReply } from '../../generated/dashboard';
import { SPEC, type RegionAction } from '../../dashboard/regionTime.ts';

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

export function useStockpileArtwork(runtime: WidgetRuntime, widget: WidgetSpec, tag: string | null, variants: readonly string[], active: boolean,
  time: RegionAction = SPEC) {
  return useHeadProduct(runtime, useGate(runtime, widget, time), 'stockpile-artwork', tag ? { country: countryFromKey(tag), variants: [...variants] } : null,
    { active, validate: validateStockpileArtwork });
}
