import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { visibleRange } from '../../components/countryList';
import DrillParts from '../../charts/drill/DrillParts';
import { fixedNumber, formatAmount, formatSigned } from '../../charts/valueAxis';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { TradeRun } from '../../generated/dashboard';
import { dealAccounting, dealWindow } from './resourceDeals';

// Fixed presentation height, not a deal limit: every recorded row is scrollable.
const ROW_HEIGHT = 32;

export default function ResourceDeals({ deals, pool, decimals, hour, direction, runtime, track, active }: {
  deals: readonly TradeRun[]; pool: bigint | null; decimals: number; hour: number; direction: 'imports' | 'exports';
  runtime: WidgetRuntime; track: BoundTrack; active: boolean;
}) {
  const list = useRef<HTMLDivElement>(null);
  const [view, setView] = useState({ top: 0, height: 0 });
  useLayoutEffect(() => {
    const element = list.current!;
    const resize = new ResizeObserver(() => setView({ top: element.scrollTop, height: element.clientHeight }));
    resize.observe(element);
    return () => resize.disconnect();
  }, []);
  useEffect(() => { if (list.current) list.current.scrollTop = 0; setView(before => ({ ...before, top: 0 })); }, [hour, deals]);
  const range = visibleRange(deals.length, view.top, view.height, ROW_HEIGHT, 0);
  const { shown, wanted } = useMemo(() => dealWindow(deals, range, hour), [deals, hour, range.start, range.end]); // eslint-disable-line react-hooks/exhaustive-deps
  const labels = useCountryLabelsAt(runtime.client, track, wanted, active);
  const accounting = useMemo(() => dealAccounting(deals), [deals]);
  const remaining = pool === null ? null : pool - accounting.total;
  const label = direction === 'imports' ? 'Imports by deal' : 'Exports by deal';
  return <div>
    <p className="drill-empty">{label} · {deals.length} deals</p>
    <div ref={list} className="resource-deal-list" tabIndex={0} aria-label={`${label}, ${deals.length} recorded deals`}
      onScroll={event => setView({ top: event.currentTarget.scrollTop, height: event.currentTarget.clientHeight })}>
      <ol className="resource-deal-rows" style={{ height: deals.length * ROW_HEIGHT }}>
        {shown.map((deal, offset) => {
          const partner = labels.labelAt(hour)(deal.partner), value = fixedNumber(deal.deliveredRaw, decimals);
          const share = pool ? `${Number(BigInt(deal.deliveredRaw) * 1000n / pool) / 10}%` : '—';
          return <li key={deal.deal} aria-setsize={deals.length} aria-posinset={range.start + offset + 1}
            style={{ top: (range.start + offset) * ROW_HEIGHT, height: ROW_HEIGHT }}>
            <span className="drill-part-label"><CountryFlagUrl className="resource-deal-flag" url={partner.flag} detail={partner.flagDetail} reason={partner.flagReason} pending={partner.flagPending} />{partner.name}</span>
            <span className="drill-part-bar" aria-hidden="true"><i style={{ width: `${accounting.widest ? Math.abs(Number(deal.deliveredRaw)) / Number(accounting.widest) * 100 : 0}%`,
              background: value < 0 ? 'var(--error)' : 'var(--positive)' }} /></span>
            <b className={value < 0 ? 'is-negative' : undefined}>{formatAmount(BigInt(deal.deliveredRaw), decimals, decimals)}</b><small>{share}</small>
          </li>;
        })}
      </ol>
    </div>
    {remaining !== null && remaining !== 0n && <DrillParts parts={[{ id: 'unidentified', label: 'Other', muted: true,
      value: fixedNumber(remaining, decimals), text: formatSigned(remaining, decimals, decimals) }]} format={String}
      total={pool === null ? null : fixedNumber(pool, decimals)} />}
    <p className="drill-empty">Total delivered: {formatAmount(accounting.total, decimals, decimals)}.</p>
  </div>;
}
