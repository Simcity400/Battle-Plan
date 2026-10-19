import { useRef } from 'react';
import { dayLabel } from '../../charts/draw';
import WidgetState from '../../components/WidgetState';
import { VirtualList } from '../../components/VirtualList';
import type { MarketItem, MarketLevel, MarketGrant, MarketLendLeaseReply } from '../../generated/dashboard';
import { priceLevel, rawAmount } from './marketData';
const actor = (tag: string | null) => tag ?? 'Country name unavailable';
function Items({ title, items, share = false }: { title: string; items: MarketItem[]; share?: boolean }) {
  return <div className="market-items"><h4>{title}</h4>{items.length ? <dl>{items.map((item, index) => <div key={index}><dt title={item.variant}>{item.name ?? 'Equipment name unavailable'}</dt><dd title={`Recorded amount: ${item.amountRaw}`}>{rawAmount(item.amountRaw, share ? 1000n : 100000n)}{share ? '%' : ''}</dd></div>)}</dl> : <p>None</p>}</div>;
}
function Levels({ levels }: { levels: MarketLevel[] }) { return levels.length ? <div className="market-items"><h4>Asking price levels</h4><dl>{levels.map((level, index) => <div key={index}><dt title={level.variant}>{level.name ?? 'Equipment name unavailable'}</dt><dd>{priceLevel(level.level)}</dd></div>)}</dl></div> : <p>Normal asking price unless specified.</p>; }
function Grants({ grants }: { grants: MarketGrant[] }) { return grants.length ? <div className="market-items"><h4>Subsidies</h4>{grants.map((grant, index) => <p key={index}>{rawAmount(grant.cicRaw)} CIC · {grant.archetype ?? 'Archetype unavailable'} · {grant.trigger !== null ? 'Conditional eligibility' : grant.targets.length ? grant.targets.join(', ') : 'No eligible countries'}</p>)}</div> : <p>No subsidies.</p>; }
export default function TradeMarketView({ data, selected, onPick }: { data: MarketLendLeaseReply; selected?: string | null; onPick?: (key: string) => void }) {
  const root = useRef<HTMLDivElement>(null);
  const offers = data.offers?.filter(row => !selected || selected === `offer:${row.country}`) ?? null;
  const contracts = data.contracts?.filter(row => !selected || selected === `contract:${row.request}:${row.id}`) ?? null;
  const agreements = data.agreements?.filter(row => !selected || selected === `agreement:${row.id}`) ?? null;
  const deliveries = data.deliveries?.filter(row => !selected || selected === `agreement:${row.agreement}` || selected === `contract:false:${row.agreement}` || offers?.some(offer => offer.tag !== null && (offer.tag === row.giver || offer.tag === row.receiver))) ?? null;

  return <div className="market-lendlease" ref={root}>
    <p className="market-caption">{dayLabel(data.hour)}</p>
    <section aria-label="International market"><h3>International market</h3>
      {offers === null ? <WidgetState kind="unavailable" message="Market inventory was not recorded." /> : <VirtualList items={offers} itemKey={row => String(row.country)} estimate={52} scrollRef={root} retainExpansion={false} label="Market offers">{row => <details open={!!selected}><summary onClick={event => { if (onPick) { event.preventDefault(); onPick(`offer:${row.country}`); } }}>{actor(row.tag)} · {row.inventory.length} listed models</summary>
        <Items title="Listed inventory" items={row.inventory} /><Levels levels={row.prices} /><Grants grants={row.subsidies} />
        <p>Subsidy balance: {rawAmount(row.subsidyTotalRaw)} CIC</p><p>Automatically accept access: {row.automation[0] ? 'Yes' : 'No'} · send access: {row.automation[1] ? 'Yes' : 'No'} · accept purchases: {row.automation[2] ? 'Yes' : 'No'}</p>
      </details>}</VirtualList>}
      <h4>Requests and contracts</h4>{contracts === null ? <p>Contract terms unavailable.</p> : !contracts.length ? <p>No requests or contracts.</p> : <VirtualList items={contracts} itemKey={row => `${row.request}:${row.id}`} estimate={52} scrollRef={root} retainExpansion={false} label="Market contract terms">{row => <details open={!!selected}><summary onClick={event => { if (onPick) { event.preventDefault(); onPick(`contract:${row.request}:${row.id}`); } }}>{actor(row.seller)} · {actor(row.buyer)} · {row.request ? 'Request' : 'Contract'}</summary><p>Contract {row.id} · factory assignment {row.speed}</p><Items title="Equipment" items={row.equipment} /><Items title="Agreed prices (CIC)" items={row.prices} /><Levels levels={row.levels} /><Grants grants={row.subsidies} /></details>}</VirtualList>}
    </section>
    <section aria-label="Lend-lease"><h3>Lend-lease</h3>
      {agreements === null ? <WidgetState kind="unavailable" message="Lend-lease terms were not recorded." /> : !agreements.length ? <p>No active agreements.</p> : <VirtualList items={agreements} itemKey={row => row.id} estimate={52} scrollRef={root} retainExpansion={false} label="Lend-lease agreements">{row => <details open={!!selected}><summary onClick={event => { if (onPick) { event.preventDefault(); onPick(`agreement:${row.id}`); } }}>{actor(row.giver)} · {actor(row.receiver)}</summary><p>Active since {dayLabel(row.activeSince)} · last delivery {dayLabel(row.lastDelivery)}</p><Items title="Monthly" items={row.monthly} /><Items title="Production share" items={row.percentage} share /><Items title="Once" items={row.once} /><Items title="Delivered" items={row.delivered} /><Items title="Lost" items={row.sunk} />
        <p>Fuel per day: {rawAmount(row.fuelDailyRaw, 32768n)} · production share: {rawAmount(row.fuelPercentageRaw, 1000n)}%</p><p>Fuel sent: {rawAmount(row.fuelSentRaw, 32768n)} · lost: {rawAmount(row.fuelSunkRaw, 32768n)} · last delivery: {rawAmount(row.lastFuelDeliveredRaw, 32768n)}</p>
      </details>}</VirtualList>}
    </section>
    <section aria-label="Deliveries"><h4>Delivery changes in this recorded hour</h4>{deliveries === null ? <p>Deliveries unavailable.</p> : !deliveries.length ? <p>No deliveries observed in this interval.</p> : <VirtualList items={deliveries} itemKey={row => row.id} estimate={44} scrollRef={root} retainExpansion={false} label="Delivery events">{row => <p title={`Agreement ${row.agreement} · occurrence ${row.occurrence}`}>{actor(row.giver)} · {actor(row.receiver)} · {row.kind === 34 ? (row.item.name ?? 'Delivery amount unavailable') : row.kind === 6 ? 'Agreement ended' : row.kind === 21 ? `${rawAmount(row.item.amountRaw)} CIC paid` : row.kind === 5 ? `${rawAmount(row.item.amountRaw, 32768n)} fuel delivered · ${rawAmount(row.lostRaw, 32768n)} lost` : `${row.item.name ?? 'Equipment name unavailable'}: ${rawAmount(row.item.amountRaw)} ${row.kind === 4 ? 'increase in lost total' : 'increase in delivered total'}`}</p>}</VirtualList>}</section>
  </div>;
}