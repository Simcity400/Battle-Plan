import { countryFromKey } from '../../dashboard/countryKey.ts';
import { nameReadText } from '../../dashboard/nameReads.ts';
import { useCallback, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { useDrill } from '../../charts/drill/useDrill';
import { dayLabel } from '../../charts/draw';
import { usNumber } from '../../charts/numberFormat';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { countryNameDetails, fitTooltip } from '../../charts/tooltip.ts';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { contentTitleHint } from '../../dashboard/contentTitles';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch, countryMatchChangeHours, membershipUnrecordedAt } from '../../dashboard/groups';
import { useChoiceHour } from '../../dashboard/atHead';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { POLITICS_HISTORY, tagAt, unknownHours, valueAt } from '../../dashboard/politicsHistory';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import BalanceCountryView from './BalanceCountryView';
import { BALANCE_UNAVAILABLE, balanceRows, balanceStructure, balanceTableValues, chooseBalance, countryHolder, exactValue, memberSince, nameWarnings, RANGE_UNAVAILABLE,
  recordValue, rowHolder, SIDE_UNAVAILABLE, validateBalanceList, validateCountryBalances, validateMemberships } from './balanceOfPowerData.ts';
import { jumpItem } from '../../charts/jump';
import { startUnseen } from '../../charts/timeline/runState';
import { liveKey, mapLive } from '../../charts/live';
import { reporting } from '../../dashboard/follow/liveCells.ts';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import type { LiveValues } from '../../charts/live';
import type { BalanceRow } from './balanceOfPowerData';
import { DEFAULT_SORT, OPTIONS } from './definition';
import type { PoliticsHistoryReply } from '../../generated/dashboard';
import { useRegion } from '../../dashboard/widgetTime.ts';
import { productRequest } from '../../dashboard/request.ts';

const NO_ROWS: readonly BalanceRow[] = [];
const PARAMS = { view: 'balances' } as const;
const rowKey = (row: BalanceRow) => row.key;
const rangeOf = (row: BalanceRow) => row.range === null ? null : row.range.name ?? RANGE_UNAVAILABLE;
const sideOf = (row: BalanceRow) => row.trending === null ? null : row.trending.name ?? SIDE_UNAVAILABLE;

const valueOf = (values: LiveValues<number>, key: string) => {
  const raw = recordValue(values, { ordinal: Number(key) });
  return raw === null ? values.missing(liveKey(key, 'value')) : exactValue(raw);
};

export function balanceColumns(sortName: (tag: string) => string, label: CountryLabel, values: () => LiveValues<number>): TableColumn<BalanceRow>[] {
  return [
    { key: 'balance', header: 'Balance', label: 'Balance', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.name ?? BALANCE_UNAVAILABLE,
      render: row => <span data-tooltip={contentTitleHint(row.token)}>{row.name ?? BALANCE_UNAVAILABLE}</span> },
    { key: 'members', header: 'Members', label: 'Members', width: 'minmax(var(--table-name-min), 2fr)', priority: 1, inTooltip: true,
      value: row => sortName(row.members[0].tag), text: row => row.members.map(member => label(member.tag).name).join(', '),
      render: row => <span className="balance-members">{row.members.map(member => <CountryCell key={member.tag} {...label(member.tag)} />)}</span> },
    { key: 'range', header: 'Range', label: 'Range', width: 'minmax(var(--table-name-min), 1fr)', priority: 2, inTooltip: true, value: rangeOf,
      render: row => <span data-tooltip={contentTitleHint(row.range?.token ?? null)}>{rangeOf(row)}</span> },
    { key: 'value', header: 'Value', label: 'Value', numeric: true, width: 'max-content', priority: 4, inTooltip: true, value: () => null,
      paint: (_, key) => valueOf(values(), key.slice(0, key.lastIndexOf('\u0000'))) },
    { key: 'trending', header: 'Leaning toward', label: 'Leaning toward', width: 'minmax(var(--table-name-min), 1fr)', priority: 3, inTooltip: true, value: sideOf,
      render: row => <span data-tooltip={contentTitleHint(row.trending?.token ?? null)}>{sideOf(row)}</span> },
  ];
}

export default function BalanceTable({ widget, runtime, active, headerSlot, setOption, onDiagnostics }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const anchor = useRef<HTMLDivElement>(null);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const head = useHeadProduct(runtime, useGate(runtime, widget), 'politics-state', track ? PARAMS : null, { active, validate: validateBalanceList, structure: balanceStructure });
  const headRoot = head.root;
  const rootRef = useCallback((element: HTMLDivElement | null) => { anchor.current = element; headRoot(element); }, [headRoot]);
  const data = useMemo(() => head.data && head.at !== null ? { ...head.data, hour: head.at } : undefined, [head.data, head.at]);
  const hour = data?.hour ?? track?.branch.lastHour ?? 0;
  const mainRegion = useRegion(widget);
  const itemDrill = useDrill(runtime, mainRegion);
  const live = head.live;
  const shown = useShownHour();
  const shownAt = useRef(head.shownAt);
  shownAt.current = head.shownAt;
  const tableLive = useMemo(() => mapLive(reporting(live, () => shown(shownAt.current())), balanceTableValues), [live, shown]);
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = following && data ? hour : null;
  const extent: [number, number] = track ? [track.branch.firstHour, track.branch.lastHour] : [0, 1];
  const limit = Math.floor(shownUntil(playhead, playback, options.afterPlayhead === true) ?? extent[1]);
  const afterPlayhead = options.afterPlayhead === true;
  const limitAt = useCallback((at: number) => Math.floor(shownUntil(at, playback, afterPlayhead) ?? extent[1]), [playback, afterPlayhead, extent[1]]);
  const axisOffset = track?.axisOffset ?? 0;
  const liveHead = useMemo(() => widget.time.mode === 'follow' && track ? { read: () => runtime.clock.getSnapshot().position + axisOffset,
    subscribe: (changed: () => void) => runtime.clock.subscribeFrame(changed) } : undefined, [widget.time.mode, track, runtime.clock, axisOffset]);

  const needsGroups = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const selected = selectedCountryKeys(widget.countries, track);
  const selectedKey = JSON.stringify(selected);
  const fallback = useMemo(() => selected, [selectedKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? hour),
    [widget.countries, fallback, groups.data, track?.branch.lastHour, hour]);
  const unread = !!data && data.unavailable.some(entry => entry.label === 'balances of power');
  const unrecorded = useCallback((at: number) => membershipUnrecordedAt(widget.countries, groups.data, at), [widget.countries, groups.data]);
  const choiceAt = useChoiceHour(runtime.clock, active, () => head.atNow() ?? hour, () => head.shownAt(), countryMatchChangeHours(match.matches), head.live.subscribe);
  const view = useMemo(() => data && !unread && match.ready && !match.reason ? balanceRows({ balances: data.balances, hour: choiceAt }, match.matches, unrecorded) : null,
    [data, unread, match, unrecorded, choiceAt]);
  const rows = view?.rows ?? NO_ROWS;

  const owner = useMemo(() => ({}), [runtime.client, track?.source.id, track?.source.revision, track?.branch.id]);
  const [opened, setOpened] = useState<{ owner: object; tag: string; token: string } | null>(null);
  const open = opened?.owner === owner ? opened : null;
  const openRow = useCallback((row: BalanceRow) => {
    const member = rowHolder(row, choiceAt, match.matches);
    setOpened({ owner, tag: member.tag, token: row.token });
    itemDrill.attach(row.token);
  }, [owner, choiceAt, match]);
  const request = track && open ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: POLITICS_HISTORY,
    params: { branchId: track.branch.id, mode: 'country', country: countryFromKey(open.tag) } }) : null;
  const history = useProduct<PoliticsHistoryReply>(runtime.client, request, active, validateCountryBalances, track?.source);
  const membershipRequest = track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: POLITICS_HISTORY,
    params: { branchId: track.branch.id, mode: 'memberships' } }) : null;
  const memberships = useProduct<PoliticsHistoryReply>(runtime.client, membershipRequest, active, validateMemberships, track?.source);
  const openHolder = useMemo(() => open && history.data ? countryHolder(history.data, open.tag, hour) : null, [open, history.data, hour]);
  const openTag = openHolder ? tagAt(openHolder.tags, hour) ?? open!.tag : null;
  const openMembers = useMemo(() => {
    const byToken = new Map<string, string[]>();
    if (!openTag || !data) return byToken;
    for (const row of balanceRows(data, () => true).rows)
      if (row.members.some(member => member.tag === openTag)) byToken.set(row.token, row.members.map(member => member.tag));
    return byToken;
  }, [openTag, data]);

  const [visible, setVisible] = useState<readonly BalanceRow[]>(NO_ROWS);
  const onVisibleRows = useCallback((shown: readonly BalanceRow[]) => setVisible(shown), []);
  const wanted = useMemo(() => [...visible.flatMap(row => row.members.map(member => ({ hour, tag: member.tag }))),
    ...(openTag ? [{ hour, tag: openTag }] : []), ...[...openMembers.values()].flat().map(tag => ({ hour, tag }))], [visible, hour, openTag, openMembers]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const sortingNames = useMemo(() => sort.column === 'members' ? rows.map(row => ({ hour, tag: row.members[0].tag })) : [], [rows, sort.column, hour]);
  const countryNames = useCountryNamesAt(runtime.client, track, sortingNames, active);
  const columns = useMemo(() => balanceColumns(tag => nameReadText(countryNames.get(nameKey(hour, tag))), labelAt(hour), live.read), [countryNames, labelAt, hour, live]);
  const known = history.data ?? memberships.data;
  const unknown = useMemo(() => track ? unknownHours(track.branch.gaps, known?.known ?? [], track.branch.firstHour, known?.lastHour ?? null) : [],
    [track, known]);
  const sinceOf = useCallback((row: BalanceRow) => memberships.data ? memberSince(memberships.data, row.members[0].tag, row.token, hour) : undefined, [memberships.data, hour]);
  const tooltip = useCallback((row: BalanceRow) => fitTooltip([{ title: row.name ?? BALANCE_UNAVAILABLE, ...countryNameDetails(...row.members.map(member => labelAt(hour)(member.tag))), rows: [
    { label: 'Members', value: row.members.map(member => labelAt(hour)(member.tag).name).join(', ') },
    ...row.range === null ? [] : [{ label: 'Range', value: rangeOf(row)!, lookupKey: row.range.token }],
    { label: 'Value', value: valueOf(live.read(), row.key), strong: true },
    ...row.trending === null ? [] : [{ label: 'Leaning toward', value: sideOf(row)!, lookupKey: row.trending.token }],
    ...sinceRow(sinceOf(row)),
  ] }], viewport), [labelAt, hour, viewport, live, sinceOf]);
  function sinceRow(since: number | null | undefined) {
    if (since === undefined) return [{ label: 'Active since', value: memberships.error ? 'Not loaded' : 'Loading…', tone: 'muted' as const }];
    if (since === null || !track) return [];
    return [{ label: startUnseen(since, track.branch.firstHour, unknown) ? 'Held before' : 'Active since', value: dayLabel(since) }];
  }
  const menu = useCallback((row: BalanceRow) => {
    const since = sinceOf(row);
    return [{ label: 'Show this balance', run: () => openRow(row) }, ...track && since != null ? [jumpItem(runtime, track.id, since)] : []];
  }, [openRow, sinceOf, track, runtime]);

  const hourRanges = useMemo(() => rows.flatMap(row => row.range && row.range.name === null ? [`${row.token}:${row.range.token}`] : []), [rows]);
  const warnings = useMemo(() => [...(data?.warnings ?? []),
    ...(data ? nameWarnings(rows.map(row => ({ token: row.token, name: row.name })), {}, hourRanges) : []),
    ...(history.error ? [`Balance of power history could not be loaded: ${history.error.message}`] : []),
    ...(memberships.error ? [`Balance of power memberships could not be loaded: ${memberships.error.message}`] : []),
    ...(view?.untagged ? [`${view.untagged} balance member${view.untagged === 1 ? '' : 's'} with no tag at this hour`] : []),
    ...(groups.error && needsGroups ? [`Country membership could not be loaded: ${groups.error.message}`] : [])], [data, rows, hourRanges, history.error, memberships.error, view, groups.error, needsGroups]);
  useDiagnostics(onDiagnostics, warnings);

  let body;
  if (!track) body = <WidgetState kind="empty" icon="sliders" message="Load a campaign to see the balance of power." />;
  else if (head.read === 'off') body = <WidgetState kind="outside" />;
  else if (head.error && !data) body = <WidgetState kind="error" failure={head.error} onRetry={head.retry} />;
  else if (needsGroups && groups.error && !groups.data) body = <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
  else if (match.reason) body = <WidgetState kind="unavailable" message={match.reason} />;
  else if (!data || !match.ready) body = <WidgetState kind="loading" label="Loading the balance of power…" />;
  else if (unread) body = <WidgetState kind="unavailable" message="The balance of power was not recorded at this date." />;
  else if (!rows.length && view?.membership) body = <WidgetState kind="unavailable" message={view.membership} />;
  else if (!rows.length) body = <WidgetState kind="empty" icon="sliders" message="No balance of power is active at this date." />;
  else body = <Table rowRevision={row => [row, columns]} key={`${track.source.id}:${track.branch.id}`} label="Balances of power" rows={rows} columns={columns} rowKey={rowKey}
    sort={sort} setOption={setOption} cellBars={options.cellBars === true} rank rankMarkers={options.rankMarkers === true}
    density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)} rowHeight={2 * (tokens.fontSize.title + tokens.fontSize.small)}
    onRowClick={openRow} menu={menu} tooltip={tooltip} onVisibleRows={onVisibleRows} hoursPerSecond={hoursPerSecond} live={tableLive} />;

  const choice = open && openHolder ? chooseBalance(openHolder, open.token, hour, limit) : null;
  const countryName = openTag ? labelAt(hour)(openTag).name : '';
  const openName = choice?.balance ? choice.balance.name ?? history.data?.names[choice.balance.token] ?? BALANCE_UNAVAILABLE : '';
  return <div className="balance-of-power" ref={rootRef}>
    {headerSlot && view && createPortal(<span className="pill balance-count">{usNumber.format(rows.length)} active</span>, headerSlot)}
    {needsGroups && groups.error && groups.data ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null}
    <DrillHost drill={itemDrill} runtime={runtime} subject={!!choice?.balance} mode="inline" anchor={anchor} title={`${countryName}: ${openName}`} day={data ? dayLabel(data.hour) : ''} canFollow={false}
      onClose={() => setOpened(null)} main={body}>
      {track && history.data && openHolder && choice?.balance && <BalanceCountryView runtime={runtime} track={track} active={active} subject={openHolder} names={history.data.names}
        balance={choice.balance} liveHead={liveHead} limitAt={limitAt} gameShown={hour <= limit ? valueAt(openHolder.shown, hour) : null} countryName={countryName} at={hour} limit={limit}
        extent={extent} playhead={playhead} playback={{ mode: playback, afterPlayhead: options.afterPlayhead === true, windowHours: Number(options.windowDays ?? 365) * 24 }}
        unknown={unknown}
        membersOf={token => openMembers.get(token)?.map(tag => labelAt(hour)(tag).name) ?? null}
        pickOwner={open} onChoose={token => setOpened(before => before && { ...before, token })} />}
    </DrillHost>
  </div>;
}
