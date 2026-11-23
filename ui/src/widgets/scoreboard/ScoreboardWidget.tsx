import { useCallback, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import { tableSortOf, tableSortToken } from '../../charts/table/options';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useHeldTimeline } from '../../dashboard/countryNamesAt';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import { lookStarts } from '../../dashboard/countryTimeline';
import type { WidgetProps } from '../../dashboard/definitions';
import { usePlaybackHour } from '../../dashboard/follow/useFollow.ts';
import { noMembersAt, rankedCandidates, resolveCountries, worldFilter } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import NoMembersState from '../../dashboard/NoMembersState';
import { contentKey, contentSet, resolveOptions, setToken } from '../../dashboard/options';
import { useFollowedProduct } from '../../dashboard/products';
import { useGate } from '../../dashboard/regionGate';
import { axisAt, SPEC } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request.ts';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useRegion, useRegionCoverage } from '../../dashboard/widgetTime';
import type { TableSettings } from '../../draw/tables/logic.ts';
import { encodeTableSettings, TABLE_SETTINGS_KEY, tableSettingsOf } from '../../draw/tables/settings.ts';
import type { FamilySources } from '../../families/contract.ts';
import FamilyTableHost, { type FamilySourceSubscription } from '../../families/table/FamilyTableHost.tsx';
import type { ScoreboardReply } from '../../generated/dashboard';
import { perfLog } from '../../shell/perf.ts';
import type { HourRanges } from '../occupied-states/occupiedProjection.ts';
import { DEFAULT_SORT, OPTIONS, TABLE_BASE } from './definition';
import { useScoreboardColumns } from './scoreboardColumns.ts';
import {
  partialSource,
  rankingColumn,
  rankingDescending,
  scoreboardParams,
  scoreboardSort,
  selectedColumns,
  validateScoreboardReply,
} from './scoreboardData.ts';
import {
  RANK_SOURCE,
  scoreboardColumnsSpec,
  scoreboardProjection,
  type ScoreboardProjectionOptions,
} from './scoreboardProjection.ts';
import './scoreboard.css';

export default function ScoreboardWidget({ widget, runtime, active, onDiagnostics, setOption, headerSlot }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const coverage = useRegionCoverage(runtime, useRegion(widget));
  const gate = useGate(runtime, widget, SPEC, active);
  const firstHour = track?.branch.firstHour ?? 0;
  const lastHour = track?.branch.lastHour ?? 0;
  const following = widget.time.mode === 'follow';

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
  const resolved = useMemo(
    () => resolveCountries(widget.countries, JSON.parse(fallbackKey) as string[], groups.data, lastHour, Number.MAX_SAFE_INTEGER),
    [widget.countries, fallbackKey, groups.data, lastHour],
  );
  const top = resolved.top ?? null;
  const recorded = useCountryGroups(runtime, track, active && top != null);
  const everyText = useMemo(() => JSON.stringify((recorded.data?.countryIdentities ?? []).map(history => history.tag).sort()), [recorded.data]);
  const every = useMemo(() => JSON.parse(everyText) as string[], [everyText]);

  const membership = useMemo<Readonly<Record<string, HourRanges>> | null>(() => {
    if (!resolved.membership) return null;
    const out: Record<string, [number, number][]> = {};
    for (const [tag, runs] of resolved.membership) {
      out[tag] = runs.map(run => [run.firstHour, run.lastHour]);
    }
    return out;
  }, [resolved.membership]);

  const candidates = useMemo(() => (top != null ? every : resolved.tags), [top, every, resolved.tags]);
  const timeline = useHeldTimeline(runtime.client, track, useContentRevision(runtime.client, active), active);
  const startsText = JSON.stringify(candidates.map(tag => [tag, lookStarts(timeline, tag, firstHour, lastHour)]));
  const starts = useMemo(() => JSON.parse(startsText) as [string, [number, boolean][]][], [startsText]);
  const wanted = useMemo(() => starts.flatMap(([tag, hours]) => hours.flatMap(([at, held]) => (held ? [{ tag, hour: at }] : []))), [starts]);
  const { exactAt } = useCountryLabelsAt(runtime.client, track, wanted, active, Infinity, true);
  const looksText = JSON.stringify(starts.map(([tag, hours]) => [tag, hours.map(([at, held]) => {
    const look = held ? exactAt(at)(tag) : undefined;
    return [at, look?.known ? look.name : null, look?.flag ?? null, look && !look.known ? look.name : null];
  })]));
  const looks = useMemo(
    () => Object.fromEntries((JSON.parse(looksText) as [string, [number, string | null, string | null, string | null][]][])
      .map(([tag, runs]) => [tag, runs.map(([from, name, flag, status]) => ({ from, name, flag: flag ?? undefined, status: status ?? undefined }))])),
    [looksText],
  );

  const tableColumns = useMemo(() => scoreboardColumnsSpec(shown, top), [shown, top]);
  const storedTable = useMemo(() => tableSettingsOf(options.tableSettings, tableColumns, TABLE_BASE), [options.tableSettings, tableColumns]);
  const tableSettings = useMemo<TableSettings>(
    () => ({ ...storedTable, order: tableColumns.map(col => col.key), sort }),
    [storedTable, tableColumns, sort.column, sort.direction],
  );

  const setTableOption = useCallback<NonNullable<WidgetProps['setOption']>>((key, value) => {
    if (key === 'tableSort') {
      const next = tableSortOf(value, DEFAULT_SORT);
      const measure = next.column === 'country' ? sort.column : next.column;
      if (registry.some(column => column.key === measure && column.available)) setOption?.('topMeasure', setToken(measure));
    }
    setOption?.(key, value);
  }, [setOption, registry, sort.column]);

  const onTableSettings = useCallback((next: TableSettings) => {
    if (next.sort.column !== sort.column || next.sort.direction !== sort.direction) {
      setTableOption('tableSort', tableSortToken(next.sort));
      return;
    }
    const hidden = shown.filter(column => next.columns[column.key]?.show === false);
    if (hidden.length) {
      setOption?.('columns', shown.filter(column => !hidden.includes(column)).map(column => setToken(column.key)).join(' '));
      return;
    }
    setOption?.(TABLE_SETTINGS_KEY, encodeTableSettings(next));
  }, [sort.column, sort.direction, setTableOption, shown, setOption]);

  const followPlan = useMemo(() => {
    if (!following || !active || !track || !registryData || resolved.reason) {
      return { subs: [] as FamilySourceSubscription[], refused: null as string | null, hasFamilies: false };
    }
    const branchId = track.branch.id;
    const subs: FamilySourceSubscription[] = [];
    let refused: string | null = null;
    const add = (name: string, rawQuery: Record<string, unknown>) => {
      const checked = gate.checkFollow(rawQuery);
      if (checked.refused) refused ??= checked.refused;
      else if (checked.query) {
        subs.push({ name, source: track.source, branch: branchId, offset: track.axisOffset, query: checked.query, channel: 'followed' });
      }
    };
    if (top != null) {
      if (!rank?.family) return { subs: [], refused: null, hasFamilies: false };
      add(RANK_SOURCE, { product: 'series-state', params: { branchId, family: rank.family, top, least: descending ? undefined : true } });
    }
    const columnReads = top != null ? 'every' : resolved.tags.length ? resolved.tags.map(tag => ({ country: countryFromKey(tag) })) : null;
    let hasFamilies = false;
    if (columnReads) {
      for (const column of shown) {
        if (column.family) {
          hasFamilies = true;
          add(column.key, { product: 'series-state', params: { branchId, family: column.family, reads: columnReads } });
        }
        if (column.partial) {
          add(partialSource(column.key), { product: 'series-state', params: { branchId, family: column.partial, reads: columnReads } });
        }
      }
    }
    return { subs, refused, hasFamilies };
  }, [following, active, track, registryData, resolved.reason, resolved.tags, top, rank, descending, shown, gate.checkFollow]);

  const playbackHour = usePlaybackHour(runtime.clock, false);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const moment = Math.floor(window?.lastHour ?? lastHour);
  const askAt = following ? null : gate.hourAt(axisAt(playbackHour));
  const askedTags = useMemo(() => (top == null ? rankedCandidates(resolved, moment, resolved.tags) : []), [top, resolved, moment]);
  const asked = useMemo(() => shown.map(column => column.key), [shown]);
  const params = !following && track && registryData && !resolved.reason && askAt !== null
    ? scoreboardParams(track.branch.id, askAt, shown, askedTags, top, rank, descending)
    : null;
  const request = params && track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'scoreboard', params }) : null;
  const validate = useCallback((value: unknown) => validateScoreboardReply(value, asked.length), [asked]);
  const result = useFollowedProduct<ScoreboardReply>(runtime.client, request, ['hour', 'tags'], active && !following, validate, track?.source);

  const staticSources = useMemo<{ readonly sources: FamilySources; readonly hour: number } | null>(() => {
    if (following || !result.data) return null;
    return {
      sources: { answers: new Map(), panels: new Map([['main', result.data]]), live: new Map() },
      hour: result.data.hour,
    };
  }, [following, result.data]);

  const exactSources = useCallback(async (wantedHour: number): Promise<FamilySources | null> => {
    if (!track || !registryData || resolved.reason || !perfLog) return null;
    const at = gate.recorded(wantedHour);
    if (at === null) return null;
    const exactParams = scoreboardParams(
      track.branch.id,
      at,
      shown,
      top == null ? rankedCandidates(resolved, at, resolved.tags) : [],
      top,
      rank,
      descending,
    );
    if (!exactParams) return null;
    const req = productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'scoreboard', params: exactParams });
    const raw = await perfLog.exactOnce(JSON.stringify(req), async () => (await runtime.client.product(req, new AbortController().signal)).data);
    return { answers: new Map(), panels: new Map([['main', raw]]), live: new Map() };
  }, [track, registryData, resolved, gate, shown, top, rank, descending, runtime.client]);

  const projectionOptions = useMemo<ScoreboardProjectionOptions>(() => ({
    shown,
    candidates,
    membership: top == null ? membership : null,
    top,
    rankMarkers: options.rankMarkers === true,
    looks,
  }), [shown, candidates, top, membership, options.rankMarkers, looks]);

  const extraWarnings = useMemo(() => {
    const absent = contentSet(options.columns).map(contentKey).filter(key => !registry.some(column => column.key === key));
    const unknown = registryData && absent.length
      ? [`${absent.length} selected ${absent.length === 1 ? 'column is' : 'columns are'} not in this recording: ${absent.join(', ')}.`]
      : [];
    const unfollowed = followPlan.refused ? [`Values are not followed here: ${followPlan.refused}.`] : [];
    return [...(registryData?.warnings ?? []), ...(groups.data?.warnings ?? []), ...unknown, ...unfollowed];
  }, [options.columns, registry, registryData, groups.data?.warnings, followPlan.refused]);

  const bounds = useMemo<readonly [number, number] | null>(
    () => (track ? [track.branch.firstHour, track.branch.lastHour] : null),
    [track?.branch.firstHour, track?.branch.lastHour],
  );

  const blockedState = (() => {
    if (!track) return <WidgetState kind="empty" icon="grid" message="Load a campaign to compare its countries." />;
    if (coverage.outside) return <WidgetState kind="outside" read={coverage} />;
    if (followPlan.refused) return <WidgetState kind="unavailable" message={followPlan.refused} />;
    if (!window && !following) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
    if (registryResult.error) return <WidgetState kind="error" failure={registryResult.error} onRetry={registryResult.retry} />;
    if (result.error && !following) return <WidgetState kind="error" failure={result.error} onRetry={result.retry} />;
    if (grouped && groups.error && !groups.data) return <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
    if (resolved.reason) return <WidgetState kind="empty" message={resolved.reason} />;
    if (registryData && !shown.length) return <WidgetState kind="unavailable" message="The selected columns are not in this recording. Choose columns in settings." />;
    if (!registryData || (grouped && !groups.data)) return <WidgetState kind="loading" label="Loading scoreboard…" />;
    const none = noMembersAt(widget.countries, resolved, groups.data, moment, lastHour);
    if (none && !following) return <NoMembersState reason={none.reason} />;
    if (top == null && !(following ? resolved.tags : askedTags).length) {
      const why = grouped && groups.data ? worldFilter(widget.countries, [], groups.data, moment, lastHour).reason : null;
      return <WidgetState kind="unavailable" message={why ?? 'No countries available for this selection.'} />;
    }
    if (top != null && !rank) return <WidgetState kind="unavailable" message="No measure to rank the top countries by is in this recording." />;
    if (following && !followPlan.hasFamilies) {
      return <WidgetState kind="unavailable" message={shown.map(column => column.reason).filter(Boolean).join(' ') || 'No selected column is followed in this recording.'} />;
    }
    if (!following && !result.data) return <WidgetState kind="loading" label="Loading scoreboard…" />;
    return null;
  })();

  const groupFailure = grouped && groups.error && groups.data ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;

  return <div className="scoreboard">
    {groupFailure}
    <FamilyTableHost
      key={track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id]) : 'none'}
      declaration={scoreboardProjection}
      options={projectionOptions}
      label="Scoreboard"
      subject="scoreboard"
      widget={widget}
      runtime={runtime}
      active={active}
      columns={tableColumns}
      settings={tableSettings}
      onSettings={setOption ? onTableSettings : undefined}
      subscriptions={followPlan.subs}
      staticSources={staticSources}
      exactSources={track && registryData && !resolved.reason && perfLog ? exactSources : undefined}
      offset={track?.axisOffset ?? 0}
      pin={following ? null : staticSources?.hour ?? null}
      bounds={bounds}
      headerSlot={headerSlot}
      onDiagnostics={onDiagnostics}
      extraWarnings={extraWarnings}
      blockedState={blockedState}
      loadingLabel="Loading scoreboard…"
    />
  </div>;
}
