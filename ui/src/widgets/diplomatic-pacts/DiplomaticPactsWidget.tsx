import { useCallback, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import { Segmented } from '../../components/Segmented';
import { CountryFlagUrl } from '../../components/CountryFlag';
import LogList from '../../charts/log/LogList';
import type { LogRow } from '../../charts/log/logData';
import { logSettings } from '../../charts/log/logOptions';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/columns';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { fitTooltip } from '../../charts/tooltip';
import type { TooltipRow } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { MEMBERSHIP_PREPARING, resolveCountries } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions } from '../../dashboard/options';
import { usePlaybackSpeed } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { pendingAt, preparedPrefix } from '../../dashboard/preparedPrefix';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { DEFAULT_SORT, OPTIONS, VIEWS } from './definition';
import { PACT_WIDTHS, days, daysText, ownerReads, pactSince, pactState, proposalHour, proposalState, shownPacts, shownProposals } from './pactsData';
import type { DiplomacyPact as Pact, DiplomacyPactsReply, DiplomacyProposal as Proposal, DiplomacyProposalsReply } from '../../generated/dashboard';
import type { PactState } from './pactsData';
import '../../charts/log/log.css';
import './diplomaticPacts.css';

const NO_WARNINGS: string[] = [];
/** A pact's stored end where it has one, else the read that first stopped listing it. */
const untilHour = (state: PactState, pact: Pact) => state.state === 'present' ? Number.MAX_SAFE_INTEGER
  : pact.endHour ?? (state.state === 'gone' ? state.by : pact.lastSeen);
const untilText = (state: PactState, pact: Pact, at: number) => state.state === 'present' ? `Present · ${daysText(days(pactSince(pact), at))}`
  : pact.endHour != null ? dayLabel(pact.endHour) : state.state === 'gone' ? `gone by ${dayLabel(state.by)}` : `last listed ${dayLabel(pact.lastSeen)}`;
const pactKey = (pact: Pact) => pact.id;

export default function DiplomaticPactsWidget({ widget, runtime, active, headerSlot, setOption, onDiagnostics }: WidgetProps) {
  const playback = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const options = resolveOptions(OPTIONS, widget.options);
  const view = options.view === 'pacts' ? 'pacts' : 'proposals';
  const viewport = useViewportHeight();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const at = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const live = widget.time.mode === 'follow';
  const groups = useCountryGroups(runtime, track, active && widget.countries.kind === 'group');
  const resolved = track && widget.countries.kind !== 'all'
    ? resolveCountries(widget.countries, [defaultCountryTag(track) ?? ''].filter(Boolean), groups.data, track.branch.lastHour, Number.MAX_SAFE_INTEGER) : null;
  const tags = widget.countries.kind === 'all' ? null : resolved?.tags ?? [];
  const request = (product: 'pacts' | 'proposals') => track && (tags === null || tags.length) ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'diplomacy-state', params: { view: product, branchId: track.branch.id, tags } } : null;
  const pacts = useProduct<DiplomacyPactsReply>(runtime.client, view === 'pacts' ? request('pacts') : null, active, undefined, track?.source);
  const proposals = useProduct<DiplomacyProposalsReply>(runtime.client, view === 'proposals' ? request('proposals') : null, active, undefined, track?.source);
  const kinds = useMemo(() => { const chosen = contentSet(options.kinds).map(contentKey); return chosen.length ? new Set(chosen) : null; }, [options.kinds]);
  const kindNames = useMemo(() => new Map(pacts.data?.kinds.map(kind => [kind.kind, kind.name]) ?? []), [pacts.data]);
  const reads = useMemo(() => ownerReads(pacts.data?.reads ?? []), [pacts.data]);
  const stateOf = useCallback((pact: Pact) => pactState(pact, at, reads), [at, reads]);
  const pactRows = useMemo(() => pacts.data ? shownPacts(pacts.data.pacts, at, kinds, options.ended !== 'hidden', reads) : [], [pacts.data, at, kinds, options.ended, reads]);
  const proposalRows = useMemo<LogRow<Proposal>[]>(() => proposals.data ? shownProposals(proposals.data.proposals, at, options.waiting === true)
    .map(item => ({ key: item.id, hour: proposalHour(item), revealedHour: item.firstSeen, by: item.storedHour == null, item })) : [], [proposals.data, at, options.waiting]);
  // Countries are named as they were when the item was last listed (or at the hour shown, if
  // that is earlier): a tag a later country takes never renames an earlier pact's endpoint.
  // Only the countries of mounted rows are read; `visible` keeps each with its item's last listing.
  const [visible, setVisible] = useState<readonly { last: number; tag: string }[]>([]);
  const wanted = useMemo(() => visible.map(item => ({ hour: Math.min(at, item.last), tag: item.tag })), [visible, at]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const country = useCallback((tag: string | null, hour: number) => {
    if (!tag) return { name: 'Country not recorded' };
    const named = labelAt(Math.min(at, hour))(tag);
    return named.known === undefined ? { ...named, name: 'Loading name…' } : named;
  }, [labelAt, at]);
  const report = (items: readonly { last: number; tag: string }[]) => setVisible(previous => previous.length === items.length
    && previous.every((item, index) => item.last === items[index].last && item.tag === items[index].tag) ? previous : items);
  const kindName = useCallback((kind: string) => kindNames.get(kind) ?? 'Relation name unavailable', [kindNames]);
  const diagnostics = useMemo(() => {
    const data = view === 'pacts' ? pacts.data : proposals.data;
    const unnamed = pacts.data?.kinds.filter(kind => !kind.name) ?? [];
    return [...(data?.warnings ?? []), ...unnamed.map(kind => `Relation kind ${kind.kind} has no name (${kind.count} pacts): ${kind.reason ?? 'not in the recorded content'}`)];
  }, [view, pacts.data, proposals.data]);
  useDiagnostics(onDiagnostics, diagnostics.length ? diagnostics : NO_WARNINGS);
  const columns = useMemo<readonly TableColumn<Pact>[]>(() => [
    { key: 'kind', header: 'Kind', label: 'Kind', width: PACT_WIDTHS.kind, value: pact => kindName(pact.kind),
      render: pact => <span title={kindNames.get(pact.kind) ? kindName(pact.kind) : `Recorded kind: ${pact.kind}`}>{kindName(pact.kind)}</span> },
    { key: 'from', header: 'From', label: 'From', width: PACT_WIDTHS.country, value: pact => country(pact.first, pact.lastSeen).name, renderMissing: true,
      render: pact => pact.first ? <CountryCell {...country(pact.first, pact.lastSeen)} /> : 'Country not recorded' },
    { key: 'to', header: 'To', label: 'To', width: PACT_WIDTHS.country, value: pact => country(pact.second, pact.lastSeen).name, renderMissing: true,
      render: pact => pact.second ? <CountryCell {...country(pact.second, pact.lastSeen)} /> : 'Country not recorded' },
    { key: 'since', header: 'Since', label: 'Since', width: 'max-content', priority: 2, value: pactSince,
      text: pact => `${pact.startHour == null ? 'by ' : ''}${dayLabel(pactSince(pact))}`,
      render: pact => `${pact.startHour == null ? 'by ' : ''}${dayLabel(pactSince(pact))}` },
    { key: 'until', header: 'Until', label: 'Until', width: 'max-content', value: pact => untilHour(stateOf(pact), pact),
      text: pact => untilText(stateOf(pact), pact, at),
      render: pact => { const state = stateOf(pact); return state.state === 'present'
        ? <span className="pill accent">{live && <i className="live-dot" aria-hidden="true" />}{untilText(state, pact, at)}</span>
        : <span className={state.state === 'unknown' ? 'pact-unknown' : undefined}>{untilText(state, pact, at)}</span>; } },
    { key: 'status', header: 'Status', label: 'Status', width: 'max-content', priority: 1, renderMissing: true,
      value: pact => pact.cancelled ? 'Cancelled' : pact.retired ? 'Retired' : null,
      render: pact => pact.cancelled ? <span className="pill">Cancelled</span> : pact.retired ? <span className="pill">Retired</span> : null },
  ], [kindName, kindNames, country, at, live, stateOf]);
  const pactTip = useCallback((pact: Pact) => {
    const rows: TooltipRow[] = [
      { label: 'From', value: country(pact.first, pact.lastSeen).name, countryFlag: pact.first ? country(pact.first, pact.lastSeen) : undefined },
      { label: 'To', value: country(pact.second, pact.lastSeen).name, countryFlag: pact.second ? country(pact.second, pact.lastSeen) : undefined },
      { label: 'Stored start', value: pact.startHour == null ? 'Not recorded' : dayLabel(pact.startHour) },
      { label: 'Stored end', value: pact.endHour == null ? 'Not recorded' : dayLabel(pact.endHour) },
      { label: 'First recorded', value: dayLabel(pact.firstSeen) },
      { label: 'Last listed', value: pact.presentAtEnd ? 'Still listed' : dayLabel(pact.lastSeen) },
      ...(stateOf(pact).state === 'gone' ? [{ label: 'No longer listed by', value: dayLabel((stateOf(pact) as { by: number }).by) }] : []),
      ...(pact.cancelled ? [{ label: 'Cancelled', value: 'Yes' }] : []), ...(pact.retired ? [{ label: 'Retired', value: 'Yes' }] : []),
    ];
    return fitTooltip([{ title: kindName(pact.kind), subtitle: dayLabel(at), rows }], viewport);
  }, [country, kindName, at, viewport, stateOf]);
  const flag = (tag: string | null, hour: number) => { const named = country(tag, hour);
    return <>{tag && 'flag' in named && <CountryFlagUrl className="log-flag" url={named.flag} reason={named.flagReason} pending={named.flagPending} />}<b>{named.name}</b></>; };
  const renderCard = (row: LogRow<Proposal>) => {
    const item = row.item, state = proposalState(item, at);
    const rerouted = (item.originalSender && item.originalSender !== item.sender) || (item.originalRecipient && item.originalRecipient !== item.recipient);
    const since = proposalHour(item);
    return <article className="proposal-card">
      <div className="proposal-route log-text">{flag(item.sender, item.lastSeen)}<span className="proposal-arrow" aria-hidden="true">→</span>{flag(item.recipient, item.lastSeen)}</div>
      {rerouted && <div className="proposal-original log-text">Originally {country(item.originalSender, item.lastSeen).name} → {country(item.originalRecipient, item.lastSeen).name}</div>}
      <div className="proposal-dates">{state === 'waiting'
        ? <span className="pill accent">{live && <i className="live-dot" aria-hidden="true" />}In the queue · {daysText(days(since, at))}</span>
        : state === 'gone' ? <span>Stored {dayLabel(since)} → gone from the queue {item.goneByBound ? 'by ' : ''}{dayLabel(item.goneBy!)} · {daysText(days(since, item.goneBy!))}</span>
          : <span>Stored {dayLabel(since)} · last in the queue {dayLabel(item.lastSeen)}; not recorded since</span>}</div>
    </article>;
  };
  const proposalTip = (row: LogRow<Proposal>) => {
    const item = row.item;
    return [{ title: 'Diplomatic proposal', subtitle: dayLabel(at), rows: [
      { label: 'From', value: country(item.sender, item.lastSeen).name }, { label: 'To', value: country(item.recipient, item.lastSeen).name },
      { label: 'Queue of', value: country(item.owner, item.lastSeen).name },
      ...(item.originalSender !== item.sender ? [{ label: 'Original sender', value: country(item.originalSender, item.lastSeen).name }] : []),
      ...(item.originalRecipient !== item.recipient ? [{ label: 'Original recipient', value: country(item.originalRecipient, item.lastSeen).name }] : []),
      { label: 'Stored', value: item.storedHour == null ? 'Not recorded' : dayLabel(item.storedHour) },
      { label: 'Last command', value: item.lastCommandHour == null ? 'Not recorded' : dayLabel(item.lastCommandHour) },
      { label: 'First in the queue', value: dayLabel(item.firstSeen) },
      { label: 'Gone from the queue', value: proposalState(item, at) === 'gone' ? `${item.goneByBound ? 'by ' : ''}${dayLabel(item.goneBy!)}`
        : proposalState(item, at) === 'waiting' ? 'Not yet' : 'Not recorded' },
    ] }];
  };
  const waitingCount = proposalRows.filter(row => proposalState(row.item, at) === 'waiting').length;
  const presentCount = pactRows.filter(pact => stateOf(pact).state === 'present').length;
  const header = headerSlot && <>{setOption && createPortal(<Segmented value={view} options={VIEWS} label="Diplomacy view" onChange={value => setOption('view', value)} />, headerSlot)}
    {createPortal(<span className="pill">{view === 'pacts' ? `${presentCount} present` : `${waitingCount} waiting`}</span>, headerSlot)}</>;
  if (!track) return <WidgetState kind="empty" icon="link" message="Load a campaign to see diplomacy." />;
  if (!window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (resolved?.reason) return <>{header}<WidgetState kind="empty" message={resolved.reason} /></>;
  const reply = view === 'pacts' ? pacts : proposals;
  if (reply.error) return <>{header}<WidgetState kind="error" detail={reply.error} onRetry={reply.retry} /></>;
  if (!reply.data) return <>{header}<WidgetState kind="loading" label="Loading diplomacy…" /></>;
  const pending = reply.data.pending.length > 0;
  if ((view === 'pacts' ? !pactRows.length : !proposalRows.length) && pendingAt(preparedPrefix(reply.data), at))
    return <>{header}<WidgetState kind="loading" label="This range is still being prepared." /></>;
  const log = logSettings(options);
  // The countries a group chooses are its members over the whole branch: none matching is not
  // proof of none while some of it is still being prepared.
  const known = !resolved || resolved.known(track.branch.firstHour, track.branch.lastHour);
  return <div className="diplomatic-pacts">{header}
    {pending && <p className="chart-line-key" role="status">Some of this campaign is still being prepared.</p>}
    {view === 'pacts'
      ? pactRows.length ? <Table key={JSON.stringify([track.source.id, track.branch.id])} label="Diplomatic pacts" rows={pactRows} columns={columns} rowKey={pactKey}
        sort={tableSortOf(options.tableSort, DEFAULT_SORT)} setOption={setOption} cellBars={false} rankMarkers={false}
        density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)} hoursPerSecond={hoursPerSecond}
        muted={pact => stateOf(pact).state !== 'present'} tooltip={pactTip}
        onVisibleRows={rows => report(rows.flatMap(pact => [pact.first, pact.second].flatMap(tag => tag ? [{ last: pact.lastSeen, tag }] : [])))}
        menu={pact => [{ ...jumpItem(runtime, track.id, pactSince(pact)), label: `Jump to since ${dayLabel(pactSince(pact))}` },
          ...(stateOf(pact).state === 'gone' ? [{ ...jumpItem(runtime, track.id, untilHour(stateOf(pact), pact)), label: `Jump to until ${dayLabel(untilHour(stateOf(pact), pact))}` }] : [])]} />
        : known ? <WidgetState kind="empty" icon="link" message="No pacts match these settings." /> : <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />
      : <LogList key={JSON.stringify([track.source.id, track.branch.id])} rows={proposalRows} label="Diplomatic proposals" layout="card" density={log.density} months={log.months}
        animate={live && log.animate} rowHeight={(row, density) => (density === 'compact' ? 64 : 76) + (row.item.originalSender !== row.item.sender || row.item.originalRecipient !== row.item.recipient ? 20 : 0)}
        renderRow={renderCard} tooltip={proposalTip}
        describe={row => `Proposal from ${country(row.item.sender, row.item.lastSeen).name} to ${country(row.item.recipient, row.item.lastSeen).name}, ${{ waiting: 'in the queue', gone: 'gone from the queue', unknown: 'not recorded since it was last in the queue' }[proposalState(row.item, at)]}`}
        menu={row => [{ ...jumpItem(runtime, track.id, proposalHour(row.item)), label: `Jump to stored ${dayLabel(proposalHour(row.item))}` },
          ...(row.item.goneBy != null && row.item.goneBy <= at ? [{ ...jumpItem(runtime, track.id, row.item.goneBy), label: `Jump to gone ${dayLabel(row.item.goneBy)}` }] : [])]}
        onVisible={rows => report(rows.flatMap(row => [row.item.sender, row.item.recipient, row.item.owner, row.item.originalSender, row.item.originalRecipient]
          .flatMap(tag => tag ? [{ last: row.item.lastSeen, tag }] : [])))}
        empty={!known ? MEMBERSHIP_PREPARING : options.waiting === true ? 'No proposals waiting.' : 'No proposals yet.'} />}
  </div>;
}
