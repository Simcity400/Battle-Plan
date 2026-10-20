import { useCallback, useMemo, useReducer, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import { Segmented } from '../../components/Segmented';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, reduceDrill } from '../../charts/drill/drillState';
import StackedShareChart from '../../charts/StackedShareChart';
import type { ShareBand } from '../../charts/StackedShareChart';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/columns';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { fitTooltip } from '../../charts/tooltip';
import type { TooltipRow } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { formatAmount, formatFixed } from '../../charts/valueAxis';
import type { PlaybackMode } from '../../charts/view';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryNamesRead } from '../../dashboard/countryNamesAt';
import { nameReadText, useHolderLooks } from '../../dashboard/holderLooks';
import type { WidgetProps } from '../../dashboard/definitions';
import { factionNameAt, noFactionMessage, useChosenFaction, useFactionDetails } from '../../dashboard/factionChoice';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackSpeed } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { FactionAccountDetail, FactionInfluenceReply } from '../../generated/dashboard';
import { DEFAULT_SORT, OPTIONS, VIEWS } from './definition';
import { SHARE_SCALE, accountHolders, influenceShareRuns, memberAccounts, memberShareRuns, rankingRows } from './influenceData';
import './factionInfluence.css';

const NO_WARNINGS: string[] = [];
const share = (raw: string | null) => raw === null ? 'Not recorded' : `${formatFixed(raw, SHARE_SCALE, 2)}%`;
const rowKey = (row: FactionAccountDetail) => `${row.ordinal}`;

export default function FactionInfluenceWidget({ widget, runtime, active, headerSlot, setOption, onDiagnostics }: WidgetProps) {
  const playback = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const options = resolveOptions(OPTIONS, widget.options);
  const view = options.view === 'ranking' ? 'ranking' : 'time';
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const { groups, pick } = useChosenFaction(widget, runtime, track, hour, active);
  const faction = pick?.faction ?? null;
  const details = useFactionDetails(runtime, track, view === 'ranking' && faction ? faction.id : null, hour, active);
  const history = useProduct<FactionInfluenceReply>(runtime.client, track && faction ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'faction-state', params: { view: 'influence', branchId: track.branch.id, faction: faction.id } } : null, active, undefined, track?.source);
  const accounts = useMemo(() => history.data ? memberAccounts(history.data) : [], [history.data]);
  const runs = useMemo(() => influenceShareRuns(accounts), [accounts]);
  const ranking = useMemo(() => details.data ? rankingRows(details.data) : [], [details.data]);
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  const anchor = useRef<HTMLDivElement>(null);
  const picked = drill.breakdown ? accounts.find(account => account.key === drill.breakdown) ?? null : null;
  // Each account is its own native holder: its band, swatch and drill are named and coloured
  // as that holder, never as whoever holds its tag now. Every account's look is read only
  // for the bands over time; the ranking reads only an opened drill's holder.
  const holders = useHolderLooks(runtime.client, track, useMemo(() => accountHolders(view === 'time' ? accounts : picked ? [picked] : []), [view, accounts, picked]), active, tokens);
  const rankingTags = useMemo(() => ranking.flatMap(row => row.tag ? [row.tag] : []), [ranking]);
  const [hovered, setHovered] = useState<number | null>(null);
  // Ranked members are named at the hour shown, hovered members at the hour hovered: both
  // hours their tags name them.
  const named = useMemo(() => [...(view === 'ranking' ? rankingTags.map(tag => ({ hour, tag })) : []),
    ...(hovered === null ? [] : accounts.filter(account => account.member.some(span => span.firstHour <= hovered && hovered <= span.lastHour)).map(account => ({ hour: hovered, tag: account.tag })))],
  [view, rankingTags, accounts, hour, hovered]);
  const names = useCountryNamesRead(runtime.client, track, named, active);
  const [visibleRows, setVisibleRows] = useState<readonly string[]>([]);
  // Flags only for the ranking rows the mounted table shows.
  const ranked = view === 'ranking' && ranking.length > 0;
  const flagged = useCountryLabelsAt(runtime.client, track, useMemo(() => ranked ? visibleRows.map(tag => ({ hour, tag })) : [], [ranked, visibleRows, hour]), active);
  const labelAt = useCallback((at: number) => (tag: string) => ({ ...flagged.labelAt(at)(tag), name: nameReadText(names.get(nameKey(at, tag))) }), [flagged, names]);
  const accountOf = useCallback((tag: string | null) => accounts.find(item => item.tag === tag && item.member.some(span => span.firstHour <= hour && hour <= span.lastHour)), [accounts, hour]);
  const bands = useMemo<ShareBand[]>(() => [...accounts.map(account => ({ id: account.key, label: holders.name(account.key), color: holders.colour(account.key) })),
    { id: 'other', label: 'Other accounts', color: tokens.grid }], [accounts, holders, tokens.grid]);
  const openMember = useCallback((key: string) => dispatch({ type: 'open', breakdown: key, hour, playing: false, playhead: null }), [hour]);
  const diagnostics = useMemo(() => [...(history.data?.warnings ?? []), ...(details.data?.warnings ?? []),
    ...(history.error ? [`Influence history unavailable: ${history.error}`] : [])], [history.data, details.data, history.error]);
  useDiagnostics(onDiagnostics, diagnostics.length ? diagnostics : NO_WARNINGS);
  const columns = useMemo<readonly TableColumn<FactionAccountDetail>[]>(() => [
    { key: 'member', header: 'Member', label: 'Member', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.tag ? labelAt(hour)(row.tag).name : null,
      renderMissing: true, render: row => row.tag ? <CountryCell {...labelAt(hour)(row.tag)} /> : 'Country unavailable' },
    { key: 'influence', header: 'Influence', label: 'Influence share', numeric: true, width: 'max-content', bar: 100000n,
      value: row => BigInt(row.influenceShareRaw), render: row => share(row.influenceShareRaw) },
    { key: 'projection', header: 'Projection', label: 'Power projection share', numeric: true, width: 'max-content', priority: 2,
      value: row => BigInt(row.projectionShareRaw), render: row => share(row.projectionShareRaw) },
    { key: 'contribution', header: 'Contribution', label: 'Contribution', numeric: true, width: 'max-content', priority: 3,
      value: row => BigInt(row.contributionRaw), render: row => formatAmount(row.contributionRaw, 5) },
  ], [labelAt, hour]);
  const rankingTip = useCallback((row: FactionAccountDetail) => {
    const name = row.tag ? labelAt(hour)(row.tag) : { name: 'Country unavailable' };
    return fitTooltip([{ title: name.name, countryFlag: row.tag ? name : undefined, subtitle: dayLabel(hour), rows: [
      { label: 'Influence share', value: share(row.influenceShareRaw) },
      { label: 'Power projection share', value: share(row.projectionShareRaw) },
      { label: 'Contribution', value: formatAmount(row.contributionRaw, 5, 5) },
      { label: 'Initiative', value: formatAmount(row.initiativeRaw, 5, 5) },
    ] }], viewport);
  }, [labelAt, hour, viewport]);
  const chartTip = useCallback((at: number) => {
    const rows: TooltipRow[] = accounts.flatMap(account => {
      if (!account.member.some(span => span.firstHour <= at && at <= span.lastHour)) return [];
      const run = account.runs.find(item => item.firstHour <= at && at <= item.lastHour);
      return [{ label: nameReadText(names.get(nameKey(at, account.tag))), value: share(run?.shareRaw ?? null), swatch: holders.colour(account.key) }];
    });
    return fitTooltip([{ title: faction ? factionNameAt(faction, at) : 'Influence', subtitle: dayLabel(at), rows: rows.length ? rows : [{ label: 'Members', value: 'Not recorded' }] }], viewport);
  }, [accounts, names, holders, faction, viewport]);
  const views = setOption ? <Segmented value={view} options={VIEWS} label="Faction influence view" onChange={value => setOption('view', value)} /> : null;
  const header = headerSlot && <>{views && createPortal(views, headerSlot)}{faction && createPortal(<span className="pill">{factionNameAt(faction, hour)}</span>, headerSlot)}</>;
  if (!track) return <WidgetState kind="empty" icon="chart-bars" message="Load a campaign to see faction influence." />;
  if (!window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (groups.error) return <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} />;
  if (!groups.data) return <WidgetState kind="loading" label="Loading faction influence…" />;
  if (!faction) return <>{header}<WidgetState kind="empty" icon="chart-bars" message={noFactionMessage(pick!.reason as 'none' | 'unobserved' | 'unknown', widget.time.mode === 'follow')} /></>;
  let main;
  if (view === 'ranking') {
    main = details.error ? <WidgetState kind="error" detail={details.error} onRetry={details.retry} />
      : !details.data ? <WidgetState kind="loading" label="Loading faction influence…" />
        : details.data.accounts === null ? <WidgetState kind="unavailable" message="Member influence was not recorded at this time." />
          : !ranking.length ? <WidgetState kind="empty" icon="chart-bars" message="This faction has no members at this time." />
            : <Table key={JSON.stringify([track.source.id, track.branch.id, faction.id])} label={`${factionNameAt(faction, hour)} influence ranking`}
              rows={ranking} columns={columns} rowKey={rowKey} rank sort={tableSortOf(options.tableSort, DEFAULT_SORT)} setOption={setOption}
              cellBars={options.cellBars === true} rankMarkers={options.rankMarkers === true} density={options.density === 'compact' ? 'compact' : 'comfortable'}
              narrow={narrowTableOf(options)} tooltip={rankingTip} hoursPerSecond={hoursPerSecond}
              onRowClick={row => { const account = accountOf(row.tag); if (account) openMember(account.key); }}
              onVisibleRows={rows => setVisibleRows(previous => { const next = rows.flatMap(row => row.tag ? [row.tag] : []);
                return previous.length === next.length && previous.every((tag, index) => tag === next[index]) ? previous : next; })}
              menu={row => { const account = accountOf(row.tag); const joined = account?.member.filter(span => span.firstHour <= hour).at(-1)?.firstHour;
                return joined == null ? [] : [{ ...jumpItem(runtime, track.id, joined), label: `Jump to joining ${dayLabel(joined)}` }]; }} />;
  } else {
    const extent: [number, number] = [window.firstHour, Math.max(window.firstHour + 1, window.lastHour)];
    main = history.error ? <WidgetState kind="error" detail={history.error} onRetry={history.retry} />
      : !history.data ? <WidgetState kind="loading" label="Loading faction influence…" />
        : !runs.length ? <WidgetState kind="unavailable" message={history.data.pending.length ? 'This range is still being prepared.' : 'Member influence was not recorded for this faction.'} />
          : <StackedShareChart key={JSON.stringify([track.source.id, track.branch.id, faction.id])} label={`${factionNameAt(faction, hour)} influence over time`}
            runs={runs} bands={bands} extent={extent} playhead={widget.time.mode === 'follow' ? hour : null} playback={options.playback as PlaybackMode} afterPlayhead={options.afterPlayhead === true} windowHours={Number(options.windowDays) * 24}
            branchStart={track.branch.firstHour} hoursPerSecond={hoursPerSecond} active={active && (!picked || options.drill === 'popout')}
            revision={`${holders.revision}:${names.size}`} tooltip={at => chartTip(at)}
            onHover={at => setHovered(at === null ? null : Math.floor(at))}
            onContextMenu={undefined} />;
  }
  return <div ref={anchor} className="faction-influence">{header}
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={picked != null} title="Member influence" day={dayLabel(hour)}
      following={false} canFollow={false} anchor={anchor} onClose={() => dispatch({ type: 'close' })} onFollow={() => {}} main={main}>
      {picked && <StackedShareChart label={`${holders.name(picked.key)} share of influence`} runs={memberShareRuns(picked)}
        bands={[{ id: 'member', label: holders.name(picked.key), color: holders.colour(picked.key) }, { id: 'rest', label: 'Rest of the faction', color: tokens.grid }]}
        extent={[track.branch.firstHour, Math.max(track.branch.firstHour + 1, track.branch.lastHour)]} playhead={hour} playback="cursor" branchStart={track.branch.firstHour}
        hoursPerSecond={hoursPerSecond} active={active} revision={`${picked.key}:${holders.revision}`} />}
    </DrillHost>
  </div>;
}

