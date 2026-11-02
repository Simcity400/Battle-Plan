import { nameReadText } from '../../dashboard/nameReads.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { usNumber } from '../../charts/numberFormat';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import { countryNameDetails, fitTooltip } from '../../charts/tooltip.ts';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { contentTitleHint } from '../../dashboard/contentTitles';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch, MEMBERSHIP_PREPARING } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { POLITICS_HISTORY, preparedThroughHour, tagAt, unknownHours, valueAt } from '../../dashboard/politicsHistory';
import { preparedPrefix } from '../../dashboard/preparedPrefix';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { PoliticsHistoryReply } from '../../generated/dashboard';
import BalanceCountryView from './BalanceCountryView';
import { BALANCE_UNAVAILABLE, balanceRowsAt, chooseBalance, exactValue, nameWarnings, pendingSpans, RANGE_UNAVAILABLE, rangeName, ROW_VALUES, rowHolder,
  rowStartUnseen, SIDE_UNAVAILABLE, sideName, validateHolderBalances } from './balanceOfPowerData.ts';
import type { BalanceRow } from './balanceOfPowerData';
import { DEFAULT_SORT, OPTIONS } from './definition';

const NO_ROWS: readonly BalanceRow[] = [];
const rowKey = (row: BalanceRow) => row.key;

/** The table's columns: members by `label` (sorted by the first's `sortName`), ranges and
 * sides by `rangeOf` and `sideOf`, the hiding columns' values in the row tooltip. */
export function balanceColumns(sortName: (tag: string) => string, label: CountryLabel, rangeOf: (row: BalanceRow) => string | null,
  sideOf: (row: BalanceRow) => string | null): TableColumn<BalanceRow>[] {
  return [
    { key: 'balance', header: 'Balance', label: 'Balance', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.name ?? BALANCE_UNAVAILABLE,
      render: row => <span data-tooltip={contentTitleHint(row.token)}>{row.name ?? BALANCE_UNAVAILABLE}</span> },
    { key: 'members', header: 'Members', label: 'Members', width: 'minmax(var(--table-name-min), 2fr)', priority: 1, inTooltip: true,
      value: row => sortName(row.members[0].tag), text: row => row.members.map(member => label(member.tag).name).join(', '),
      render: row => <span className="balance-members">{row.members.map(member => <CountryCell key={member.holder} {...label(member.tag)} />)}</span> },
    { key: 'range', header: 'Range', label: 'Range', width: 'minmax(var(--table-name-min), 1fr)', priority: 2, inTooltip: true, value: rangeOf,
      render: row => <span data-tooltip={contentTitleHint(row.range)}>{rangeOf(row)}</span> },
    { key: 'value', header: 'Value', label: 'Value', numeric: true, width: 'max-content', priority: 4, inTooltip: true, value: ROW_VALUES.value, render: row => exactValue(row.value) },
    { key: 'trending', header: 'Leaning toward', label: 'Leaning toward', width: 'minmax(var(--table-name-min), 1fr)', priority: 3, inTooltip: true, value: sideOf,
      render: row => <span data-tooltip={contentTitleHint(row.trending)}>{sideOf(row)}</span> },
  ];
}

/** Every balance active at the playback date, for all countries, a group or several
 * chosen countries; a row opens that balance's one-country view in the widget. */
export default function BalanceTable({ widget, runtime, active, headerSlot, setOption, onDiagnostics }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const following = widget.time.mode === 'follow';
  const playbackHour = useTableHour(runtime.clock, active && following);
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const anchor = useRef<HTMLDivElement>(null);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const pinned = widget.time.mode === 'pin';
  const playhead = window && (following || pinned) ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const hour = Math.floor(playhead ?? extent[1]);
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const limit = Math.floor(shownUntil(playhead, playback, options.afterPlayhead === true) ?? extent[1]);
  const inside = !!track && !!window && track.branch.firstHour <= hour && hour <= track.branch.lastHour;

  const needsGroups = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const selected = selectedTags(widget.countries, track);
  const selectedKey = JSON.stringify(selected);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const fallback = useMemo(() => selected, [selectedKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? hour),
    [widget.countries, fallback, groups.data, track?.branch.lastHour, hour]);
  // Every holder's balances once per branch; playback only looks them up locally.
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: POLITICS_HISTORY,
    params: { branchId: track.branch.id, mode: 'balances' } } : null;
  const { data, error, retry } = useProduct<PoliticsHistoryReply>(runtime.client, request, active, validateHolderBalances, track?.source);
  const prepared = !!data && preparedThroughHour(data, hour);
  const view = useMemo(() => data && prepared && match.ready && !match.reason ? balanceRowsAt(data, hour, match.matches) : null, [data, prepared, hour, match]);
  const rows = view?.rows ?? NO_ROWS;

  // The one-country view a row opens, held for this recording and branch only.
  const owner = useMemo(() => ({}), [runtime.client, track?.source.id, track?.source.revision, track?.branch.id]); // eslint-disable-line react-hooks/exhaustive-deps
  const [opened, setOpened] = useState<{ owner: object; holder: string; tag: string; token: string } | null>(null);
  const open = opened?.owner === owner ? opened : null;
  const openRow = useCallback((row: BalanceRow) => {
    const member = rowHolder(row, hour, match.matches);
    setOpened({ owner, holder: member.holder, tag: member.tag, token: row.token });
  }, [owner, hour, match]);

  const [visible, setVisible] = useState<readonly BalanceRow[]>(NO_ROWS);
  const onVisibleRows = useCallback((shown: readonly BalanceRow[]) => setVisible(shown), []);
  const openHolder = useMemo(() => open && data ? data.holderBalances.find(entry => entry.holder === open.holder) ?? null : null, [open, data]);
  const openTag = openHolder ? tagAt(openHolder.tags, hour) ?? open!.tag : null;
  // Members of every balance the opened country holds at the hour: the holders sharing its record.
  const openMembers = useMemo(() => {
    const byToken = new Map<string, string[]>();
    if (!openHolder || !data) return byToken;
    for (const row of balanceRowsAt(data, hour, () => true)?.rows ?? NO_ROWS)
      if (row.members.some(member => member.holder === openHolder.holder)) byToken.set(row.token, row.members.map(member => member.tag));
    return byToken;
  }, [openHolder, data, hour]);
  const wanted = useMemo(() => [...visible.flatMap(row => row.members.map(member => ({ hour, tag: member.tag }))),
    ...(openTag ? [{ hour, tag: openTag }] : []), ...[...openMembers.values()].flat().map(tag => ({ hour, tag }))], [visible, hour, openTag, openMembers]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  // A Members sort needs every row's first name; flags still load only for mounted rows.
  const sortingNames = useMemo(() => sort.column === 'members' ? rows.map(row => ({ hour, tag: row.members[0].tag })) : [], [rows, sort.column, hour]);
  const countryNames = useCountryNamesAt(runtime.client, track, sortingNames, active);
  const names = data?.names;
  const rangeOf = useCallback((row: BalanceRow) => row.range === null || !names ? null : rangeName(row.balance, row.range, names) ?? RANGE_UNAVAILABLE, [names]);
  const sideOf = useCallback((row: BalanceRow) => row.trending === null || !names ? null : sideName(row.balance, row.trending, names) ?? SIDE_UNAVAILABLE, [names]);
  const columns = useMemo(() => balanceColumns(tag => nameReadText(countryNames.get(nameKey(hour, tag))), labelAt(hour), rangeOf, sideOf),
    [countryNames, labelAt, hour, rangeOf, sideOf]);
  const pending = useMemo(() => track && data ? pendingSpans(preparedPrefix(data)?.pending ?? [], data.lastHour, track.branch) : [], [track, data]);
  const unknown = useMemo(() => track ? unknownHours(track.branch.gaps, data?.known ?? [], track.branch.firstHour, data?.lastHour ?? null, pending) : [],
    [track, data, pending]);
  const tooltip = useCallback((row: BalanceRow) => fitTooltip([{ title: row.name ?? BALANCE_UNAVAILABLE, ...countryNameDetails(...row.members.map(member => labelAt(hour)(member.tag))), rows: [
    { label: 'Members', value: row.members.map(member => labelAt(hour)(member.tag).name).join(', ') },
    ...row.range === null ? [] : [{ label: 'Range', value: rangeOf(row)!, lookupKey: row.range }],
    { label: 'Value', value: exactValue(row.value), strong: true },
    ...row.trending === null ? [] : [{ label: 'Leaning toward', value: sideOf(row)!, lookupKey: row.trending }],
    { label: track && rowStartUnseen(row, track.branch.firstHour, unknown) ? 'Held before' : 'Active since', value: dayLabel(row.since) },
  ] }], viewport), [labelAt, hour, rangeOf, sideOf, viewport, track, unknown]);
  const menu = useCallback((row: BalanceRow) => [{ label: 'Show this balance', run: () => openRow(row) },
    ...track ? [jumpItem(runtime, track.id, row.since)] : []], [openRow, track, runtime]);

  const warnings = useMemo(() => [...(data?.warnings ?? []),
    ...(data ? nameWarnings(rows.map(row => row.balance), data.names) : []),
    ...(view?.untagged ? [`${view.untagged} balance member${view.untagged === 1 ? '' : 's'} with no tag at this hour`] : []),
    ...(groups.error && needsGroups ? [`Country membership could not be loaded: ${groups.error.message}`] : [])], [data, rows, view, groups.error, needsGroups]);
  useDiagnostics(onDiagnostics, warnings);
  useEffect(() => { if (open && !openHolder && data) setOpened(null); }, [open, openHolder, data]);

  let body;
  if (!track) body = <WidgetState kind="empty" icon="sliders" message="Load a campaign to see the balance of power." />;
  else if (!window || !inside) body = <WidgetState kind="unavailable" message="This time is outside the recording." />;
  else if (error && !data) body = <WidgetState kind="error" failure={error} onRetry={retry} />;
  else if (needsGroups && groups.error && !groups.data) body = <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
  else if (match.reason) body = <WidgetState kind="unavailable" message={match.reason} />;
  else if (!data || !match.ready) body = <WidgetState kind="loading" label="Loading the balance of power…" />;
  else if (!prepared) body = <WidgetState kind="loading" label="This range is still being prepared." />;
  else if (!view) body = <WidgetState kind="unavailable" message="The balance of power was not recorded at this date." />;
  else if (!rows.length) body = match.known(hour) ? <WidgetState kind="empty" icon="sliders" message="No balance of power is active at this date." />
    : <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
  else body = <Table rowRevision={row => [row, columns]} key={`${track.source.id}:${track.branch.id}`} label="Balances of power" rows={rows} columns={columns} rowKey={rowKey}
    sort={sort} setOption={setOption} cellBars={options.cellBars === true} rank rankMarkers={options.rankMarkers === true}
    density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)} rowHeight={2 * (tokens.fontSize.title + tokens.fontSize.small)}
    onRowClick={openRow} menu={menu} tooltip={tooltip} onVisibleRows={onVisibleRows} hoursPerSecond={hoursPerSecond} />;

  const choice = open && openHolder ? chooseBalance(openHolder, open.token, hour, limit) : null;
  const countryName = openTag ? labelAt(hour)(openTag).name : '';
  const openName = choice?.balance ? choice.balance.name ?? data?.names[choice.balance.token] ?? BALANCE_UNAVAILABLE : '';
  return <div className="balance-of-power" ref={anchor}>
    {headerSlot && view && createPortal(<span className="pill balance-count">{usNumber.format(rows.length)} active</span>, headerSlot)}
    {needsGroups && groups.error && groups.data ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null}
    <DrillHost mode="inline" open={!!choice?.balance} anchor={anchor} title={`${countryName}: ${openName}`} day={dayLabel(hour)} following={false} canFollow={false}
      onClose={() => setOpened(null)} onFollow={() => {}} main={body}>
      {track && data && openHolder && choice?.balance && <BalanceCountryView runtime={runtime} track={track} active={active} subject={openHolder} names={data.names}
        balance={choice.balance} gameShown={hour <= limit ? valueAt(openHolder.shown, hour) : null} countryName={countryName} at={hour} limit={limit}
        extent={extent} playhead={playhead} playback={{ mode: playback, afterPlayhead: options.afterPlayhead === true, windowHours: Number(options.windowDays ?? 365) * 24 }}
        unknown={unknown} pending={pending}
        membersOf={token => openMembers.get(token)?.map(tag => labelAt(hour)(tag).name) ?? null}
        pickOwner={open} onChoose={token => setOpened(before => before && { ...before, token })} />}
    </DrillHost>
  </div>;
}
