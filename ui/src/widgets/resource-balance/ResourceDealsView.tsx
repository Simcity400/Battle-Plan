import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { visibleRange } from '../../components/countryList';
import type { Live, LiveValues } from '../../charts/live';
import LiveText from '../../charts/LiveText';
import type { LiveTexts } from '../../charts/LiveText';
import { formatAmount, formatSigned } from '../../charts/valueAxis';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { ResourceDeal } from '../../generated/dashboard';
import { balanceOf, dealAccounting, dealAt, poolsAt } from './resourceData';

const ROW_HEIGHT = 32;

function rowLook(deal: ResourceDeal, values: LiveValues<number>, decimals: number, pool: bigint | null, widest: bigint) {
  const raw = dealAt(deal, values).deliveredRaw;
  if (raw === null) return { amount: '—', width: '0%', negative: false, share: '—', title: values.missing(`${deal.deal}\u0000delivered`) };
  const value = BigInt(raw), magnitude = value < 0n ? -value : value;
  return { amount: formatAmount(value, decimals, decimals), width: `${widest ? Number(magnitude * 100000n / widest) / 1000 : 0}%`, negative: value < 0n,
    share: pool ? `${Number(value * 1000n / pool) / 10}%` : '—', title: '' };
}

export default function ResourceDeals({ deals, live, resource, decimals, hour, direction, runtime, track, active }: {
  deals: readonly ResourceDeal[]; live: Live<LiveValues<number>>; resource: number; decimals: number; hour: number; direction: 'imports' | 'exports';
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
  useEffect(() => { if (list.current) list.current.scrollTop = 0; setView(before => ({ ...before, top: 0 })); }, [hour]);
  const range = visibleRange(deals.length, view.top, view.height, ROW_HEIGHT, 0);
  const shown = useMemo(() => deals.slice(range.start, range.end), [deals, range.start, range.end]);
  const wanted = useMemo(() => shown.map(deal => ({ tag: deal.partner, hour })), [shown, hour]);
  const labels = useCountryLabelsAt(runtime.client, track, wanted, active);
  const poolOf = (values: LiveValues<number>) => {
    const pools = poolsAt(values, resource);
    return pools ? direction === 'imports' ? balanceOf(pools).imported : balanceOf(pools).exported : null;
  };
  const cells = useRef(new Map<string, { amount: HTMLElement | null; bar: HTMLElement | null; share: HTMLElement | null }>());
  useLayoutEffect(() => {
    const paint = () => {
      const values = live.read(), pool = poolOf(values);
      const { widest } = dealAccounting(deals.map(deal => dealAt(deal, values)));
      for (const deal of shown) {
        const held = cells.current.get(deal.deal);
        if (!held) continue;
        const look = rowLook(deal, values, decimals, pool, widest);
        if (held.amount && held.amount.textContent !== look.amount) { held.amount.textContent = look.amount; held.amount.className = look.negative ? 'is-negative' : ''; held.amount.title = look.title; }
        if (held.bar) { held.bar.style.width = look.width; held.bar.style.background = look.negative ? 'var(--error)' : 'var(--positive)'; }
        if (held.share && held.share.textContent !== look.share) held.share.textContent = look.share;
      }
    };
    paint();
    return live.subscribe(paint);
  });
  const texts = useMemo<LiveTexts>(() => ({
    text: key => {
      const values = live.read(), pool = poolOf(values), { total } = dealAccounting(deals.map(deal => dealAt(deal, values)));
      if (key === 'total') return `Total delivered: ${total === null ? 'not known' : formatAmount(total, decimals, decimals)}.`;
      const rest = pool === null || total === null ? null : pool - total;
      return rest === null || rest === 0n ? '' : `Other (not from a listed deal): ${formatSigned(rest, decimals, decimals)}.`;
    },
    subscribe: changed => live.subscribe(changed),
  }), [live, deals, decimals, resource, direction]);
  const label = direction === 'imports' ? 'Imports by deal' : 'Exports by deal';
  const values = live.read(), pool = poolOf(values), { widest } = dealAccounting(deals.map(deal => dealAt(deal, values)));
  return <div>
    <p className="drill-empty">{label} · {deals.length} deals</p>
    <div ref={list} className="resource-deal-list" tabIndex={0} aria-label={`${label}, ${deals.length} recorded deals`}
      onScroll={event => setView({ top: event.currentTarget.scrollTop, height: event.currentTarget.clientHeight })}>
      <ol className="resource-deal-rows" style={{ height: deals.length * ROW_HEIGHT }}>
        {shown.map((deal, offset) => {
          const partner = labels.labelAt(hour)(deal.partner), look = rowLook(deal, values, decimals, pool, widest);
          const held = () => cells.current.get(deal.deal) ?? (cells.current.set(deal.deal, { amount: null, bar: null, share: null }), cells.current.get(deal.deal)!);
          return <li key={deal.deal} aria-setsize={deals.length} aria-posinset={range.start + offset + 1}
            style={{ top: (range.start + offset) * ROW_HEIGHT, height: ROW_HEIGHT }}>
            <span className="drill-part-label"><CountryFlagUrl className="resource-deal-flag" url={partner.flag} detail={partner.flagDetail} reason={partner.flagReason} pending={partner.flagPending} />{partner.name}</span>
            <span className="drill-part-bar" aria-hidden="true"><i ref={element => { held().bar = element; }}
              style={{ width: look.width, background: look.negative ? 'var(--error)' : 'var(--positive)' }} /></span>
            <b ref={element => { held().amount = element; }} className={look.negative ? 'is-negative' : undefined} title={look.title}>{look.amount}</b>
            <small ref={element => { held().share = element; }}>{look.share}</small>
          </li>;
        })}
      </ol>
    </div>
    <p className="drill-empty"><LiveText live={texts} at="other" /></p>
    <p className="drill-empty"><LiveText live={texts} at="total" /></p>
  </div>;
}
