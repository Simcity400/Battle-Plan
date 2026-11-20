import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useMemo } from 'react';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import { validateStockpileArtwork } from '../equipment-stockpile/useStockpileArtwork';
import type { StockpileArtworkReply } from '../../generated/dashboard';

/** A type's picture for the breakdown's table: its icon (null draws the named placeholder) and its name. */
export interface ProducedPicture { icon: string | null; name: string | null }

const NO_PICTURES: ReadonlyMap<string, ProducedPicture> = new Map();

/** Each equipment type's picture for `tag`'s breakdown at `hour`, keyed `type:<key>`, dated to that hour's read: the
 * stockpile family's artwork (`stockpile-artwork`), its country's best design as the game draws it. A type without a
 * picture is absent, and its row shows the named placeholder. */
export function useProducedArtwork(runtime: WidgetRuntime, track: BoundTrack, tag: string | null, hour: number | null,
  active: boolean): ReadonlyMap<string, ProducedPicture> {
  const art = useProduct<StockpileArtworkReply>(runtime.client, tag && hour !== null ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'stockpile-artwork', params: { branchId: track.branch.id, country: countryFromKey(tag), variants: [], hour } } : null, active, validateStockpileArtwork, track.source);
  return useMemo(() => art.data
    ? new Map(art.data.entries.filter(entry => entry.key.startsWith('type:')).map(entry => [entry.key, { icon: entry.icon, name: entry.name }]))
    : NO_PICTURES, [art.data]);
}
