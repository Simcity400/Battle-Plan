import { useCallback, useMemo, useReducer, useRef, useState } from 'react';
import type { CSSProperties } from 'react';
import WidgetState from '../../components/WidgetState';
import { ChipSet } from '../../components/ChipSet';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { Icon } from '../../components/Icons';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState';
import { identityTagAt, partyTagsAt } from '../../dashboard/marketHistory';
import { preparedPrefix } from '../../dashboard/preparedPrefix';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import LogList from '../../charts/log/LogList';
import type { LogRow } from '../../charts/log/logData';
import { logCountryRequests } from '../../charts/log/logData';
import { logSettings } from '../../charts/log/logOptions';
import { useChartTokens } from '../../charts/useChartTokens';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { selectedTags } from '../../dashboard/countrySelection';
import { MEMBERSHIP_PREPARING } from '../../dashboard/groups';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { WidgetProps } from '../../dashboard/definitions';
import MarketDrill from '../../dashboard/MarketDrill';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { DETAILS, OPTIONS } from './definition';
import { KINDS, LEDGER_EMPTY, ledgerCountries, ledgerEmpty, ledgerFigure, ledgerIcon, ledgerTitle, loadedHolders, shownKinds } from './ledgerData';
import type { KindKey, LedgerItem } from './ledgerData';
import { useMarketLedger } from './useMarketLedger';
import '../event-feed/eventFeed.css';
import './marketLedger.css';

const COLUMNS = 'calc(var(--space-6) + var(--space-2) + var(--space-1)) minmax(0, 1fr) auto';
const PALETTE: Record<number, number> = { 10: 0, 11: 2, 12: 5, 20: 1, 21: 3, 34: 1 };

/** Every market occurrence of the chosen countries' deals, newest first: contracts
 * made and removed, requests posted, deliveries and payments. A row opens the contract. */
export default function MarketLedgerWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  // The open drill may still show an earlier hour while its newest loads.
  const [drillShown, setDrillShown] = useState<number | undefined>();
  const playbackHour = usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const tokens = useChartTokens();
  const options = resolveOptions(OPTIONS, widget.options);
  const log = logSettings(options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = window ? Math.floor(window.lastHour) : null;
  // A date range's start bounds the ledger as its end does; otherwise it starts with the branch.
  const from = widget.time.mode === 'range' && window && track && hour !== null ? Math.min(Math.max(Math.floor(window.firstHour), track.branch.firstHour), hour) : null;
  const firstHour = from ?? track?.branch.firstHour ?? 0;
  const needsGroups = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const fallback = useMemo(() => widget.countries.kind === 'selected' ? selectedTags(widget.countries, track) : [], [widget.countries, track]);
  const chosen = useMemo(() => track ? ledgerCountries(widget.countries, fallback, groups.data, firstHour, track.branch.lastHour)
    : { countries: null, reason: null, known: () => false }, [widget.countries, fallback, groups.data, track, firstHour]);
  const enabled = (key: KindKey) => options[key] !== false;
  const kinds = shownKinds(enabled);
  const ledger = useMarketLedger(runtime, track, active, from, hour, chosen.countries, kinds);
  const warnings = useMemo(() => chosen.countries?.unobserved
    ? [...ledger.warnings, 'Group membership was not recorded at some hours; occurrences then are not listed.'] : ledger.warnings,
  [chosen.countries?.unobserved, ledger.warnings]);
  useDiagnostics(onDiagnostics, warnings);
  const [visible, setVisible] = useState<readonly LogRow<LedgerItem>[]>([]);
  const wanted = useMemo(() => logCountryRequests(visible, row => ({ hour: row.hour, tags: [row.item.seller, row.item.buyer] })), [visible]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const name = useCallback((row: LogRow<LedgerItem>) => (tag: string | null) => tag === null ? 'Country name unavailable' : labelAt(row.hour)(tag).name, [labelAt]);
  const anchor = useRef<HTMLDivElement>(null);
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  const pickOwner = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id]) : null;
  const [pick, setPick] = useState<{ client: typeof runtime.client; owner: string | null; item: LedgerItem } | null>(null);
  const picked = pick && pick.client === runtime.client && pick.owner === pickOwner ? pick.item : null;
  const describe = (row: LogRow<LedgerItem>) => {
    const named = name(row);
    const figure = ledgerFigure(row.item);
    return [dayLabel(row.hour), ledgerTitle(row.item, named), `${named(row.item.seller)} → ${named(row.item.buyer)}`,
      row.item.equipment, figure?.text].filter(Boolean).join(' · ');
  };
  const open = (item: LedgerItem, at: number) => {
    setPick({ client: runtime.client, owner: pickOwner, item });
    dispatch({ type: 'open', breakdown: DETAILS.id, hour: at, playing: runtime.clock.getSnapshot().playing, playhead: widget.time.mode === 'follow' ? hour : null });
  };
  const drillAt = drillHour(drill, hour) ?? hour ?? 0;
  // A picked party follows its holder through the identity history where it is exact, else
  // through what the loaded pages read.
  const identities = useCountryGroups(runtime, track, active && picked !== null);
  const holders = useMemo(() => loadedHolders(ledger.pages), [ledger.pages]);
  const exact = useMemo(() => {
    const data = identities.data;
    if (!data) return undefined;
    const prefix = preparedPrefix(data);
    return (id: string, hour: number) => identityTagAt(data.countryIdentities, prefix, id, hour);
  }, [identities.data]);
  const pickedTags = picked ? partyTagsAt(picked.parties, holders, drillAt, exact) : null;
  const chips = KINDS.map(kind => ({ value: kind.key, label: kind.label }));

  if (!track) return <WidgetState kind="empty" icon="history" message="Load a campaign to see market activity." />;
  if (hour === null) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  const groupFailure = needsGroups && groups.error ? <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (chosen.reason) return <>{groupFailure}<WidgetState kind="unavailable" message={chosen.reason} /></>;
  if (ledger.error) return <>{groupFailure}<WidgetState kind="error" detail={ledger.error} onRetry={ledger.retry} /></>;
  if (kinds.length && (!chosen.countries || !ledger.data)) return <>{groupFailure}<WidgetState kind="loading" label="Loading market activity…" /></>;
  const main = <div className="log-widget market-ledger">
    {log.chips && <div className="log-chips"><ChipSet label="Market occurrences" chips={chips}
      selected={KINDS.filter(kind => enabled(kind.key)).map(kind => kind.key)} max={KINDS.length} onToggle={key => setOption?.(key, !enabled(key as KindKey))} /></div>}
    <LogList key={ledger.identity} pages={ledger.pages} label="Market ledger" density={log.density} months={log.months}
      preparedCoverage={ledger.prefix?.coverage} animate={log.animate && widget.time.mode === 'follow'} playhead={widget.time.mode === 'follow' ? hour : null}
      columns={COLUMNS} onVisible={setVisible} onWantedPages={ledger.onWantedPages}
      renderRow={row => {
        const named = name(row);
        const figure = ledgerFigure(row.item);
        const seller = labelAt(row.hour)(row.item.seller ?? '');
        return <>
          <span className="log-art" aria-hidden="true"><span className="event-feed-icon" style={{ '--event-colour': tokens.categories[PALETTE[row.item.row.kind] ?? 0] } as CSSProperties}>
            <Icon name={ledgerIcon(row.item.row.kind)} /></span></span>
          <span className="log-text market-ledger-text" onClick={() => open(row.item, row.hour)}><b>{ledgerTitle(row.item, named)}</b>
            <small>{row.item.seller && <CountryFlagUrl className="log-flag" url={seller.flag} reason={seller.flagReason} pending={seller.flagPending} />}
              {named(row.item.seller)} → {named(row.item.buyer)}{row.item.equipment ? ` · ${row.item.equipment}` : ''}</small></span>
          {figure ? <span className={`market-ledger-figure${figure.negative ? ' is-negative' : ''}`} onClick={() => open(row.item, row.hour)}>{figure.text}</span> : <span />}
        </>;
      }}
      describe={describe}
      tooltip={row => {
        const named = name(row);
        const figure = ledgerFigure(row.item);
        return [{ title: ledgerTitle(row.item, named), subtitle: dayLabel(row.hour), rows: [
          { label: 'Seller', value: named(row.item.seller) }, { label: 'Buyer', value: named(row.item.buyer) },
          ...row.item.equipment ? [{ label: 'Equipment', value: row.item.equipment }] : [],
          ...figure ? [{ label: 'Amount', value: figure.text }] : [],
          ...row.item.row.reason ? [{ label: row.item.row.reason, tone: 'muted' as const }] : [],
          { label: 'Contract', value: row.item.row.contract, tone: 'muted' as const },
        ] }];
      }}
      menu={row => [{ label: 'Show contract', run: () => open(row.item, row.hour) }, jumpItem(runtime, track.id, row.hour)]}
      // Occurrences whose parties' membership is still being prepared are not listed yet: none is not proof of none.
      empty={!chosen.known(firstHour, hour) ? MEMBERSHIP_PREPARING : ledger.data ? LEDGER_EMPTY[ledgerEmpty(ledger.data, firstHour, hour)] : LEDGER_EMPTY.none} />
    {ledger.errors.size > 0 && <div className="event-feed-load">Some occurrences couldn’t load. <button type="button" onClick={ledger.retry}>Retry</button></div>}
  </div>;
  return <div ref={anchor} className="market-ledger-host">{groupFailure}
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown !== null && !!picked} anchor={anchor}
      title={picked ? ledgerTitle(picked, tag => tag ?? 'Country name unavailable') : DETAILS.label} day={dayLabel(drillShown ?? drillAt)}
      following={drill.follow} canFollow={widget.time.mode === 'follow'} onClose={() => dispatch({ type: 'close' })} onFollow={() => dispatch({ type: 'follow' })}
      main={main}>
      {/* A posted request (kind 10) is the market's request with that id, not a contract. */}
      {picked && (pickedTags ? <MarketDrill runtime={runtime} track={track} hour={drillAt} active={active} onAnswered={setDrillShown}
        tags={pickedTags} selected={`contract:${picked.row.kind === 10}:${picked.row.contract}`} />
        : <WidgetState kind="unavailable" message="Neither party was recorded at this hour." />)}
    </DrillHost>
  </div>;
}
