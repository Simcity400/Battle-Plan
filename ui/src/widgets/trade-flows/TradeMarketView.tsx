import { useRef } from 'react';
import type { ReactNode } from 'react';
import { dayLabel } from '../../charts/draw';
import { liveKey } from '../../charts/live';
import type { LiveValues } from '../../charts/live';
import WidgetState from '../../components/WidgetState';
import { MissingName, ValueState } from '../../components/ValueState';
import { VirtualList } from '../../components/VirtualList';
import type { MarketAt, MarketItem, MarketLevel, MarketGrant, LeaseAmount, LeaseHour, LeaseItems } from './types';
import { priceLevel, rawAmount } from './marketData';
const actor = (tag: string | null, missing?: string | null): ReactNode => tag ?? (missing ? <ValueState kind="missing" narrow reason={missing} /> : <MissingName reason="Country name unavailable" />);
const leaseHourNode = (held: LeaseHour, zero: string): ReactNode => held.value === null
  ? <ValueState kind="missing" narrow reason={held.missing ?? 'Date not recorded'} />
  : held.value > 0 ? dayLabel(held.value) : zero;
const leaseAmountNode = (held: LeaseAmount, denominator: bigint, suffix = ''): ReactNode => held.valueRaw === null
  ? <ValueState kind="missing" narrow reason={held.missing ?? 'Amount not recorded'} />
  : `${rawAmount(held.valueRaw, denominator)}${suffix}`;
function Items({ title, items, share = false, missing = null }: { title: string; items: MarketItem[] | null; share?: boolean; missing?: string | null }) {
  return <div className="market-items"><h4>{title}</h4>{items === null ? <p><ValueState kind="missing" reason={missing ?? undefined} /></p> : items.length ? <dl>{items.map((item, index) => <div key={index}><dt title={item.variant}>{item.name ?? <MissingName reason="Equipment name unavailable" />}</dt><dd title={`Recorded amount: ${item.amountRaw}`}>{rawAmount(item.amountRaw, share ? 1000n : 100000n)}{share ? '%' : ''}</dd></div>)}</dl> : <p>None</p>}</div>;
}
function Pool({ title, pool, share = false }: { title: string; pool: LeaseItems; share?: boolean }) {
  return <Items title={title} items={pool.items} share={share} missing={pool.missing ?? null} />;
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
      {agreements === null ? <WidgetState kind="unavailable" message="Lend-lease terms were not recorded." /> : !agreements.length ? <p>No active agreements.</p> : <VirtualList items={agreements} itemKey={row => row.id} estimate={52} scrollRef={root} retainExpansion={false} label="Lend-lease agreements">{row => <details open={!!selected}><summary onClick={event => { if (onPick) { event.preventDefault(); onPick(`agreement:${row.id}`); } }}>{actor(row.giver)} · {actor(row.receiver, row.receiverMissing)}</summary><p>Active since {leaseHourNode(row.activeSince, 'Not recorded')} · last delivery {leaseHourNode(row.lastDelivery, 'No delivery yet')}</p><Pool title="Monthly" pool={row.monthly} /><Pool title="Production share" pool={row.percentage} share /><Pool title="Once" pool={row.once} /><Pool title="Delivered" pool={row.delivered} /><Pool title="Lost" pool={row.sunk} />
        <p>Fuel per day: {leaseAmountNode(row.fuelDailyRaw, 32768n)} · production share: {leaseAmountNode(row.fuelPercentageRaw, 1000n, '%')}</p><p>Fuel sent: {fuel(row.key, 'fuelSent')} · lost: {fuel(row.key, 'fuelLost')} · last delivery: {leaseAmountNode(row.lastFuelDeliveredRaw, 32768n)}</p>
      </details>}</VirtualList>}
    </section>
    {deliveries && <section aria-label="Deliveries"><h4>Deliveries and payments through this date</h4>{deliveries}</section>}
  </div>;
}
