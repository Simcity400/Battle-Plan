import { useCallback, useMemo } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import { CountryFlagUrl } from '../components/CountryFlag';
import { Icon } from '../components/Icons';
import WidgetState from '../components/WidgetState';
import { dayLabel } from '../charts/draw';
import { jumpItem } from '../charts/jump';
import LogList from '../charts/log/LogList';
import { useRevealClock } from '../charts/reveal';
import { logCountryRequests, NO_COUNTRY_REQUESTS, sameCountryRequests } from '../charts/log/logData';
import type { CountryRequest, LogDensity, LogRow } from '../charts/log/logData';
import { useChartTokens } from '../charts/useChartTokens';
import { useCountryLabelsAt } from './countryFlagsAt';
import type { WidgetRuntime } from './definitions';
import type { BoundTrack } from './model';
import { useLogPages } from './useLogPages';
import { useDistinctState } from './useDistinctState';
import type { LogCut, LogSpec } from './useLogPages';
import { ledgerEntry, ledgerFacets, ledgerFigure, ledgerIcon, ledgerPartyName, ledgerSelection, ledgerTitle, MARKET_LEDGER, MARKET_LOG } from './marketLedger.ts';
import type { LedgerItem, PartyState } from './marketLedger';
import './market.css';
import type { BoundWindow } from './regionTime';

const COLUMNS = 'calc(var(--space-6) + var(--space-2) + var(--space-1)) minmax(0, 1fr) auto';
const PALETTE: Record<number, number> = { 3: 1, 4: 5, 5: 4, 6: 5, 10: 0, 11: 2, 12: 5, 20: 1, 21: 3, 34: 1 };

export interface MarketOccurrencesProps {
  runtime: WidgetRuntime;
  track: BoundTrack;
  kinds: readonly number[];
  tags: readonly string[] | null;
  span: BoundWindow | null;
  cut: LogCut;
  active: boolean;
  label: string;
  density: LogDensity;
  months: boolean;
  animate: boolean;
  onOpen?: (item: LedgerItem, hour: number) => void;
  empty?: ReactNode;
}

export default function MarketOccurrences({ runtime, track, kinds, tags, span, cut, active, label, density, months, animate, onOpen, empty }: MarketOccurrencesProps) {
  const tokens = useChartTokens();
  const facets = useMemo(() => ledgerFacets(kinds, tags), [kinds, tags]);
  const spec = useMemo<LogSpec | null>(() => kinds.length && span ? { source: track.source, branchId: track.branch.id, log: MARKET_LOG, product: MARKET_LEDGER, facets,
    firstHour: span.firstHour, lastHour: span.lastHour } : null, [track.source, track.branch.id, kinds.length, facets, span?.firstHour, span?.lastHour]);
  const list = useLogPages<LedgerItem>(runtime.client, runtime.clock, spec, cut, active, ledgerEntry);
  const offset = cut.follow ? cut.offset : 0;
  const trackHour = useCallback((clockHour: number) => Math.floor(clockHour) + offset, [offset]);
  const reveal = useRevealClock(runtime.clock, cut.follow, trackHour);
  const [wanted, setWanted] = useDistinctState<readonly CountryRequest[]>(NO_COUNTRY_REQUESTS, sameCountryRequests);
  const onVisible = useCallback((rows: readonly LogRow<LedgerItem>[]) => setWanted(logCountryRequests(rows, row => ({ hour: row.hour, tags: [row.item.seller, row.item.buyer] }))), [setWanted]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const name = useCallback((row: LogRow<LedgerItem>) => (tag: string | null, state: PartyState = 'notRecorded') =>
    ledgerPartyName(tag, state, t => labelAt(row.hour)(t).name), [labelAt]);
  if (list.error) return <WidgetState kind="error" failure={list.error} onRetry={list.retry} />;
  const describe = (row: LogRow<LedgerItem>) => {
    const named = name(row), figure = ledgerFigure(row.item);
    return [dayLabel(row.hour), ledgerTitle(row.item, named), `${named(row.item.seller, row.item.sellerState)} → ${named(row.item.buyer, row.item.buyerState)}`, row.item.equipment, figure?.text].filter(Boolean).join(' · ');
  };
  return <LogList pages={list.pages} onWantedPages={list.onWantedPages} label={label} density={density} months={months}
    animate={animate && cut.follow} playhead={cut.follow ? Math.floor(runtime.clock.getHour()) + cut.offset : null} reveal={reveal} columns={COLUMNS} onVisible={onVisible}
    renderRow={row => {
      const named = name(row), figure = ledgerFigure(row.item), seller = row.item.seller === null ? null : labelAt(row.hour)(row.item.seller);
      const open = onOpen && ledgerSelection(row.item) !== null ? () => onOpen(row.item, row.hour) : undefined;
      return <>
        <span className="log-art" aria-hidden="true"><span className="event-feed-icon" style={{ '--event-colour': tokens.categories[row.item.kind !== null ? (PALETTE[row.item.kind] ?? 0) : 0] } as CSSProperties}>
          <Icon name={ledgerIcon(row.item.kind)} /></span></span>
        <span className="log-text market-ledger-text" onClick={open}><b>{ledgerTitle(row.item, named)}</b>
          <small>{seller && <CountryFlagUrl className="log-flag" url={seller.flag} detail={seller.flagDetail} reason={seller.flagReason} pending={seller.flagPending} />}
            {named(row.item.seller, row.item.sellerState)} → {named(row.item.buyer, row.item.buyerState)}{row.item.variant !== '0' ? ` · ${row.item.equipment ?? row.item.equipmentMissing ?? 'Equipment design not recorded'}` : ''}</small></span>
        {figure ? <span className={`market-ledger-figure${figure.negative ? ' is-negative' : ''}`} onClick={open}>{figure.text}</span> : <span />}
      </>;
    }}
    describe={describe}
    tooltip={row => {
      const named = name(row), figure = ledgerFigure(row.item);
      return [{ title: ledgerTitle(row.item, named), subtitle: dayLabel(row.hour), rows: [
        { label: 'Seller', value: named(row.item.seller, row.item.sellerState) }, { label: 'Buyer', value: named(row.item.buyer, row.item.buyerState) },
        ...row.item.variant !== '0' ? [{ label: 'Equipment', value: row.item.equipment ?? row.item.equipmentMissing ?? 'Equipment design not recorded' }] : [],
        ...figure ? [{ label: 'Amount', value: figure.text }] : [],
        ...row.item.reason ? [{ label: row.item.reason, tone: 'muted' as const }] : [],
        { label: 'Contract', value: row.item.contract ?? 'Not recorded', tone: 'muted' as const },
      ] }];
    }}
    menu={row => [...onOpen && ledgerSelection(row.item) !== null ? [{ label: 'Show contract', run: () => onOpen(row.item, row.hour) }] : [], jumpItem(runtime, track.id, row.hour)]}
    empty={list.counts ? empty ?? 'No market activity matches these settings.' : <WidgetState kind="loading" label="Loading market activity…" />} />;
}
