import { useRef } from 'react';
import type { ReactNode } from 'react';
import { dayLabel } from '../../charts/draw';
import { liveKey } from '../../charts/live';
import type { LiveValues } from '../../charts/live';
import WidgetState from '../../components/WidgetState';
import { MissingName, ValueState } from '../../components/ValueState';
import { VirtualList } from '../../components/VirtualList';
import type { MarketAt, MarketItem, MarketLevel, MarketGrant } from './types';
import { priceLevel, rawAmount } from './marketData';
const actor = (tag: string | null, missing?: string | null): ReactNode => tag ?? (missing ? <ValueState kind="missing" narrow reason={missing} /> : <MissingName reason="Country name unavailable" />);
const leaseHourNode = (value: number | null, missing?: string, zero?: string): ReactNode => value === null
  ? <ValueState kind="missing" narrow reason={missing ?? 'Date not recorded'} />
  : zero !== undefined && value <= 0 ? zero : dayLabel(value);
const leaseAmountNode = (valueRaw: string | null, missing: string | undefined, denominator: bigint, suffix = ''): ReactNode => valueRaw === null
  ? <ValueState kind="missing" narrow reason={missing ?? 'Amount not recorded'} />
  : `${rawAmount(valueRaw, denominator)}${suffix}`;
function Items({ title, items, share = false, missing = null }: { title: string; items: MarketItem[] | null; share?: boolean; missing?: string | null }) {
  return <div className="market-items"><h4>{title}</h4>{items === null ? <p><ValueState kind="missing" reason={missing ?? undefined} /></p> : items.length ? <dl>{items.map((item, index) => <div key={index}><dt title={item.variant}>{item.name ?? <MissingName reason="Equipment name unavailable" />}</dt><dd title={`Recorded amount: ${item.amountRaw}`}>{rawAmount(item.amountRaw, share ? 1000n : 100000n)}{share ? '%' : ''}</dd></div>)}</dl> : <p>None</p>}</div>;
}
function Levels({ levels }: { levels: MarketLevel[] }) { return levels.length ? <div className="market-items"><h4>Asking price levels</h4><dl>{levels.map((level, index) => <div key={index}><dt title={level.variant}>{level.name ?? <MissingName reason="Equipment name unavailable" />}</dt><dd>{priceLevel(level.level)}</dd></div>)}</dl></div> : <p>Normal asking price unless specified.</p>; }
function Grants({ grants }: { grants: MarketGrant[] }) { return grants.length ? <div className="market-items"><h4>Subsidies</h4>{grants.map((grant, index) => <p key={index}>{rawAmount(grant.cicRaw)} CIC · {grant.archetype ?? 'Archetype unavailable'} · {grant.trigger !== null ? 'Conditional eligibility' : grant.targets.length ? grant.targets.join(', ') : 'No eligible countries'}</p>)}</div> : <p>No subsidies.</p>; }
export default function TradeMarketView({ data, selected, onPick, values, deliveries }: { data: MarketAt; selected?: string | null; onPick?: (key: string) => void;
  values?: LiveValues<number | bigint>; deliveries?: ReactNode }) {
  const root = useRef<HTMLDivElement>(null);
  const offers = data.offers?.filter(row => !selected || selected === `offer:${row.country}`) ?? null;
  const contracts = data.contracts?.filter(row => !selected || selected === `contract:${row.request}:${row.id}`) ?? null;
  const agreements = data.agreements?.filter(row => !selected || selected === `agreement:${row.id}`) ?? null;
  const rawOf = (row: string, column: string): bigint | undefined => {
    const key = liveKey(row, column);
    const held = values?.raw?.(key) ?? values?.get(key);
    return held === undefined ? undefined : typeof held === 'bigint' ? held : BigInt(Math.trunc(held));
  };
  const missingOf = (row: string, column: string) => values?.missing(liveKey(row, column));
  const count = (row: string, column: string): ReactNode => {
    const value = rawOf(row, column);
    return value === undefined ? <ValueState kind="missing" narrow reason={missingOf(row, column)} /> : value.toLocaleString();
  };
  const collected = (row: string): ReactNode => {
    const value = rawOf(row, 'collected');
    return value === undefined ? <ValueState kind="missing" narrow reason={missingOf(row, 'collected')} /> : `${rawAmount(value.toString())} CIC`;
  };
  const fuel = (row: string, column: string): ReactNode => {
    const value = rawOf(row, column);
    return value === undefined ? <ValueState kind="missing" narrow reason={missingOf(row, column)} /> : rawAmount(value.toString(), 32768n);
  };

  return <div className="market-lendlease" ref={root}>
    <p className="market-caption">{dayLabel(data.hour)}</p>
    <section aria-label="International market"><h3>International market</h3>
      {offers === null ? <WidgetState kind="unavailable" message="Market inventory was not recorded." /> : <VirtualList items={offers} itemKey={row => String(row.country)} estimate={52} scrollRef={root} retainExpansion={false} label="Market offers">{row => <details open={!!selected}><summary onClick={event => { if (onPick) { event.preventDefault(); onPick(`offer:${row.country}`); } }}>{actor(row.tag)} · {row.inventory.length} listed models</summary>
        <Items title="Listed inventory" items={row.inventory} /><Levels levels={row.prices} /><Grants grants={row.subsidies} />
        <p>Subsidy balance: {rawAmount(row.subsidyTotalRaw)} CIC</p><p>Automatically send market access: {row.autoSendMarketAccess ? 'Yes' : 'No'}</p>
      </details>}</VirtualList>}
      <h4>Requests and contracts</h4>{contracts === null ? <p>Contract terms unavailable.</p> : !contracts.length ? <p>No requests or contracts.</p> : <VirtualList items={contracts} itemKey={row => `${row.request}:${row.id}`} estimate={52} scrollRef={root} retainExpansion={false} label="Market contract terms">{row => <details open={!!selected}><summary onClick={event => { if (onPick) { event.preventDefault(); onPick(`contract:${row.request}:${row.id}`); } }}>{actor(row.seller)} · {actor(row.buyer)} · {row.request ? 'Request' : 'Contract'}</summary><p>Contract {row.id} · factory assignment {row.speed}</p>
        {row.purchase ? <p>Convoys {count(row.key, 'assigned')} of {count(row.key, 'convoys')} · collected {collected(row.key)} · deliveries {count(row.key, 'deliveries')}</p>
          : !row.request && <p>{data.purchasesMissing ?? 'No purchase progress recorded for this contract.'}</p>}
        <Items title="Equipment" items={row.equipment} /><Items title="Agreed prices (CIC)" items={row.prices} /><Levels levels={row.levels} /><Grants grants={row.subsidies} /></details>}</VirtualList>}
    </section>
    <section aria-label="Lend-lease"><h3>Lend-lease</h3>
      {agreements === null ? <WidgetState kind="unavailable" message="Lend-lease terms were not recorded." /> : !agreements.length ? <p>No active agreements.</p> : <VirtualList items={agreements} itemKey={row => row.id} estimate={52} scrollRef={root} retainExpansion={false} label="Lend-lease agreements">{row => <details open={!!selected}><summary onClick={event => { if (onPick) { event.preventDefault(); onPick(`agreement:${row.id}`); } }}>{actor(row.giver)} · {actor(row.receiver, row.receiverMissing)}</summary><p>Active since {leaseHourNode(row.activeSince, row.activeSinceMissing)} · last delivery {leaseHourNode(row.lastDelivery, row.lastDeliveryMissing, 'No delivery yet')}</p><Items title="Monthly" items={row.monthly} missing={row.monthlyMissing} /><Items title="Production share" items={row.percentage} share missing={row.percentageMissing} /><Items title="Once" items={row.once} missing={row.onceMissing} /><Items title="Delivered" items={row.delivered} missing={row.deliveredMissing} /><Items title="Lost" items={row.sunk} missing={row.sunkMissing} />
        <p>Fuel per day: {leaseAmountNode(row.fuelDailyRaw, row.fuelDailyRawMissing, 32768n)} · production share: {leaseAmountNode(row.fuelPercentageRaw, row.fuelPercentageRawMissing, 1000n, '%')}</p><p>Fuel sent: {fuel(row.key, 'fuelSent')} · lost: {fuel(row.key, 'fuelLost')} · last delivery: {leaseAmountNode(row.lastFuelDeliveredRaw, row.lastFuelDeliveredRawMissing, 32768n)}</p>
      </details>}</VirtualList>}
    </section>
    {deliveries && <section aria-label="Deliveries"><h4>Deliveries and payments through this date</h4>{deliveries}</section>}
  </div>;
}
