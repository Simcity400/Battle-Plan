import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { liveByColumn, liveKey, noValues } from '../../charts/live.ts';
import type { Live, LiveValues } from '../../charts/live';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import type { SortValue } from '../../charts/table/logic';
import { narrowTableOf, tableSortOf, tableSortToken } from '../../charts/table/options';
import { countryNameDetails, fitTooltip } from '../../charts/tooltip.ts';
import { useViewportHeight } from '../../charts/useChartTokens';
import { useIdentityHour } from '../../dashboard/atHead';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { looksRead } from '../../dashboard/countryLooks';
import { trackHourOf } from '../../dashboard/settledHour';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { followStore } from '../../dashboard/follow/connection.ts';
import { liveFollowed } from '../../dashboard/follow/liveFollowed.ts';
import { stickyLimit, useStickyMembers } from '../../dashboard/follow/stickyMembers.ts';
import { shownThrough } from '../../dashboard/follow/store.ts';
import type { FollowEntry, FollowQuery } from '../../dashboard/follow/store';
import { useFallbackHour } from '../../dashboard/follow/useFollow.ts';
import { noMembersAt, rankedCandidates, resolveCountries, worldFilter } from '../../dashboard/groups';
import NoMembersState from '../../dashboard/NoMembersState';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useFollowedProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useShownHour, useWhyCommit } from '../../dashboard/widgetPerf';
import { useHeadHour } from '../army-size/headHour.ts';
import { DEFAULT_SORT, OPTIONS } from './definition';
import { handedLive, useFollowedAll } from './followedAll.ts';
import type { HandedSources } from './followedAll';
import { useScoreboardColumns } from './scoreboardColumns.ts';
import { followedRow, followedValues, formatCell, isPartial, rankedTags, rankingColumn, rankingDescending, replyRows, scoreboardColumnNote, scoreboardParams,
  scoreboardSort, scoreboardTooltip, selectedColumns, validateScoreboardReply } from './scoreboardData.ts';
import type { BoardRow, Cell } from './scoreboardData';
import './scoreboard.css';
import type { ScoreboardColumnKey, ScoreboardFamily, ScoreboardReply } from '../../generated/dashboard';

const NO_WARNINGS: string[] = [];
const NO_ROWS: readonly BoardRow[] = [];
const rowKey = (row: BoardRow) => row.key;
const familyKey = (family: ScoreboardFamily) => `${family.kind}\u0000${family.measure}`;
const tagOf = (key: string) => key.slice(0, key.lastIndexOf('\u0000'));
const silent = (reason: string): Live<LiveValues<SortValue>> => { const values = noValues(reason); return { read: () => values, subscribe: () => () => {} }; };
const NOT_READ_YET = noValues('Not read yet');

/** Countries side by side across the measures chosen, at the playhead or a pinned hour. At the head the rows (a top N
 * followed by rank, or the chosen countries) are structure and every value is painted from followed series; at a pinned
 * hour, or where the service does not follow them, one `scoreboard` reply answers the hour. */
export default function ScoreboardWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const viewport = useViewportHeight();
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const lastHour = track?.branch.lastHour ?? 0;
  const registryResult = useScoreboardColumns(runtime, track, active);
  const registryData = registryResult.data;
  const registry = useMemo(() => registryData?.columns ?? [], [registryData]);
  const shown = useMemo(() => selectedColumns(registry, options.columns, registryData?.defaultColumns ?? []), [registry, options.columns, registryData]);
  const sort = scoreboardSort(registry, shown, tableSortOf(options.tableSort, DEFAULT_SORT), registryData?.defaultSort ?? '');
  const rank = rankingColumn(registry, sort, options.topMeasure, registryData?.defaultSort ?? '');
  const descending = rankingDescending(sort, rank);
  const grouped = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const groups = useCountryGroups(runtime, track, active && grouped);
  const fallbackKey = JSON.stringify(selectedCountryKeys(widget.countries, track));
  const resolved = useMemo(() => resolveCountries(widget.countries, JSON.parse(fallbackKey) as string[], groups.data, lastHour, Number.MAX_SAFE_INTEGER),
    [widget.countries, fallbackKey, groups.data, lastHour]);
  const top = resolved.top ?? null;
  const following = active && widget.time.mode === 'follow';
  const headAt = (hour: number) => track ? resolveWindow(widget, track, runtime.tracks, hour)?.lastHour ?? null : null;

  const [rootElement, setRoot] = useState<HTMLDivElement | null>(null);
  const root = useMemo(() => ({ current: rootElement }), [rootElement]);
  const store = useMemo(() => track ? followStore(runtime.client, track.source, runtime.clock, track.axisOffset) : null, [runtime.client, runtime.clock, track?.source, track?.axisOffset]); // eslint-disable-line react-hooks/exhaustive-deps
  const branchId = track?.branch.id;
  const rankFamily = rank?.family ?? null;
  const rankQuery = useMemo<FollowQuery | null>(() => following && branchId && top != null && rankFamily
    ? { product: 'series-state', params: { branchId, family: rankFamily, top, least: descending ? undefined : true } } : null,
  [following, branchId, top, rankFamily?.kind, rankFamily?.measure, descending]); // eslint-disable-line react-hooks/exhaustive-deps
  const ranking = useFollowedAll(store, useMemo(() => [rankQuery], [rankQuery]), root);
  const rankEntry = ranking.entries[0] ?? null;
  const woken = useMemo(() => new Set<() => void>(), []);
  useEffect(() => {
    if (!store || !rankEntry) return undefined;
    return store.watch(rankEntry, () => { for (const wake of [...woken]) wake(); });
  }, [store, rankEntry, woken]);
  // A top's members as a set: the table orders them by the painted values, so a change of rank alone renders nothing.
  const membersAt = (hour: number) => top != null ? rankedTags(rankEntry?.answer).sort() : rankedCandidates(resolved, Math.floor(headAt(hour) ?? lastHour), resolved.tags);
  const structureHour = useHeadHour(runtime.clock, following, hour => membersAt(hour).join('\u0000'), woken);
  const membersText = following ? membersAt(structureHour).join('\u0000') : '';
  const members = useMemo(() => membersText ? membersText.split('\u0000') : [], [membersText]);
  const families = useMemo(() => {
    const out = new Map<string, ScoreboardFamily>();
    for (const column of shown) for (const family of [column.family, column.partial]) if (family) out.set(familyKey(family), family);
    return [...out.values()];
  }, [shown]);
  const familiesText = JSON.stringify(families);
  // The columns follow every country the rows have named since the top or the choice was asked, so a country entering
  // the top it held before, or one leaving it, asks no column again and its values are already held.
  const named = useStickyMembers(JSON.stringify([branchId, top, rankFamily, descending, following]), members,
    stickyLimit(top ?? members.length));
  const columnQueries = useMemo<(FollowQuery | null)[]>(() => families.map(family => following && branchId && named.length
    ? { product: 'series-state', params: { branchId, family, reads: named.map(tag => ({ country: countryFromKey(tag), hour: 0 })) } } : null),
  [familiesText, following, branchId, named]); // eslint-disable-line react-hooks/exhaustive-deps
  // A new set of members is asked under new queries without a render of its own: each column is painted from its last
  // answered entry until the new one answers, so its values never blank as the rows change.
  const columnsFollowed = useFollowedAll(store, columnQueries, root, undefined, 'availability');
  const columnsNow = useRef(columnsFollowed);
  columnsNow.current = columnsFollowed;
  // Followed unless the service refuses a query: one still pending shows its cells as not read yet, never asks.
  const followedHead = following && !!store && !ranking.reason && !columnsFollowed.reason && (top == null || rankQuery !== null);
  const handed = useMemo<HandedSources<LiveValues<bigint>>>(() => new Map(), [store, branchId]);
  const sources = useMemo(() => {
    const out = new Map<string, Live<LiveValues<bigint>> & { entry: () => FollowEntry | null }>();
    if (store) families.forEach((family, at) => out.set(familyKey(family), handedLive(store, () => columnsNow.current, at, handed, familyKey(family),
      entry => liveFollowed(store, entry, (held: FollowEntry) => followedValues(held.answer)), () => NOT_READ_YET)));
    return out;
  }, [store, familiesText, handed]); // eslint-disable-line react-hooks/exhaustive-deps
  // A new source for each set of members: the table sorts the rows it is given by the values now, in the commit that
  // shows them, never by the last order it took (which lacks a new member) and again a slot later.
  const live = useMemo(() => followedHead ? liveByColumn<SortValue>(Object.fromEntries(shown.map(column => [column.key,
    (column.family && sources.get(familyKey(column.family))) || silent(column.family ? 'Not read yet' : column.reason ?? 'Unavailable')]))) : undefined,
  [followedHead, shown, sources, members]); // eslint-disable-line react-hooks/exhaustive-deps
  const partialAt = useCallback((family: ScoreboardFamily | null, tag: string) => !!family && followedHead
    && isPartial(sources.get(familyKey(family))?.read().get(tag)), [followedHead, sources]);

  const playbackHour = useFallbackHour(runtime.clock, following && !followedHead);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const moment = Math.floor(window?.lastHour ?? lastHour);
  const askedTags = useMemo(() => top == null ? rankedCandidates(resolved, moment, resolved.tags) : [], [top, resolved, moment]);
  const asked = useMemo(() => shown.map(column => column.key), [shown]);
  const params = !followedHead && track && registryData && !resolved.reason ? scoreboardParams(track.branch.id, moment, shown, askedTags, top, rank, descending) : null;
  const request = params && track ? { sourceId: track.source.id, revision: track.source.revision, product: 'scoreboard', params } : null;
  const validate = useCallback((value: unknown) => validateScoreboardReply(value, asked.length), [asked]);
  const result = useFollowedProduct<ScoreboardReply>(runtime.client, request, ['hour', 'tags'], active, validate, track?.source);
  const reply = followedHead ? undefined : result.data;

  // At the head the rows are named as they are at the hour shown: read where one of them last changed.
  const identityHour = useIdentityHour(runtime.client, track, named, runtime.clock, followedHead,
    hour => track ? trackHourOf(widget, track, runtime.tracks, hour) : null);
  const looksHour = looksRead(followedHead, identityHour, reply?.hour ?? moment).hour;
  const rows = useMemo(() => followedHead ? members.map(tag => followedRow(tag, looksHour)) : reply ? replyRows(reply, asked) : NO_ROWS,
    [followedHead, members, looksHour, reply, asked]);
  // The hour shown: the newest every followed answer holds through, told as answers land and at each frame.
  const reportShown = useShownHour();
  const shownAt = useRef<() => number | null>(() => null);
  shownAt.current = () => {
    if (!followedHead) return reply?.hour ?? null;
    let through: number | null = null;
    for (const entry of [rankEntry, ...[...sources.values()].map(source => source.entry())]) {
      if (!entry) continue;
      const hour = shownThrough(entry);
      if (hour === null) return null;
      through = through === null ? hour : Math.min(through, hour);
    }
    return through;
  };
  useEffect(() => {
    const tell = () => reportShown(shownAt.current());
    tell();
    const stops = [runtime.clock.subscribeFrame(tell), ...store && rankEntry ? [store.watch(rankEntry, tell)] : [], ...[...sources.values()].map(source => source.subscribe(tell))];
    return () => { for (const stop of stops) stop(); };
  }, [runtime.clock, store, rankEntry, sources, reportShown, followedHead, reply]);
  const [visible, setVisible] = useState<readonly BoardRow[]>(NO_ROWS);
  // At the head every country the rows have named is labelled (a few hundred at most), so rows scrolling into view, or a
  // country coming back into the top, ask nothing more.
  const wanted = useMemo(() => (followedHead ? named : (sort.column === 'country' ? rows : visible).map(row => row.tag)).map(tag => ({ tag, hour: looksHour })),
    [named, rows, visible, sort.column, looksHour, followedHead]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const label = useMemo(() => labelAt(looksHour), [labelAt, looksHour]);
  const columns = useMemo<readonly TableColumn<BoardRow>[]>(() => [
    { key: 'country', header: 'Country', label: 'Country', width: 'minmax(var(--table-country-min), 2fr)',
      hint: top != null ? 'Top countries, sorted by name.' : undefined, renderMissing: true,
      value: row => label(row.tag).name, text: row => label(row.tag).name, render: row => <CountryCell {...label(row.tag)} /> },
    ...shown.map((column, index): TableColumn<BoardRow> => ({ key: column.key, header: column.label, label: column.label, numeric: true, renderMissing: true,
      width: 'max-content', priority: index + 1, inTooltip: true, bar: 'visible', hint: scoreboardColumnNote(column),
      value: row => row.cells.get(column.key)?.amount,
      render: row => {
        const cell = row.cells.get(column.key);
        return cell?.amount != null ? formatCell(column, cell.amount, cell.partial) : <span title={cell?.missing ?? column.reason ?? undefined}>—</span>;
      },
      paint: (value, key) => formatCell(column, BigInt(value), partialAt(column.partial, tagOf(key))) })),
  ], [shown, label, top, partialAt]);
  const tooltip = useCallback((row: BoardRow) => {
    const values = live?.read();
    const cell = (key: ScoreboardColumnKey): Cell => {
      const column = shown.find(item => item.key === key);
      if (!values) return row.cells.get(key) ?? { amount: null, missing: column?.reason ?? null, partial: false };
      const amount = values.get(liveKey(row.key, key));
      return amount == null ? { amount: null, missing: values.missing(liveKey(row.key, key)), partial: false }
        : { amount: BigInt(amount), missing: null, partial: partialAt(column?.partial ?? null, row.tag) };
    };
    const hour = followedHead ? headAt(runtime.clock.getHour()) ?? row.hour : row.hour;
    const look = label(row.tag);
    return fitTooltip([{ ...scoreboardTooltip(cell, shown, look.name, Math.floor(hour)), ...countryNameDetails(look) }], viewport);
  }, [live, shown, followedHead, label, partialAt, viewport, runtime.clock]); // eslint-disable-line react-hooks/exhaustive-deps
  const setTableOption = useCallback<NonNullable<WidgetProps['setOption']>>((key, value) => {
    if (key === 'tableSort') {
      const next = tableSortOf(value, DEFAULT_SORT);
      const measure = next.column === 'country' ? sort.column : next.column;
      if (registry.some(column => column.key === measure && column.available)) setOption?.('topMeasure', setToken(measure));
    }
    setOption?.(key, value);
  }, [setOption, registry, sort.column]);
  const groupsWarnings = groups.data?.warnings;
  useWhyCommit('ScoreboardWidget', { structureHour, members: membersText, named, rankEntry, columns: columnsFollowed, looksHour, labels: labelAt, reply, registry: registryData, visible });
  useEffect(() => {
    const absent = contentSet(options.columns).map(contentKey).filter(key => !registry.some(column => column.key === key));
    const unknown = registryData && absent.length
      ? [`${absent.length} selected ${absent.length === 1 ? 'column is' : 'columns are'} not in this recording: ${absent.join(', ')}.`] : [];
    const unfollowed = following && !followedHead && store ? [`Values requested at the playhead: ${ranking.reason ?? columnsFollowed.reason
      ?? 'the service does not follow them'}.`] : [];
    const warnings = [...(registryData?.warnings ?? []), ...(reply?.warnings ?? []), ...(groupsWarnings ?? []), ...unknown, ...unfollowed];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [registryData, reply, groupsWarnings, options.columns, onDiagnostics, following, followedHead, store, ranking.reason, columnsFollowed.reason, registry]);

  const hour = followedHead ? Math.floor(headAt(structureHour) ?? lastHour) : moment;
  const state = (() => {
    if (!track) return <WidgetState kind="empty" icon="grid" message="Load a campaign to compare its countries." />;
    if (!window && !followedHead) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
    if (registryResult.error) return <WidgetState kind="error" failure={registryResult.error} onRetry={registryResult.retry} />;
    if (result.error && !followedHead) return <WidgetState kind="error" failure={result.error} onRetry={result.retry} />;
    if (grouped && groups.error && !groups.data) return <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
    if (resolved.reason) return <WidgetState kind="empty" message={resolved.reason} />;
    if (registryData && !shown.length) return <WidgetState kind="unavailable" message="The selected columns are not in this recording. Choose columns in settings." />;
    if (!registryData || (grouped && !groups.data)) return <WidgetState kind="loading" label="Loading scoreboard…" />;
    // A group with no member asks for nothing: why, at the hour.
    const none = noMembersAt(widget.countries, resolved, groups.data, hour, lastHour);
    if (none) return <NoMembersState reason={none.reason} />;
    if (top == null && !(followedHead ? members : askedTags).length) {
      // A group whose members are all outside it at this hour says so, as an empty group.
      const why = grouped && groups.data ? worldFilter(widget.countries, [], groups.data, hour, lastHour).reason : null;
      return <WidgetState kind="unavailable" message={why ?? 'No countries available for this selection.'} />;
    }
    if (top != null && !rank) return <WidgetState kind="unavailable" message="No measure to rank the top countries by is in this recording." />;
    if (followedHead ? top != null && !rankEntry?.answer : !reply) return <WidgetState kind="loading" label="Loading scoreboard…" />;
    if (reply?.missing) return <WidgetState kind="unavailable" message={reply.missing} />;
    if (!rows.length) return <WidgetState kind="unavailable" message="No countries available for this selection." />;
    return null;
  })();
  const groupFailure = grouped && groups.error && groups.data ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  return <div ref={setRoot} className="scoreboard">{groupFailure}{state ?? <Table key={JSON.stringify([track!.source.id, track!.source.revision, track!.branch.id])}
    label="Scoreboard" rows={rows} columns={columns} rowKey={rowKey} live={live}
    rowRevision={row => [label(row.tag), ...shown.map(column => { const cell = row.cells.get(column.key); return [cell?.amount, cell?.partial, cell?.missing]; })]}
    sort={sort} setOption={setOption ? setTableOption : undefined} sortOptionValue={tableSortToken}
    cellBars={options.cellBars === true} rank rankMarkers={options.rankMarkers === true}
    density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)} tooltip={tooltip} onVisibleRows={followedHead ? undefined : setVisible}
    hoursPerSecond={hoursPerSecond} />}</div>;
}
