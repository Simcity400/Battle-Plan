import ArtFallback from '../components/ArtFallback';
import './market.css';

export default function MarketEquipment({ name, icon, missing, detail }: { name: string | null; icon: string | null; missing?: string | null; detail?: string | null }) {
  return <span className="market-equipment">{icon ? <img src={icon} alt="" /> : <ArtFallback icon="hammer" name={name ?? 'Equipment'} reason={missing} />}
    <span className="market-equipment-name" title={name === null ? missing ?? undefined : undefined}>{name ?? 'Equipment name unavailable'}</span>
    {detail && <small>{detail}</small>}</span>;
}