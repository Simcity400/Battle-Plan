import { useGate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import TimeChart from '../../charts/TimeChart';
import type { ChartData, ChartLayer } from '../../charts/TimeChart';
import type { ChartStrip } from '../../charts/strip';
import type { PlotFrame } from '../../charts/frame';
import Legend from '../../charts/Legend';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { dayLabel, drawLine, font } from '../../charts/draw';
import { extentOfAll } from '../../charts/series';
import { binsLabel, drawLineSummary, summaryExtent } from '../../charts/chartSummary';
import type { ChartBucket, ChartSummary } from '../../charts/chartSummary';
import { lineStyles, SECONDARY_DASH } from '../../charts/palette';
import { countryNameDetails, fitTooltip } from '../../charts/tooltip.ts';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { useAtHead, useIdentityHour } from '../../dashboard/atHead';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { branchHour } from '../../dashboard/countryLooks';
import { useHolderLooks } from '../../dashboard/holderLooks';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { followStore } from '../../dashboard/follow/store.ts';
import { FollowedText } from '../../dashboard/follow/FollowedText.tsx';
import { liveFollowed } from '../../dashboard/follow/liveFollowed.ts';
import { shownThrough } from '../../dashboard/follow/store.ts';
import type { FollowEntry } from '../../dashboard/follow/store';
import { followRefusal, followsHead, useFollow } from '../../dashboard/follow/useFollow.ts';
import { useProduct } from '../../dashboard/products';
import { useTiles } from '../../dashboard/tiles/useTiles';
import { useHeadHour } from '../army-size/headHour.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import { TRAILING_HOURS } from '../../dashboard/model';
import type { ApmReply, ApmSeries, ApmValue } from '../../generated/dashboard';
import { apmRuns, apmText, countText, playerNames, pointAt, valueNote, apmValueText, validateApm, APM_EXPLANATION, GAPS_NOTE } from './apmData';
import { apmFromSort, apmHeadQuery, apmHeadRows, apmHeadStructure, apmHolders, apmLiveValues, apmPausedView, apmSortValue } from './apmHead.ts';
import { DEFAULT_SORT } from './definition';
import { useApm, useApmRegistry } from './useApm';
import { useChangedState } from '../../dashboard/changedState.ts';
import { productRequest } from '../../dashboard/request.ts';

const NO_ROWS: readonly ApmSeries[] = [];
const NO_TAGS: readonly string[] = [];
const rowKey = (row: ApmSeries) => row.id;
const FILL = { display: 'flex', flexDirection: 'column', flex: '1 1 auto', minHeight: 0, minWidth: 0, height: '100%' } as const;

function tiledBin(summary: ChartSummary | undefined, hour: number): ChartBucket | null {
  const binHours = summary?.tiles?.binHours;
  if (!summary || !binHours) return null;
  const bucket = summary.buckets[Math.floor(hour / binHours) - Math.floor(summary.from / binHours)];
  return bucket && bucket.firstHour <= hour + binHours ? bucket : null;
}

export function ApmWidget(props: WidgetProps & { graph: boolean }) {
  const { graph, runtime, active, widget, setOption, onDiagnostics } = props;
  const [view, setView] = useChangedState<{ identity: string; from: number; to: number; width: number } | null>(null);
  const query = useApm(props, graph, view, widget.time.mode === 'follow' && active);
  const { data, track, options, first, last } = query;
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const motion = usePlaybackMotion(runtime.clock);
  const paused = options.showPausedActions === true;
  const players = options.group === 'player';
  const following = widget.time.mode === 'follow';
  const gate = useGate(runtime, widget, SPEC, active);

  const [rootElement, setRoot] = useState<HTMLDivElement | null>(null);
  const root = useMemo(() => ({ current: rootElement }), [rootElement]);
  const store = useMemo(() => track ? followStore(runtime.client, track, runtime.clock) : null, [runtime.client, runtime.clock, track?.source, track?.axisOffset]);
  const headKey = query.head ? JSON.stringify(query.head) : null;
  const headQuery = useMemo(() => following && active && track && query.head ? gate.follow(apmHeadQuery(query.head, track.branch, TRAILING_HOURS)) : null,
    [following, active, track?.branch.id, track?.branch.firstHour, track?.branch.lastHour, headKey, gate.follow]);
  const woken = useMemo(() => new Set<() => void>(), []);
  const shown = useShownHour();
  const followed = useFollow(store, headQuery, root, useCallback((entry: FollowEntry) => {
    for (const wake of [...woken]) wake();
    shown(shownThrough(entry));
  }, [woken, shown]));
  const headEntry = followsHead(followed) ? followed.entry : null;
  const refused = followRefusal(followed);
  const entryRef = useRef(headEntry); entryRef.current = headEntry;
  useHeadHour(runtime.clock, headEntry !== null && !graph, () => apmHeadStructure(entryRef.current?.answer), woken);
  const headStructure = headEntry && !graph ? apmHeadStructure(headEntry.answer) : null;
  const headRows = useMemo(() => headEntry ? apmHeadRows(headEntry.answer) : null, [headEntry, headStructure]);
  const headRefused = headEntry?.answer?.kind === 'notRecorded' ? headEntry.answer.reason : null;
  const live = useMemo(() => store && headEntry ? liveFollowed(headEntry, entry => apmLiveValues(entry.answer)) : undefined, [store, headEntry]);
  const current = (row: ApmSeries): ApmValue => (entryRef.current ? apmHeadRows(entryRef.current.answer).find(held => held.id === row.id)?.summary : undefined) ?? row.summary;

  const tiled = graph && headEntry !== null;
  const branchSpan = track ? gate.span(track.branch.firstHour, track.branch.lastHour) : null;
  const branchSeries = useProduct<ApmReply>(runtime.client, tiled && track && query.head && branchSpan ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'player-apm',
    params: { ...query.head, registry: false, ...branchSpan, buckets: 1 } }) : null, active, validateApm, track?.source);
  const ids = useMemo(() => branchSeries.data?.series.map(series => series.id) ?? [], [branchSeries.data]);
  const tileSeries = (value: string) => tiled && track && query.head && ids.length ? { source: track.source, span: gate.span, branchId: track.branch.id, lastHour: track.branch.lastHour, tags: ids,
    family: { kind: 'player-apm', group: query.head.group, tags: query.head.tags, membership: query.head.membership, players: query.head.players, value } } : null;
  const rates = useTiles(runtime.client, tileSeries('apm'), active, true);
  const pausedRates = useTiles(runtime.client, paused ? tileSeries('pausedOrders') : null, active, true);

  const registry = useApmRegistry(widget, runtime, active && players);
  const requested = players && !registry.data ? NO_ROWS : data?.series ?? NO_ROWS;
  const rows = headEntry ? (players && !registry.data ? NO_ROWS : graph ? branchSeries.data?.series ?? NO_ROWS : headRows ?? NO_ROWS) : requested;
  const names = useMemo(() => playerNames(rows, registry.data?.players), [rows, registry.data]);
  const [visibleRows, setVisibleRows] = useState<readonly ApmSeries[]>(NO_ROWS);
  const wanted = useMemo(() => visibleRows.flatMap(row => row.tag ? [{ tag: row.tag, hour: row.hour }] : []), [visibleRows]);
  const labels = useCountryLabelsAt(runtime.client, track, wanted, active && !graph);
  const graphTags = useMemo(() => tiled && !players ? rows.flatMap(row => row.tag ? [row.tag] : []) : NO_TAGS, [tiled, players, rows]);
  const identityHour = useIdentityHour(runtime.client, track, graphTags, runtime.clock, active && tiled && !players,
    hour => track ? branchHour(track.branch, hour + track.axisOffset) : null);
  const holdersAt = active && tiled && !players ? identityHour : null;
  const holders = useMemo(() => graph && !players ? apmHolders(rows, holdersAt) : [], [graph, players, rows, holdersAt]);
  const looks = useHolderLooks(runtime.client, track, holders, active && graph, tokens);
  const styles = useMemo(() => lineStyles((registry.data?.players ?? []).map((player, preferred) => ({ key: player.id, preferred })), tokens.categories), [registry.data, tokens.categories]);
  const lines = useMemo(() => rows.map(row => ({ row, runs: apmRuns(row), paused: apmRuns(row, true),
    color: players ? styles.get(row.id)!.color : looks.colour(row.id),
    primary: players ? styles.get(row.id)!.primary : [], secondary: players ? styles.get(row.id)!.secondary : SECONDARY_DASH })), [rows, looks, players, styles]);
  const readRates = rates.read, readPaused = pausedRates.read;
  const layers = useMemo<ChartLayer[]>(() => lines.map(line => tiled ? {
    draw: (ctx, frame, chartTokens, state) => {
      const summary = readRates()?.get(line.row.id)?.summary;
      if (summary) drawLineSummary(ctx, frame, chartTokens, summary, line.color, true, state.playhead, { dash: line.primary, endDot: true });
    },
    extent: x => { const summary = readRates()?.get(line.row.id)?.summary; return summary ? summaryExtent(summary, x, null) : null; },
  } : {
    draw: (ctx, frame, chartTokens, state) => { drawLine(ctx, frame, chartTokens, line.runs, line.color, { held: true, dash: line.primary, endDot: true, until: state.playhead ?? undefined }); },
    extent: x => extentOfAll([line.runs], x[0], x[1]),
  }), [lines, tiled, readRates]);
  const strip = useMemo<ChartStrip | undefined>(() => paused ? {
    height: height => Math.min(100, Math.max(48, height * 0.24)),
    draw: (ctx, frame, chartTokens, state) => {
      if (!frame.strip) return;
      const { top, height } = frame.strip;
      const until = Math.min(frame.x[1], state.playhead ?? Infinity);
      const summaries = tiled ? lines.map(line => readPaused()?.get(line.row.id)?.summary) : [];
      const extent = tiled
        ? summaries.reduce<[number, number] | null>((most, summary) => {
          const shown = summary ? summaryExtent(summary, [frame.x[0], until], null) : null;
          return shown ? [Math.min(most?.[0] ?? shown[0], shown[0]), Math.max(most?.[1] ?? shown[1], shown[1])] : most;
        }, null)
        : extentOfAll(lines.map(line => line.paused), frame.x[0], until);
      const maximum = Math.max(1, extent?.[1] ?? 1);
      const toY = (value: number) => top + height - (value / maximum) * height;
      const pausedFrame: PlotFrame = { ...frame, plotTop: top, plotHeight: height, strip: null, y: [0, maximum], log: false, toY,
        valueAt: y => (top + height - y) / height * maximum };
      ctx.save();
      ctx.strokeStyle = chartTokens.axis; ctx.lineWidth = chartTokens.lineWidth.grid;
      ctx.beginPath(); ctx.moveTo(frame.plotLeft, top + height); ctx.lineTo(frame.plotLeft + frame.plotWidth, top + height); ctx.stroke();
      ctx.fillStyle = chartTokens.muted; ctx.font = font(chartTokens, chartTokens.fontSize.small); ctx.textAlign = 'right'; ctx.textBaseline = 'top';
      ctx.fillText('Paused', frame.plotLeft - 6, top); ctx.fillText('orders', frame.plotLeft - 6, top + chartTokens.fontSize.small * 1.3);
      ctx.textBaseline = 'bottom'; ctx.fillText(`0–${Math.ceil(maximum).toLocaleString()}`, frame.plotLeft - 6, top + height);
      ctx.beginPath(); ctx.rect(frame.plotLeft, top, frame.plotWidth + 2, height); ctx.clip();
      lines.forEach((line, at) => {
        const summary = summaries[at];
        if (tiled) { if (summary) drawLineSummary(ctx, pausedFrame, chartTokens, summary, line.color, true, state.playhead, { dash: line.secondary, endDot: true }); }
        else drawLine(ctx, pausedFrame, chartTokens, line.paused, line.color, { held: true, dash: line.secondary, endDot: true, until: state.playhead ?? undefined });
      });
      ctx.restore();
    },
  } : undefined, [paused, lines, tiled, readPaused]);
  const chartData = useMemo<ChartData | undefined>(() => tiled ? { revision: () => [readRates(), readPaused()],
    subscribe: listener => { const stops = [rates.subscribe(listener), pausedRates.subscribe(listener)]; return () => stops.forEach(stop => stop()); } } : undefined,
  [tiled, readRates, readPaused, rates.subscribe, pausedRates.subscribe]);
  const tileView = rates.onView, pausedView = pausedRates.onView;
  const onView = useCallback((next: { from: number; to: number; end: number; width: number }) => {
    if (tiled) { tileView(next); pausedView(next); return; }
    const value = { identity: query.identity, from: Math.floor(next.from), to: Math.floor(next.to), width: Math.ceil(next.width) };
    setView(previous => previous && previous.identity === value.identity && previous.from === value.from && previous.to === value.to && previous.width === value.width ? previous : value);
  }, [query.identity, tiled, tileView, pausedView]);
  const liveHead = useMemo(() => following && track ? { read: () => runtime.clock.getSnapshot().position + track.axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined,
    [following, runtime.clock, track?.axisOffset]);
  const pausedHead = useAtHead(runtime.clock, graph && following, hour => runtime.clock.getSnapshot().playing ? null : Math.floor(hour));
  const stillView = track && first != null && last != null ? apmPausedView(pausedHead === null ? null : pausedHead + track.axisOffset, first, last, TRAILING_HOURS) : null;
  const columns = useMemo<readonly TableColumn<ApmSeries>[]>(() => [
    { key: 'name', header: players ? 'Player' : 'Country', label: players ? 'Player' : 'Country', width: 'minmax(var(--table-name-min), 2fr)',
      value: row => players ? names.get(row.id)! : row.tag,
      render: row => players ? <span title={names.get(row.id)!}>{names.get(row.id)!}</span> : row.tag ? <CountryCell {...labels.labelAt(row.hour)(row.tag)} /> : 'Unknown' },
    ...(players ? [{ key: 'country', header: 'Country', label: 'Country', width: 'minmax(var(--table-name-min), 1fr)', priority: 2,
      value: (row: ApmSeries) => row.tag, render: (row: ApmSeries) => row.tag ? <CountryCell {...labels.labelAt(row.hour)(row.tag)} /> : 'Unknown' }] : []),
    { key: 'apm', header: 'eAPM', label: 'eAPM', hint: 'Orders per unpaused real minute', numeric: true, bar: 'visible', width: 'max-content',
      value: row => apmSortValue(row.summary), render: row => row.summary.gaps ? <span title={GAPS_NOTE}>{apmValueText(row.summary)}</span> : apmValueText(row.summary),
      paint: value => { const { apm, gaps } = apmFromSort(value as bigint); return gaps ? `${apmText(apm)}*` : apmText(apm); } },
    { key: 'orders', header: 'Orders', label: 'Orders', hint: 'Player-issued gameplay orders during unpaused play', numeric: true, width: 'max-content', priority: 1,
      value: row => row.summary.orders === null ? null : BigInt(row.summary.orders), render: row => countText(row.summary.orders), paint: value => countText(String(value)) },
    ...(paused ? [{ key: 'paused', header: 'Paused orders', label: 'Paused orders', numeric: true, width: 'max-content', priority: 1,
      value: (row: ApmSeries) => row.summary.pausedOrders === null ? null : BigInt(row.summary.pausedOrders), render: (row: ApmSeries) => countText(row.summary.pausedOrders),
      paint: (value: NonNullable<ReturnType<typeof apmSortValue>> | string | number) => countText(String(value)) }] : []),
    { key: 'minutes', header: 'Unpaused minutes', label: 'Unpaused minutes', numeric: true, width: 'max-content', priority: 3,
      value: row => row.summary.unpausedMinutes, render: row => apmText(row.summary.unpausedMinutes), paint: value => apmText(Number(value)) },
  ], [players, paused, labels, names]);
  useDiagnostics(onDiagnostics, headEntry ? branchSeries.data?.warnings ?? [] : data?.warnings ?? []);
  const showing = data && !query.error && (!players || registry.data) ? query.answered?.lastHour ?? null : null;
  useEffect(() => { if (!headEntry) shown(showing); }, [shown, showing, headEntry]);

  const body = (() => {
    if (!track) return <WidgetState kind="empty" icon="chart-bars" message="Load a campaign to see eAPM." />;
    if (refused) return <WidgetState kind="unavailable" message={refused} />;
    if (!query.window || first == null || last == null || first > last) return <WidgetState kind="outside" />;
    if (query.groups.error && widget.countries.kind === 'group') return <WidgetState kind="error" failure={query.groups.error} onRetry={query.groups.retry} />;
    if (query.reason && !headEntry) return <WidgetState kind="unavailable" message={query.reason} />;
    if (players && registry.error) return <WidgetState kind="error" failure={registry.error} onRetry={registry.retry} />;
    if (headRefused) return <WidgetState kind="unavailable" message={headRefused} />;
    if (headEntry) {
      if (tiled && branchSeries.error) return <WidgetState kind="error" failure={branchSeries.error} onRetry={branchSeries.retry} />;
      if (headEntry.answer === undefined || (players && !registry.data) || (tiled && !branchSeries.data)) return <WidgetState kind="loading" label="Loading eAPM…" />;
      if (tiled && !branchSeries.data!.available) return <WidgetState kind="unavailable" message="Player orders were not recorded in this campaign." />;
    } else {
      if (query.error) return <WidgetState kind="error" failure={query.error} onRetry={query.retry} />;
      if (!data || (players && !registry.data) || !query.known) return <WidgetState kind="loading" label="Loading eAPM…" />;
      if (!data.available) return <WidgetState kind="unavailable" message="Player orders were not recorded in this campaign." />;
    }
    if (!rows.length && !graph) return <WidgetState kind="empty" message="No eAPM is available for these countries, players and dates." />;
    const period = headEntry ? 'The week to the head' : `${dayLabel(query.answered?.firstHour ?? first)} – ${dayLabel(query.answered?.lastHour ?? last)}`;
    if (!graph) return <Table rowRevision={row => [row, columns]} label="eAPM over the selected period" rows={rows} rowKey={rowKey} columns={columns} live={live}
      sort={tableSortOf(options.tableSort, DEFAULT_SORT)} setOption={setOption} cellBars={options.cellBars === true} rank rankMarkers={options.rankMarkers === true}
      density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)} hoursPerSecond={motion}
      onVisibleRows={setVisibleRows} tooltip={row => {
        const value = current(row);
        return { ...countryNameDetails(players ? null : labels.labelAt(row.hour)(row.tag ?? '')), title: players ? names.get(row.id)! : labels.labelAt(row.hour)(row.tag ?? '').name, subtitle: period,
          rows: [{ label: 'eAPM', value: apmValueText(value) }, { label: 'Unpaused orders', value: countText(value.orders) },
            ...(paused ? [{ label: 'Paused orders', value: countText(value.pausedOrders) }] : []),
            { label: 'Unpaused real minutes', value: apmText(value.unpausedMinutes) }], note: valueNote(value) };
      }} />;
    const label = (row: ApmSeries) => players ? names.get(row.id)! : looks.name(row.id);
    return <div className="stacked-share-chart"><div className="stacked-share-plot">
      <TimeChart key={query.identity} label="eAPM: player-issued orders per unpaused real minute" extent={stillView ?? [first, Math.max(first + 1, last)]}
        playhead={following ? query.playbackHour + track.axisOffset : null} liveHead={liveHead}
        playback={following && !stillView ? 'window' : 'cursor'} windowHours={TRAILING_HOURS} branchStart={track.branch.firstHour}
        axes={{ xFrom: null, xTo: null, yMin: options.yMin as number | null, yMax: options.yMax as number | null, log: false }}
        layers={layers} strip={strip} active={active} notRecorded={tiled && rates.unrecorded && (!paused || pausedRates.unrecorded) ? rates.unrecorded : null}
        revision={[data, branchSeries.data, looks.revision, paused]} data={chartData} hoursPerSecond={motion} onSummaryView={onView}
        formatValue={value => `${apmText(value)} eAPM`}
        tooltip={at => {
          if (tiled) {
            const [rates, pausedOrders] = [readRates(), readPaused()];
            const bins = lines.map(line => [tiledBin(rates?.get(line.row.id)?.summary, at), paused ? tiledBin(pausedOrders?.get(line.row.id)?.summary, at) : null] as const);
            return fitTooltip([{ ...countryNameDetails(...(players ? [] : lines.map(line => ({ nameFailure: looks.nameFailure?.(line.row.id) })))), title: binsLabel(at, bins.flat()),
              rows: lines.flatMap((line, index) => {
                const [rate, orders] = bins[index];
                return [{ label: `${label(line.row)} · eAPM`, value: rate === null ? 'Unknown' : apmText(rate.firstValue), swatch: line.color },
                  ...(paused ? [{ label: `${label(line.row)} · paused orders`, value: orders === null ? 'Unknown' : Math.round(orders.firstValue).toLocaleString(), swatch: line.color }] : [])];
              }), note: APM_EXPLANATION }], viewport);
          }
          const points = lines.map(line => ({ line, point: pointAt(line.row, at) }));
          return fitTooltip([{ ...countryNameDetails(...(players ? [] : points.map(({ line }) => ({ nameFailure: looks.nameFailure?.(line.row.id) })))), title: dayLabel(at), rows: points.flatMap(({ line, point }) => [
            { label: `${label(line.row)} · eAPM`, value: apmValueText(point?.value), swatch: line.color },
            ...(paused ? [{ label: `${label(line.row)} · paused orders`, value: countText(point?.value.pausedOrders), swatch: line.color }] : [])]),
          note: points.some(({ point }) => point?.value.gaps) ? `${APM_EXPLANATION} * ${GAPS_NOTE}` : APM_EXPLANATION }], viewport);
        }} />
    </div><Legend label="eAPM" always items={lines.map(line => ({ id: line.row.id, label: label(line.row), color: line.color,
      value: headEntry && store
        ? <FollowedText entry={headEntry} write={(entry, cell) => {
          const value = apmHeadRows(entry.answer).find(held => held.id === line.row.id)?.summary;
          cell.put(value ? `${apmValueText(value)} eAPM` : '—', value ? valueNote(value) : entry.answer?.kind === 'notRecorded' ? entry.answer.reason : 'Not played in the week to the head');
        }} />
        : `${apmValueText(line.row.summary)} eAPM`,
      tooltip: headEntry ? APM_EXPLANATION : valueNote(line.row.summary) }))} /></div>;
  })();
  return <div ref={setRoot} style={FILL}>{body}</div>;
}
