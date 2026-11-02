import { useCallback, useMemo, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import type { ChartStrip } from '../../charts/strip';
import type { PlotFrame } from '../../charts/frame';
import Legend from '../../charts/Legend';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { dayLabel, drawLine, font } from '../../charts/draw';
import { extentOfAll } from '../../charts/series';
import { lineStyles, SECONDARY_DASH } from '../../charts/palette';
import { countryNameDetails, fitTooltip } from '../../charts/tooltip.ts';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { useHolderLooks } from '../../dashboard/holderLooks';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { WidgetProps } from '../../dashboard/definitions';
import { TRAILING_HOURS } from '../../dashboard/model';
import type { ApmSeries } from '../../generated/dashboard';
import { apmRuns, apmText, countText, playerNames, pointAt, valueNote, apmValueText, APM_EXPLANATION, GAPS_NOTE } from './apmData';
import { DEFAULT_SORT } from './definition';
import { useApm, useApmRegistry } from './useApm';

const NO_ROWS: readonly ApmSeries[] = [];
const rowKey = (row: ApmSeries) => row.id;

export function ApmWidget(props: WidgetProps & { graph: boolean }) {
  const { graph, runtime, active, widget, setOption, onDiagnostics } = props;
  const [view, setView] = useState<{ identity: string; from: number; to: number; width: number } | null>(null);
  const query = useApm(props, graph, view);
  const { data, track, options, first, last } = query;
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const motion = usePlaybackMotion(runtime.clock);
  const paused = options.showPausedActions === true;
  const players = options.group === 'player';
  const registry = useApmRegistry(widget, runtime, active && players);
  const rows = players && !registry.data ? NO_ROWS : data?.series ?? NO_ROWS;
  const names = useMemo(() => playerNames(rows, registry.data?.players), [rows, registry.data]);
  const [visibleRows, setVisibleRows] = useState<readonly ApmSeries[]>(NO_ROWS);
  const wanted = useMemo(() => visibleRows.flatMap(row => row.tag ? [{ tag: row.tag, hour: row.hour }] : []), [visibleRows]);
  const labels = useCountryLabelsAt(runtime.client, track, wanted, active && !graph);
  const holders = useMemo(() => graph && !players ? rows.flatMap(row => row.tag ? [{ key: row.id, tag: row.tag, hour: row.hour }] : []) : [], [graph, players, rows]);
  const looks = useHolderLooks(runtime.client, track, holders, active && graph, tokens);
  const styles = useMemo(() => lineStyles((registry.data?.players ?? []).map((player, preferred) => ({ key: player.id, preferred })), tokens.categories), [registry.data, tokens.categories]);
  const lines = useMemo(() => rows.map(row => ({ row, runs: apmRuns(row), paused: apmRuns(row, true),
    color: players ? styles.get(row.id)!.color : looks.colour(row.id),
    primary: players ? styles.get(row.id)!.primary : [], secondary: players ? styles.get(row.id)!.secondary : SECONDARY_DASH })), [rows, looks, players, styles]);
  const layers = useMemo<ChartLayer[]>(() => lines.map(line => ({
    draw: (ctx, frame, chartTokens, state) => { drawLine(ctx, frame, chartTokens, line.runs, line.color, { held: true, dash: line.primary, endDot: true, until: state.playhead ?? undefined }); },
    extent: x => extentOfAll([line.runs], x[0], x[1]),
  })), [lines]);
  const strip = useMemo<ChartStrip | undefined>(() => paused ? {
    height: height => Math.min(100, Math.max(48, height * 0.24)),
    draw: (ctx, frame, chartTokens, state) => {
      if (!frame.strip) return;
      const { top, height } = frame.strip;
      const extent = extentOfAll(lines.map(line => line.paused), frame.x[0], Math.min(frame.x[1], state.playhead ?? Infinity));
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
      for (const line of lines) drawLine(ctx, pausedFrame, chartTokens, line.paused, line.color, { held: true, dash: line.secondary, endDot: true, until: state.playhead ?? undefined });
      ctx.restore();
    },
  } : undefined, [paused, lines]);
  const onView = useCallback((next: { from: number; to: number; width: number }) => {
    const value = { identity: query.identity, from: Math.floor(next.from), to: Math.floor(next.to), width: Math.ceil(next.width) };
    setView(previous => previous && previous.identity === value.identity && previous.from === value.from && previous.to === value.to && previous.width === value.width ? previous : value);
  }, [query.identity]);
  const columns = useMemo<readonly TableColumn<ApmSeries>[]>(() => [
    { key: 'name', header: players ? 'Player' : 'Country', label: players ? 'Player' : 'Country', width: 'minmax(var(--table-name-min), 2fr)',
      value: row => players ? names.get(row.id)! : row.tag,
      render: row => players ? <span title={names.get(row.id)!}>{names.get(row.id)!}</span> : row.tag ? <CountryCell {...labels.labelAt(row.hour)(row.tag)} /> : 'Unknown' },
    ...(players ? [{ key: 'country', header: 'Country', label: 'Country', width: 'minmax(var(--table-name-min), 1fr)', priority: 2,
      value: (row: ApmSeries) => row.tag, render: (row: ApmSeries) => row.tag ? <CountryCell {...labels.labelAt(row.hour)(row.tag)} /> : 'Unknown' }] : []),
    { key: 'apm', header: 'eAPM', label: 'eAPM', hint: 'Orders per unpaused real minute', numeric: true, bar: 'visible', width: 'max-content',
      value: row => row.summary.apm, render: row => row.summary.gaps ? <span title={GAPS_NOTE}>{apmValueText(row.summary)}</span> : apmValueText(row.summary) },
    { key: 'orders', header: 'Orders', label: 'Orders', hint: 'Player-issued gameplay orders during unpaused play', numeric: true, width: 'max-content', priority: 1,
      value: row => row.summary.orders === null ? null : BigInt(row.summary.orders), render: row => countText(row.summary.orders) },
    ...(paused ? [{ key: 'paused', header: 'Paused orders', label: 'Paused orders', numeric: true, width: 'max-content', priority: 1,
      value: (row: ApmSeries) => row.summary.pausedOrders === null ? null : BigInt(row.summary.pausedOrders), render: (row: ApmSeries) => countText(row.summary.pausedOrders) }] : []),
    { key: 'minutes', header: 'Unpaused minutes', label: 'Unpaused minutes', numeric: true, width: 'max-content', priority: 3,
      value: row => row.summary.unpausedMinutes, render: row => apmText(row.summary.unpausedMinutes) },
  ], [players, paused, labels, names]);
  useDiagnostics(onDiagnostics, data?.warnings ?? []);

  if (!track) return <WidgetState kind="empty" icon="chart-bars" message="Load a campaign to see eAPM." />;
  if (!query.window || first == null || last == null || first > last) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (query.groups.error && widget.countries.kind === 'group') return <WidgetState kind="error" failure={query.groups.error} onRetry={query.groups.retry} />;
  if (query.reason) return <WidgetState kind="unavailable" message={query.reason} />;
  if (players && registry.error) return <WidgetState kind="error" failure={registry.error} onRetry={registry.retry} />;
  if (query.error) return <WidgetState kind="error" failure={query.error} onRetry={query.retry} />;
  if (!data || (players && !registry.data) || !query.known || (!graph && query.pending)) return <WidgetState kind="loading" label="Loading eAPM…" />;
  if (!data.available) return <WidgetState kind="unavailable" message="Player orders were not recorded in this campaign." />;
  if (!rows.length && !graph) return <WidgetState kind="empty" message="No eAPM is available for these countries, players and dates." />;
  if (!graph) return <Table rowRevision={row => [row, columns]} label="eAPM over the selected period" rows={rows} rowKey={rowKey} columns={columns}
    sort={tableSortOf(options.tableSort, DEFAULT_SORT)} setOption={setOption} cellBars={options.cellBars === true} rank rankMarkers={options.rankMarkers === true}
    density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)} hoursPerSecond={motion}
    onVisibleRows={setVisibleRows} tooltip={row => ({ ...countryNameDetails(players ? null : labels.labelAt(row.hour)(row.tag ?? '')), title: players ? names.get(row.id)! : labels.labelAt(row.hour)(row.tag ?? '').name, subtitle: `${dayLabel(first)} – ${dayLabel(last)}`,
      rows: [{ label: 'eAPM', value: apmValueText(row.summary) }, { label: 'Unpaused orders', value: countText(row.summary.orders) },
        ...(paused ? [{ label: 'Paused orders', value: countText(row.summary.pausedOrders) }] : []),
        { label: 'Unpaused real minutes', value: apmText(row.summary.unpausedMinutes) }], note: valueNote(row.summary) })} />;
  const label = (row: ApmSeries) => players ? names.get(row.id)! : looks.name(row.id);
  return <div className="stacked-share-chart"><div className="stacked-share-plot">
    <TimeChart key={query.identity} label="eAPM: player-issued orders per unpaused real minute" extent={[first, Math.max(first + 1, last)]}
      playhead={widget.time.mode === 'follow' ? query.playbackHour + track.axisOffset : null}
      playback={widget.time.mode === 'follow' ? 'window' : 'cursor'} windowHours={TRAILING_HOURS} branchStart={track.branch.firstHour}
      axes={{ xFrom: null, xTo: null, yMin: options.yMin as number | null, yMax: options.yMax as number | null, log: false }}
      layers={layers} strip={strip} active={active} revision={[data, looks.revision, paused]} hoursPerSecond={motion} onSummaryView={onView}
      formatValue={value => `${apmText(value)} eAPM`}
      tooltip={at => {
        const points = lines.map(line => ({ line, point: pointAt(line.row, at) }));
        return fitTooltip([{ ...countryNameDetails(...(players ? [] : points.map(({ line }) => ({ nameFailure: looks.nameFailure?.(line.row.id) })))), title: dayLabel(at), rows: points.flatMap(({ line, point }) => [
          { label: `${label(line.row)} · eAPM`, value: apmValueText(point?.value), swatch: line.color },
          ...(paused ? [{ label: `${label(line.row)} · paused orders`, value: countText(point?.value.pausedOrders), swatch: line.color }] : [])]),
        note: points.some(({ point }) => point?.value.gaps) ? `${APM_EXPLANATION} * ${GAPS_NOTE}` : APM_EXPLANATION }], viewport);
      }} />
  </div><Legend label="eAPM" always items={lines.map(line => ({ id: line.row.id, label: label(line.row), color: line.color,
    value: query.pending ? 'Loading…' : `${apmValueText(line.row.summary)} eAPM`, tooltip: valueNote(line.row.summary) }))} /></div>;
}
