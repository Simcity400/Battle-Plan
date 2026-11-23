import { useCallback, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import { narrowTableOf, tableSortOf, tableSortToken } from '../../charts/table/options';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { useHeldTimeline } from '../../dashboard/countryNamesAt';
import { lookStarts } from '../../dashboard/countryTimeline';
import type { WidgetProps } from '../../dashboard/definitions';
import { TRAILING_HOURS } from '../../dashboard/model';
import { useGate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request.ts';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useRegion, useRegionCoverage } from '../../dashboard/widgetTime';
import type { TableSettings } from '../../draw/tables/logic.ts';
import { tableSettingsOf } from '../../draw/tables/settings.ts';
import type { FamilySources } from '../../families/contract.ts';
import FamilyTableHost, { type FamilySourceSubscription } from '../../families/table/FamilyTableHost.tsx';
import { perfLog } from '../../shell/perf.ts';
import { playerNames } from './apmData';
import { apmHeadQuery } from './apmHead.ts';
import { apmTableColumns, apmTableProjection, type ApmTableProjectionOptions } from './apmProjection.ts';
import { DEFAULT_SORT } from './definition';
import { useApm, useApmRegistry } from './useApm';

export default function ApmTableWidget(props: WidgetProps) {
  const { widget, runtime, active, setOption, onDiagnostics, headerSlot } = props;
  const following = widget.time.mode === 'follow';
  const query = useApm(props, false, null, following && active);
  const { data, track, options, first, last } = query;
  const paused = options.showPausedActions === true;
  const players = options.group === 'player';
  const gate = useGate(runtime, widget, SPEC, active);
  const coverage = useRegionCoverage(runtime, useRegion(widget));
  const registry = useApmRegistry(widget, runtime, active && players);

  const headKey = query.head ? JSON.stringify(query.head) : null;
  const checked = useMemo(
    () => (following && active && track && query.head ? gate.checkFollow(apmHeadQuery(query.head, track.branch, TRAILING_HOURS)) : null),
    [following, active, track?.branch.id, track?.branch.firstHour, track?.branch.lastHour, headKey, gate.checkFollow],
  );
  const subscriptions = useMemo<readonly FamilySourceSubscription[]>(
    () => (track && checked?.query
      ? [{ name: 'main', source: track.source, branch: track.branch.id, offset: track.axisOffset, query: checked.query, channel: 'followed' }]
      : []),
    [track?.source, track?.branch.id, track?.axisOffset, checked?.query],
  );

  const allGroups = useCountryGroups(runtime, track, active);
  const allTagsText = useMemo(() => JSON.stringify((allGroups.data?.countryIdentities ?? []).map(h => h.tag).sort()), [allGroups.data]);
  const allTags = useMemo(() => JSON.parse(allTagsText) as string[], [allTagsText]);
  const firstHour = track?.branch.firstHour ?? 0;
  const lastHour = track?.branch.lastHour ?? 0;
  const timeline = useHeldTimeline(runtime.client, track, useContentRevision(runtime.client, active), active);
  const startsText = JSON.stringify(allTags.map(tag => [tag, lookStarts(timeline, tag, firstHour, lastHour)]));
  const starts = useMemo(() => JSON.parse(startsText) as [string, [number, boolean][]][], [startsText]);
  const wanted = useMemo(() => starts.flatMap(([tag, hours]) => hours.flatMap(([at, held]) => (held ? [{ tag, hour: at }] : []))), [starts]);
  const { exactAt } = useCountryLabelsAt(runtime.client, track, wanted, active, Infinity, true);
  const labelsText = JSON.stringify(starts.map(([tag, hours]) => [tag, hours.map(([at, held]) => {
    const look = held ? exactAt(at)(tag) : undefined;
    return [at, look?.known ? look.name : null, look?.flag ?? null, look && !look.known ? look.name : null];
  })]));
  const labels = useMemo(
    () => Object.fromEntries((JSON.parse(labelsText) as [string, [number, string | null, string | null, string | null][]][])
      .map(([tag, runs]) => [tag, runs.map(([from, name, flag, status]) => ({ from, name, flag: flag ?? undefined, status: status ?? undefined }))])),
    [labelsText],
  );

  const namesRecord = useMemo<Readonly<Record<string, string>>>(() => {
    const map = playerNames(registry.data?.players ?? []);
    return Object.fromEntries(map);
  }, [registry.data]);

  const columns = useMemo(() => apmTableColumns(players, paused, options.cellBars === true), [players, paused, options.cellBars]);
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const overflow = narrowTableOf(options) === 'scroll' ? 'scroll' as const : 'tooltip' as const;
  const density = options.density === 'compact' ? 'compact' as const : 'comfortable' as const;
  const settings = useMemo<TableSettings>(
    () => tableSettingsOf(undefined, columns, { density, overflow, sort }),
    [columns, density, overflow, sort.column, sort.direction],
  );
  const onSettings = useCallback((next: TableSettings) => {
    if (next.sort.column !== sort.column || next.sort.direction !== sort.direction) {
      setOption?.('tableSort', tableSortToken(next.sort));
    }
  }, [sort.column, sort.direction, setOption]);

  const staticSources = useMemo<{ readonly sources: FamilySources; readonly hour: number } | null>(() => {
    if (following || !data) return null;
    return {
      sources: { answers: new Map(), panels: new Map([['main', data]]), live: new Map() },
      hour: query.answered?.lastHour ?? last ?? lastHour,
    };
  }, [following, data, query.answered?.lastHour, last, lastHour]);

  const exactSources = useCallback(async (wantedHour: number): Promise<FamilySources | null> => {
    if (!track || !query.head || !perfLog) return null;
    const recorded = gate.recorded(wantedHour);
    if (recorded === null) return null;
    const end = Math.min(recorded, track.branch.lastHour);
    const start = Math.max(track.branch.firstHour, end - (TRAILING_HOURS - 1));
    const span = gate.span(start, end);
    if (!span) return null;
    const req = productRequest({
      sourceId: track.source.id,
      revision: track.source.revision,
      product: 'player-apm',
      params: { ...query.head, registry: false, ...span, buckets: 1 },
    });
    const raw = await perfLog.exactOnce(JSON.stringify(req), async () => (await runtime.client.product(req, new AbortController().signal)).data);
    return { answers: new Map(), panels: new Map([['main', raw]]), live: new Map() };
  }, [track, query.head, gate, runtime.client]);

  const projectionOptions = useMemo<ApmTableProjectionOptions>(() => ({
    players,
    showPaused: paused,
    cellBars: options.cellBars === true,
    rankMarkers: options.rankMarkers === true,
    playerNames: namesRecord,
    labels,
  }), [players, paused, options.cellBars, options.rankMarkers, namesRecord, labels]);

  const bounds = useMemo<readonly [number, number] | null>(
    () => (track ? [track.branch.firstHour, track.branch.lastHour] : null),
    [track?.branch.firstHour, track?.branch.lastHour],
  );

  const blockedState = (() => {
    if (!track) return <WidgetState kind="empty" icon="chart-bars" message="Load a campaign to see eAPM." />;
    if (coverage.outside) return <WidgetState kind="outside" read={coverage} />;
    if (checked?.refused) return <WidgetState kind="unavailable" message={checked.refused} />;
    if (!following && (!query.window || first == null || last == null || first > last)) return <WidgetState kind="outside" />;
    if (query.groups.error && widget.countries.kind === 'group') return <WidgetState kind="error" failure={query.groups.error} onRetry={query.groups.retry} />;
    if (query.reason && !following) return <WidgetState kind="unavailable" message={query.reason} />;
    if (players && registry.error) return <WidgetState kind="error" failure={registry.error} onRetry={registry.retry} />;
    if (players && !registry.data) return <WidgetState kind="loading" label="Loading eAPM…" />;
    if (following) {
      if (query.reason) return <WidgetState kind="unavailable" message={query.reason} />;
      return null;
    }
    if (query.error) return <WidgetState kind="error" failure={query.error} onRetry={query.retry} />;
    if (!data || !query.known) return <WidgetState kind="loading" label="Loading eAPM…" />;
    return null;
  })();

  return <FamilyTableHost
    key={track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id]) : 'none'}
    declaration={apmTableProjection}
    options={projectionOptions}
    label="eAPM over the selected period"
    subject="eAPM"
    widget={widget}
    runtime={runtime}
    active={active}
    columns={columns}
    settings={settings}
    onSettings={setOption ? onSettings : undefined}
    subscriptions={subscriptions}
    staticSources={staticSources}
    exactSources={track && query.head && perfLog ? exactSources : undefined}
    offset={track?.axisOffset ?? 0}
    pin={following ? null : staticSources?.hour ?? null}
    bounds={bounds}
    headerSlot={headerSlot}
    onDiagnostics={onDiagnostics}
    blockedState={blockedState}
    loadingLabel="Loading eAPM…"
  />;
}
