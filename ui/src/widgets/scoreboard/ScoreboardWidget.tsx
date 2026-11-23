import { useGate } from '../../dashboard/regionGate';
import { axisAt, SPEC } from '../../dashboard/regionTime';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { liveByColumn, liveKey, noValues } from '../../charts/live.ts';
import type { Live, LiveValues } from '../../charts/live';
import { tableSortOf, tableSortToken } from '../../charts/table/options';
import FamilyTable, { type FamilyColumn } from '../../draw/tables/FamilyTable.tsx';
import type { ExactRows } from '../../draw/tables/FamilyTable.tsx';
import { perfLog } from '../../shell/perf.ts';
import { encodeTableSettings, OFFERED, TABLE_SETTINGS_KEY, tableSettingsOf } from '../../draw/tables/settings.ts';
import type { TableSettings } from '../../draw/tables/logic.ts';
import type { TableReading, TableRowModel, TableValues } from '../../draw/tables/surface.ts';
import { countryNameDetails, fitTooltip, hideTooltip, showTooltip } from '../../charts/tooltip.ts';
import { useViewportHeight } from '../../charts/useChartTokens';
import { useIdentityHour } from '../../dashboard/atHead';
import { useHeldTimeline } from '../../dashboard/countryNamesAt';
import { useContentRevision } from '../../dashboard/contentRevision';
import { lookStarts } from '../../dashboard/countryTimeline';
import { lookAt } from '../../draw/tables/looks.ts';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { looksRead } from '../../dashboard/countryLooks';
import { trackHourOf } from '../../dashboard/settledHour';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { followStore } from '../../dashboard/follow/store.ts';
import { liveFollowed } from '../../dashboard/follow/liveFollowed.ts';
import { stickyLimit, stickyMembers, useStickyMembers, type StickyMembers } from '../../dashboard/follow/stickyMembers.ts';
import { shownThrough } from '../../dashboard/follow/store.ts';
import type { FollowEntry, FollowQuery } from '../../dashboard/follow/store';
import type { FollowedAnswer } from '../../dashboard/follow/message.ts';
import { usePlaybackHour } from '../../dashboard/follow/useFollow.ts';
import { noMembersAt, rankedCandidates, resolveCountries, worldFilter } from '../../dashboard/groups';
import NoMembersState from '../../dashboard/NoMembersState';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken } from '../../dashboard/options';
import { useFollowedProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useRegion, useRegionCoverage } from '../../dashboard/widgetTime';
import { useShownHour, useWhyCommit } from '../../dashboard/widgetPerf';
import { useHeadHour } from '../army-size/headHour.ts';
import { DEFAULT_SORT, OPTIONS, TABLE_BASE } from './definition';
import { handedLive, useFollowedAll } from './followedAll.ts';
import { useScoreboardColumns } from './scoreboardColumns.ts';
import { drawnCell, followedRow, followedValues, measureCell, partialSource, rankedTags, rankingColumn, rankingDescending, replyRows, replyValues,
  scoreboardColumnNote, scoreboardParams, scoreboardSort, scoreboardTooltip, selectedColumns, validateScoreboardReply } from './scoreboardData.ts';
import type { BoardRow } from './scoreboardData';
import './scoreboard.css';
import type { ScoreboardFamily, ScoreboardReply } from '../../generated/dashboard';
import { productRequest } from '../../dashboard/request.ts';

const NO_WARNINGS: string[] = [];
const NO_ROWS: readonly BoardRow[] = [];
const familyKey = (family: ScoreboardFamily) => `${family.kind}\u0000${family.measure}`;
const silent = <V,>(reason: string): Live<LiveValues<V>> => { const values = noValues(reason); return { read: () => values, subscribe: () => () => {} }; };
const NOT_READ_YET = noValues('Not read yet');
const RANK_SOURCE = 'top:rank';
const NO_NAMES: readonly string[] = [];
const rankState = (answer: FollowedAnswer | undefined) => !answer ? '' : answer.kind === 'notRecorded' ? `refused:${answer.reason}` : 'ranked';

export default function ScoreboardWidget({ widget, runtime, active, onDiagnostics, setOption, headerSlot }: WidgetProps) {
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const coverage = useRegionCoverage(runtime, useRegion(widget));
  const gate = useGate(runtime, widget, SPEC, active);
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
  const recorded = useCountryGroups(runtime, track, active && top != null);
  const everyText = useMemo(() => JSON.stringify((recorded.data?.countryIdentities ?? []).map(history => history.tag).sort()), [recorded.data]);
  const every = useMemo(() => JSON.parse(everyText) as string[], [everyText]);
  const headAt = (hour: number) => track ? resolveWindow(widget, track, runtime.tracks, hour)?.lastHour ?? null : null;

  const [rootElement, setRoot] = useState<HTMLDivElement | null>(null);
  const root = useMemo(() => ({ current: rootElement }), [rootElement]);
  const store = useMemo(() => track ? followStore(runtime.client, track, runtime.clock) : null, [runtime.client, runtime.clock, track?.source, track?.axisOffset]);
  const branchId = track?.branch.id;
  const rankFamily = rank?.family ?? null;
  const rankQuery = useMemo<FollowQuery | null>(() => following && branchId && top != null && rankFamily
    ? gate.follow({ product: 'series-state', params: { branchId, family: rankFamily, top, least: descending ? undefined : true } }) : null,
  [following, branchId, top, rankFamily?.kind, rankFamily?.measure, descending, gate.follow]);
  const ranking = useFollowedAll(store, useMemo(() => [rankQuery], [rankQuery]), root);
  const rankEntry = ranking.entries[0] ?? null;
  const woken = useMemo(() => new Set<() => void>(), []);
  useEffect(() => {
    if (!rankEntry) return undefined;
    return rankEntry.subscribe(() => { for (const wake of [...woken]) wake(); });
  }, [rankEntry, woken]);
  const stickyReset = JSON.stringify([branchId, top, rankFamily, descending, following]);
  const sticky = useRef<StickyMembers | null>(null);
  const membersAt = (hour: number) => top != null
    ? (sticky.current = stickyMembers(sticky.current, stickyReset, rankedTags(rankEntry?.answer), stickyLimit(top))).tags
    : rankedCandidates(resolved, Math.floor(headAt(hour) ?? lastHour), resolved.tags);
  const structureHour = useHeadHour(runtime.clock, following, hour => top != null ? rankState(rankEntry?.answer) : JSON.stringify(membersAt(hour)), woken, top == null);
  const membersText = following && top == null ? JSON.stringify(membersAt(structureHour)) : '[]';
  const members = useMemo(() => JSON.parse(membersText) as string[], [membersText]);
  const families = useMemo(() => {
    const out = new Map<string, ScoreboardFamily>();
    for (const column of shown) for (const family of [column.family, column.partial]) if (family) out.set(familyKey(family), family);
    return [...out.values()];
  }, [shown]);
  const familiesText = JSON.stringify(families);
  const chosen = useStickyMembers(stickyReset, members, stickyLimit(members.length));
  const named = top != null ? every : chosen;
  const columnReads = top != null ? 'every' : named.length ? named.map(tag => ({ country: countryFromKey(tag) })) : null;
  const columnReadsText = JSON.stringify(columnReads);
  const columnQueries = useMemo<(FollowQuery | null)[]>(() => families.map(family => following && branchId && columnReads
    ? gate.follow({ product: 'series-state', params: { branchId, family, reads: columnReads } }) : null),
  [familiesText, following, branchId, columnReadsText, gate.follow]);
  const columnsFollowed = useFollowedAll(store, columnQueries, root);
  const columnsNow = useRef(columnsFollowed);
  columnsNow.current = columnsFollowed;
  const followedHead = following && !!store && (top == null || rankQuery !== null);
  const refused = following ? ranking.reason ?? columnsFollowed.reason : undefined;
  const sources = useMemo(() => {
    const out = new Map<string, Live<LiveValues<bigint>> & { entry: () => FollowEntry | null }>();
    if (store) families.forEach((family, at) => out.set(familyKey(family), handedLive(() => columnsNow.current, at,
      entry => liveFollowed(entry, (held: FollowEntry) => followedValues(held.answer)), () => NOT_READ_YET)));
    return out;
  }, [store, familiesText]);

  const playbackHour = usePlaybackHour(runtime.clock, false);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const moment = Math.floor(window?.lastHour ?? lastHour);
  const askAt = following ? gate.hourAt(axisAt(playbackHour)) : gate.hour;
  const askedTags = useMemo(() => top == null ? rankedCandidates(resolved, moment, resolved.tags) : [], [top, resolved, moment]);
  const asked = useMemo(() => shown.map(column => column.key), [shown]);
  const params = !following && track && registryData && !resolved.reason && askAt !== null ? scoreboardParams(track.branch.id, askAt, shown, askedTags, top, rank, descending) : null;
  const request = params && track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'scoreboard', params }) : null;
  const validate = useCallback((value: unknown) => validateScoreboardReply(value, asked.length), [asked]);
  const result = useFollowedProduct<ScoreboardReply>(runtime.client, request, ['hour', 'tags'], active, validate, track?.source);
  const reply = followedHead ? undefined : result.data;
  const toTableRows = useRef<(board: readonly BoardRow[]) => TableRowModel[]>(() => []);
  const exactRows: ExactRows | undefined = perfLog && track && registryData && !resolved.reason ? wanted => {
    const at = gate.recorded(wanted);
    if (at === null) return Promise.reject(new Error('This hour was not recorded.'));
    const params = scoreboardParams(track.branch.id, at, shown, top == null ? rankedCandidates(resolved, at, resolved.tags) : [], top, rank, descending);
    if (!params) return Promise.reject(new Error('Nothing to read at this hour.'));
    const request = productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'scoreboard', params });
    return perfLog!.exactOnce(JSON.stringify(request), async () => {
      const board = replyRows(validateScoreboardReply((await runtime.client.product(request, new AbortController().signal)).data, asked.length), asked);
      return { rows: toTableRows.current(board), values: replyValues(board, shown) };
    });
  } : undefined;

  const identityHour = useIdentityHour(runtime.client, track, followedHead && top != null ? NO_NAMES : named, runtime.clock, false,
    hour => track ? trackHourOf(widget, track, runtime.tracks, hour) : null);
  const looksHour = looksRead(false, identityHour, reply?.hour ?? moment).hour;
  const timeline = useHeldTimeline(runtime.client, track, useContentRevision(runtime.client, active && followedHead), active && followedHead);
  const startsText = followedHead ? JSON.stringify(named.map(tag => [tag, lookStarts(timeline, tag, track?.branch.firstHour ?? moment, moment)])) : '[]';
  const starts = useMemo(() => JSON.parse(startsText) as [string, [number, boolean][]][], [startsText]);
  const rows = useMemo(() => followedHead ? (top != null ? every : members).map(tag => followedRow(tag, looksHour)) : reply ? replyRows(reply, asked) : NO_ROWS,
    [followedHead, top, every, members, looksHour, reply, asked]);
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
    if (!perfLog) return undefined;
    const tell = () => reportShown(shownAt.current());
    tell();
    const stops = [...rankEntry ? [rankEntry.subscribe(tell)] : [], ...[...sources.values()].map(source => source.subscribe(tell))];
    return () => { for (const stop of stops) stop(); };
  }, [runtime.clock, store, rankEntry, sources, reportShown, followedHead, reply]);
  const wanted = useMemo(() => followedHead ? starts.flatMap(([tag, hours]) => hours.flatMap(([hour, held]) => held ? [{ tag, hour }] : []))
    : rows.map(row => ({ tag: row.tag, hour: looksHour })),
  [starts, rows, looksHour, followedHead]);
  const { labelAt, exactAt } = useCountryLabelsAt(runtime.client, track, wanted, active, Infinity, true);
  const label = useMemo(() => exactAt(looksHour), [exactAt, looksHour]);
  const looksText = JSON.stringify(starts.map(([tag, hours]) => [tag, hours.map(([hour, held]) => {
    const look = held ? exactAt(hour)(tag) : undefined;
    return [hour, look?.known ? look.name : null, look?.flag ?? null, look && !look.known ? look.name : null];
  })]));
  const looks = useMemo(() => new Map((JSON.parse(looksText) as [string, [number, string | null, string | null, string | null][]][])
    .map(([tag, runs]) => [tag, runs.map(([from, name, flag, status]) => ({ from, name, flag: flag ?? undefined, status: status ?? undefined }))])), [looksText]);
  const tableColumns = useMemo<FamilyColumn[]>(() => [
    { key: 'country', label: 'Country', as: 'flagName', offered: OFFERED.country, priority: 0, text: true,
      hint: top != null ? 'Top countries, sorted by name.' : undefined },
    ...shown.map((column, index): FamilyColumn => ({ key: column.key, label: column.label, as: 'faint', tone: 'blue', offered: OFFERED.number, priority: index + 1,
      numeric: true, format: { decimals: column.decimals, percent: column.unit === 'percent', zero: column.noneAtZero ? 'None' : undefined }, hideable: shown.length > 1, hint: scoreboardColumnNote(column) })),
  ], [shown, top]);
  const tableRowsOf = useCallback((board: readonly BoardRow[]): TableRowModel[] => board.map(row => {
    const runs = looks.get(row.tag), look = runs ? undefined : label(row.tag);
    const named = look && (look.known ? { text: look.name } : { missing: look.name });
    return { key: row.key, cells: { country: runs ? { looks: runs, iconSlot: true, country: row.tag } : { ...named, icon: look!.flag, iconSlot: true, country: row.tag },
      ...Object.fromEntries(shown.map(column => [column.key, measureCell(column, row.tag)])) } };
  }), [shown, label, looks]);
  toTableRows.current = tableRowsOf;
  const tableRows = useMemo<TableRowModel[]>(() => tableRowsOf(rows), [rows, tableRowsOf]);
  const tableLive = useMemo<Live<TableValues>>(() => {
    if (!followedHead) { const values = replyValues(rows, shown); return { read: () => values, subscribe: () => () => {} }; }
    return liveByColumn<bigint>(Object.fromEntries(shown.flatMap(column => {
      const value = column.family && sources.get(familyKey(column.family)), partial = column.partial && sources.get(familyKey(column.partial));
      return [[column.key, value || silent(column.family ? 'Not read yet' : column.reason ?? 'Unavailable')], ...partial ? [[partialSource(column.key), partial]] : []];
    })));
  }, [followedHead, rows, shown, sources]);
  const sourceIds = useMemo(() => ({ current: new Map<string, number>() }), [store, branchId]);
  const rankingNow = useRef(ranking);
  rankingNow.current = ranking;
  const tableSources = useMemo<Live<Readonly<Record<string, number>>> | undefined>(() => {
    if (!followedHead) return undefined;
    const at = new Map(families.map((family, index) => [familyKey(family), index]));
    return {
      read: () => {
        const ids: Record<string, number> = {};
        for (const column of shown) for (const [name, family] of [[column.key, column.family], [partialSource(column.key), column.partial]] as const) {
          if (!family) continue;
          const id = columnsNow.current.current(at.get(familyKey(family))!)?.id ?? sourceIds.current.get(name);
          if (id === undefined) continue;
          sourceIds.current.set(name, id); ids[name] = id;
        }
        const rankId = top != null ? rankingNow.current.current(0)?.id : undefined;
        if (rankId !== undefined) ids[RANK_SOURCE] = rankId;
        return ids;
      },
      subscribe: changed => {
        const stops = [columnsNow.current.subscribe(changed), rankingNow.current.subscribe(changed)];
        return () => { for (const stop of stops) stop(); };
      },
    };
  }, [followedHead, families, shown, sourceIds, top]);
  const ranked = useMemo(() => ({ markers: options.rankMarkers === true }), [options.rankMarkers]);
  const tableMembers = useMemo(() => followedHead && top != null ? { source: RANK_SOURCE, most: top } : undefined, [followedHead, top]);
  const storedTable = useMemo(() => tableSettingsOf(options.tableSettings, tableColumns, TABLE_BASE), [options.tableSettings, tableColumns]);
  const tableSettings = useMemo<TableSettings>(() => ({ ...storedTable, order: tableColumns.map(column => column.key), sort }),
    [storedTable, tableColumns, sort.column, sort.direction]);
  const byKey = useMemo(() => new Map(rows.map(row => [row.key, row])), [rows]);
  const tooltipOwner = `scoreboard-table:${widget.id}`;
  useEffect(() => () => hideTooltip(tooltipOwner), [tooltipOwner]);
  const tooltipKeys = useCallback((key: string) => {
    const tag = byKey.get(key)?.tag ?? key;
    return shown.flatMap(column => [liveKey(tag, column.key), liveKey(tag, partialSource(column.key))]);
  }, [shown, byKey]);
  const tooltip = useCallback((row: BoardRow, drawn: TableReading) => {
    const runs = looks.get(row.tag), at = lookAt(runs, Math.floor(drawn.hour)), held = label(row.tag);
    const look = runs ? { ...held, name: at?.name ?? at?.status ?? 'Country name unavailable', flag: at?.flag } : held;
    return fitTooltip([{ ...scoreboardTooltip(key => drawnCell(drawn, row.tag, key), shown, look.name, Math.floor(drawn.hour)), ...countryNameDetails(look) }], viewport);
  }, [shown, label, looks, viewport]);
  const setTableOption = useCallback<NonNullable<WidgetProps['setOption']>>((key, value) => {
    if (key === 'tableSort') {
      const next = tableSortOf(value, DEFAULT_SORT);
      const measure = next.column === 'country' ? sort.column : next.column;
      if (registry.some(column => column.key === measure && column.available)) setOption?.('topMeasure', setToken(measure));
    }
    setOption?.(key, value);
  }, [setOption, registry, sort.column]);
  const onTableSettings = useCallback((next: TableSettings) => {
    if (next.sort.column !== sort.column || next.sort.direction !== sort.direction) { setTableOption('tableSort', tableSortToken(next.sort)); return; }
    const hidden = shown.filter(column => next.columns[column.key]?.show === false);
    if (hidden.length) { setOption?.('columns', shown.filter(column => !hidden.includes(column)).map(column => setToken(column.key)).join(' ')); return; }
    setOption?.(TABLE_SETTINGS_KEY, encodeTableSettings(next));
  }, [sort.column, sort.direction, setTableOption, shown, setOption]);
  const groupsWarnings = groups.data?.warnings;
  useWhyCommit('ScoreboardWidget', { structureHour, members: membersText, named, rankEntry, columns: columnsFollowed, looksHour, labels: labelAt, reply, registry: registryData });
  useEffect(() => {
    const absent = contentSet(options.columns).map(contentKey).filter(key => !registry.some(column => column.key === key));
    const unknown = registryData && absent.length
      ? [`${absent.length} selected ${absent.length === 1 ? 'column is' : 'columns are'} not in this recording: ${absent.join(', ')}.`] : [];
    const unfollowed = refused ? [`Values are not followed here: ${refused}.`] : following && !followedHead ? ['Values are not followed here: the ranked top names no measure.'] : [];
    const warnings = [...(registryData?.warnings ?? []), ...(reply?.warnings ?? []), ...(groupsWarnings ?? []), ...unknown, ...unfollowed];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [registryData, reply, groupsWarnings, options.columns, onDiagnostics, following, followedHead, refused, registry]);

  const hour = followedHead ? Math.floor(headAt(structureHour) ?? lastHour) : moment;
  const state = (() => {
    if (!track) return <WidgetState kind="empty" icon="grid" message="Load a campaign to compare its countries." />;
    if (coverage.outside) return <WidgetState kind="outside" read={coverage} />;
    if (followedHead && refused) return <WidgetState kind="unavailable" technical={refused} />;
    const followError = followedHead ? ranking.error ?? columnsFollowed.error : undefined;
    if (followError) return <WidgetState kind="error" failure={followError} onRetry={() => { ranking.retry(); columnsFollowed.retry(); }} />;
    if (!window && !followedHead) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
    if (registryResult.error) return <WidgetState kind="error" failure={registryResult.error} onRetry={registryResult.retry} />;
    if (result.error && !followedHead) return <WidgetState kind="error" failure={result.error} onRetry={result.retry} />;
    if (grouped && groups.error && !groups.data) return <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
    if (resolved.reason) return <WidgetState kind="empty" message={resolved.reason} />;
    if (registryData && !shown.length) return <WidgetState kind="unavailable" message="The selected columns are not in this recording. Choose columns in settings." />;
    if (!registryData || (grouped && !groups.data)) return <WidgetState kind="loading" label="Loading scoreboard…" />;
    const none = noMembersAt(widget.countries, resolved, groups.data, hour, lastHour);
    if (none) return <NoMembersState reason={none.reason} />;
    if (top == null && !(followedHead ? members : askedTags).length) {
      const why = grouped && groups.data ? worldFilter(widget.countries, [], groups.data, hour, lastHour).reason : null;
      return <WidgetState kind="unavailable" message={why ?? 'No countries available for this selection.'} />;
    }
    if (top != null && !rank) return <WidgetState kind="unavailable" message="No measure to rank the top countries by is in this recording." />;
    if (followedHead && (ranking.outside || columnsFollowed.outside)) return <WidgetState kind="outside" />;
    if (followedHead && !families.length) return <WidgetState kind="unavailable" message={shown.map(column => column.reason).filter(Boolean).join(' ') || 'No selected column is followed in this recording.'} />;
    if (followedHead ? top != null && !rankEntry?.answer : !reply) return <WidgetState kind="loading" label="Loading scoreboard…" />;
    if (followedHead && top != null && rankEntry?.answer?.kind === 'notRecorded') return <WidgetState kind="unavailable" message={rankEntry.answer.reason} />;
    if (reply?.missing) return <WidgetState kind="unavailable" message={reply.missing} />;
    if (!rows.length) return <WidgetState kind="unavailable" message="No countries available for this selection." />;
    return null;
  })();
  const groupFailure = grouped && groups.error && groups.data ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  return <div ref={setRoot} className="scoreboard">{groupFailure}{state ?? <FamilyTable key={JSON.stringify([track!.source.id, track!.source.revision, track!.branch.id])}
    label="Scoreboard" clock={runtime.clock} offset={track!.axisOffset} columns={tableColumns} rows={tableRows}
    settings={tableSettings} onSettings={setOption ? onTableSettings : undefined} live={tableLive} sources={tableSources} shownAt={reply?.hour ?? hour}
    fixed={widget.time.mode === 'follow' || !reply ? undefined : reply.hour - track!.axisOffset}
    rank={ranked} members={tableMembers} headerSlot={headerSlot ?? null} tooltipKeys={tooltipKeys} exactRows={exactRows}
    onHover={(key, at, drawn) => {
      const row = key === null ? undefined : byKey.get(key);
      if (row && at && drawn) showTooltip(tooltipOwner, tooltip(row, drawn), at.x, at.y, row.key); else hideTooltip(tooltipOwner);
    }} />}</div>;
}
