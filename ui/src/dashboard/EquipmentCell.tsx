// Equipment art and in-game name as the stockpile table shows them, for the market
// and lend-lease tables: art only for mounted rows, the hammer where none loads.
import { useMemo } from 'react';
import ArtFallback from '../components/ArtFallback';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack } from './model';
import { variantDecimal } from './economyHistory';
import { useStockpileArtwork } from '../widgets/equipment-stockpile/useStockpileArtwork';
import '../widgets/equipment-stockpile/equipmentStockpile.css';

export const EQUIPMENT_UNNAMED = 'Equipment name unavailable';

export function EquipmentCell({ name, detail, icon }: { name: string | null; detail?: string | null; icon?: string | null }) {
  return <span className="stockpile-equipment">
    {icon ? <img className="stockpile-art" src={icon} alt="" /> : <span className="stockpile-art"><ArtFallback icon="hammer" /></span>}
    <span><b>{name ?? EQUIPMENT_UNNAMED}</b>{detail && <small>{detail}</small>}</span>
  </span>;
}

/** The pictures of the variants (hexadecimal handles) the mounted rows show, read for
 * `tag` at `hour` through the stockpile's artwork product. */
export function useVariantArt(runtime: WidgetRuntime, track: BoundTrack | undefined, tag: string | null, hour: number | null,
  variants: readonly string[], active: boolean): ReadonlyMap<string, string> {
  const decimal = useMemo(() => [...new Set(variants)].sort().map(variantDecimal), [variants]);
  const artwork = useStockpileArtwork(runtime, track, tag ?? '', tag && decimal.length ? hour : null, decimal, active);
  return useMemo(() => new Map((artwork?.data?.entries ?? []).flatMap(entry => entry.icon && entry.key.startsWith('variant:')
    ? [[BigInt(entry.variant).toString(16), entry.icon] as const] : [])), [artwork]);
}
