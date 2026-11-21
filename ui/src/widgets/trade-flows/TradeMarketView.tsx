import { useRef } from 'react';
import type { ReactNode } from 'react';
import { dayLabel } from '../../charts/draw';
import { liveKey } from '../../charts/live';
import type { LiveValues } from '../../charts/live';
import WidgetState from '../../components/WidgetState';
import { VirtualList } from '../../components/VirtualList';
import type { MarketAt, MarketItem, MarketLevel, MarketGrant } from './types';
import { priceLevel, rawAmount } from './marketData';
const actor = (tag: string | null) => tag ?? 'Country name unavailable';
function Items({ title, items, share = false }: { title: string; items: MarketItem[]; share?: boolean }) {
  return <div className="market-items"><h4>{title}</h4>{items.length ? <dl>{items.map((item, index) => <div key={index}><dt title={item.variant}>{item.name ?? 'Equipment name unavailable'}</dt><dd title={`Recorded amount: ${item.amountRaw}`}>{rawAmount(item.amountRaw, share ? 1000n : 100000n)}{share ? '%' : ''}</dd></div>)}</dl> : <p>None</p>}</div>;
}
function Levels({ levels }: { levels: MarketLevel[] }) { return levels.length ? <div className="market-items"><h4>Asking price levels</h4><dl>{levels.map((level, index) => <div key={index}><dt title={level.variant}>{level.name ?? 'Equipment name unavailable'}</dt><dd>{priceLevel(level.level)}</dd></div>)}</dl></div> : <p>Normal asking price unless specified.</p>; }
function Grants({ grants }: { grants: MarketGrant[] }) { return grants.length ? <div className="market-items"><h4>Subsidies</h4>{grants.map((grant, index) => <p key={index}>{rawAmount(grant.cicRaw)} CIC · {grant.archetype ?? 'Archetype unavailable'} · {grant.trigger !== null ? 'Conditional eligibility' : grant.targets.length ? grant.targets.join(', ') : 'No eligible countries'}</p>)}</div> : <p>No subsidies.</p>; }
export default function TradeMarketView({ data, selected, onPick, values, deliveries }: { data: MarketAt; selected?: string | null; onPick?: (key: string) => void;
  values?: LiveValues<number>; deliveries?: ReactNode }) {
  const root = useRef<HTMLDivElement>(null);
  const offers = data.offers?.filter(row => !selected || selected === `offer:${row.country}`) ?? null;
  const contracts = data.contracts?.filter(row => !selected || selected === `contract:${row.request}:${row.id}`) ?? null;
  const agreements = data.agreements?.filter(row => !selected || selected === `agreement:${row.id}`) ?? null;
  const cell = (row: string, column: string) => values?.get(liveKey(row, column));
  const count = (row: string, column: string) => cell(row, column)?.toLocaleString() ?? '—';
  const fuel = (row: string, column: string) => { const value = cell(row, column); return value === undefined ? '—' : rawAmount(String(Math.trunc(value)), 32768n); };

  return <div className="market-lendlease" ref={root}>
    <p className="market-caption">{dayLabel(data.hour)}</p>
    <section aria-label="International market"><h3>International market</h3>
      {offers === null ? <WidgetState kind="unavailable" message="Market inventory was not recorded." /> : <VirtualList items={offers} itemKey={row => String(row.country)} estimate={52} scrollRef={root} retainExpansion={false} label="Market offers">{row => <details open={!!selected}><summary onClick={event => { if (onPick) { event.preventDefault(); onPick(`offer:${row.country}`); } }}>{actor(row.tag)} · {row.inventory.length} listed models</summary>
        <Items title="Listed inventory" items={row.inventory} /><Levels levels={row.prices} /><Grants grants={row.subsidies} />
        <p>Subsidy balance: {rawAmount(row.subsidyTotalRaw)} CIC</p><p>Automatically send market access: {row.autoSendMarketAccess ? 'Yes' : 'No'}</p>
      </details>}</VirtualList>}
      <h4>Requests and contracts</h4>{contracts === null ? <p>Contract terms unavailable.</p> : !contracts.length ? <p>No requests or contracts.</p> : <VirtualList items={contracts} itemKey={row => `${row.request}:${row.id}`} estimate={52} scrollRef={root} retainExpansion={false} label="Market contract terms">{row => <details open={!!selected}><summary onClick={event => { if (onPick) { event.preventDefault(); onPick(`contract:${row.request}:${row.id}`); } }}>{actor(row.seller)} · {actor(row.buyer)} · {row.request ? 'Request' : 'Contract'}</summary><p>Contract {row.id} · factory assignment {row.speed}</p>
        {row.purchase ? <p>Convoys {count(row.key, 'assigned')} of {count(row.key, 'convoys')} · collected {cell(row.key, 'collected') === undefined ? '—' : `${rawAmount(String(Math.trunc(cell(row.key, 'collected')!)))} CIC`} · deliveries {count(row.key, 'deliveries')}</p>
          : !row.request && <p>{data.purchasesMissing ?? 'No purchase progress recorded for this contract.'}</p>}
        <Items title="Equipment" items={row.equipment} /><Items title="Agreed prices (CIC)" items={row.prices} /><Levels levels={row.levels} /><Grants grants={row.subsidies} /></details>}</VirtualList>}
    </section>
    <section aria-label="Lend-lease"><h3>Lend-lease</h3>
      {agreements === null ? <WidgetState kind="unavailable" message="Lend-lease terms were not recorded." /> : !agreements.length ? <p>No active agreements.</p> : <VirtualList items={agreements} itemKey={row => row.id} estimate={52} scrollRef={root} retainExpansion={false} label="Lend-lease agreements">{row => <details open={!!selected}><summary onClick={event => { if (onPick) { event.preventDefault(); onPick(`agreement:${row.id}`); } }}>{actor(row.giver)} · {actor(row.receiver)}</summary><p>Active since {dayLabel(row.activeSince)} · last delivery {dayLabel(row.lastDelivery)}</p><Items title="Monthly" items={row.monthly} /><Items title="Production share" items={row.percentage} share /><Items title="Once" items={row.once} /><Items title="Delivered" items={row.delivered} /><Items title="Lost" items={row.sunk} />
        <p>Fuel per day: {rawAmount(row.fuelDailyRaw, 32768n)} · production share: {rawAmount(row.fuelPercentageRaw, 1000n)}%</p><p>Fuel sent: {fuel(row.key, 'fuelSent')} · lost: {fuel(row.key, 'fuelLost')} · last delivery: {rawAmount(row.lastFuelDeliveredRaw, 32768n)}</p>
      </details>}</VirtualList>}
    </section>
    {deliveries && <section aria-label="Deliveries"><h4>Deliveries and payments through this date</h4>{deliveries}</section>}
  </div>;
}
