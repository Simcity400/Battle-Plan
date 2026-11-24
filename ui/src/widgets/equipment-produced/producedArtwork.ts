import type { GatedHour } from '../../dashboard/regionTime';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useMemo } from 'react';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import { validateStockpileArtwork } from '../equipment-stockpile/stockpileArtwork.ts';
import type { StockpileArtworkReply } from '../../generated/dashboard';
import { productRequest } from '../../dashboard/request.ts';

export interface ProducedPicture { icon: string | null; name: string | null }

const NO_PICTURES: ReadonlyMap<string, ProducedPicture> = new Map();

export function useProducedArtwork(runtime: WidgetRuntime, track: BoundTrack, tag: string | null, hour: GatedHour | null,
  active: boolean): ReadonlyMap<string, ProducedPicture> {
  const art = useProduct<StockpileArtworkReply>(runtime.client, tag && hour !== null ? productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'stockpile-artwork', params: { branchId: track.branch.id, country: countryFromKey(tag), variants: [], hour } }) : null, active, validateStockpileArtwork, track.source);
  return useMemo(() => art.data
    ? new Map(art.data.entries.filter(entry => entry.key.startsWith('type:')).map(entry => [entry.key, { icon: entry.icon, name: entry.name }]))
    : NO_PICTURES, [art.data]);
}
