import type { Gate } from '../../dashboard/regionGate';
import { useEffect, useMemo, useRef, useState } from 'react';
import { Segmented } from '../../components/Segmented';
import DrillParts from '../../charts/drill/DrillParts';
import type { DrillPart, LiveDrillParts } from '../../charts/drill/DrillParts';
import { dayLabel, drawBands, drawPlayhead } from '../../charts/draw';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import type { Live } from '../../charts/live';
import TimeChart from '../../charts/TimeChart';
import type { ChartData, ChartLayer, ChartPick } from '../../charts/TimeChart';
import type { ChartTokens } from '../../charts/tokens';
import { fitTooltip } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import type { TooltipRow } from '../../charts/tooltip';
import { fixedNumber } from '../../charts/valueAxis';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import type { PlaybackController } from '../../dashboard/playback';
import WidgetState from '../../components/WidgetState';
import { engineSum, partLayers, partName, partsAt, partsStack, partsWithin, REMAINDER_LABEL, stackRange, visibleSegment } from './partsData';
import { formatPercent } from './stabilityData';
import type { Measure } from './stabilityData';
import { useStabilityParts } from './stabilityParts';
import type { CoveredBranch } from '../../dashboard/regionTime';
import type { SeriesOwner } from '../../dashboard/keyedSeries';
import type { MeasureParts, StabilityPart } from './types';

export const MEASURE_NAMES: Record<Measure, string> = { stability: 'Stability', warSupport: 'War support' };

export default function StabilityDrill({ client, clock, gate, source, branch, lastHour, owner, measure, onMeasure, hour, following, liveHead, onPick,
  onContextMenu, extent, playback, branchStart, hoursPerSecond, active, tokens, onDiagnostics }: {
  client: DashboardClient; clock: PlaybackController; gate: Gate; source: DashboardSource; branch: CoveredBranch & { id: string }; lastHour: number;
  owner: SeriesOwner | null;
  playback: { playhead: number | null; mode: PlaybackMode; afterPlayhead: boolean; windowHours: number };
  onContextMenu?: (hour: number, clientX: number, clientY: number) => void;
  measure: Measure; onMeasure?: (measure: Measure) => void;
  hour: number | null; onPick?: (hour: number) => void;
  following: boolean; liveHead?: Live<number>;
  extent: readonly [number, number]; branchStart: number; hoursPerSecond: number | undefined; active: boolean; tokens: ChartTokens;
  onDiagnostics?: (notes: readonly string[]) => void;
}) {
  const viewport = useViewportHeight();
  const root = useRef<HTMLDivElement>(null);
  const [hover, setHover] = useState<number | null>(null);
  const parts = useStabilityParts(client, clock, gate, source, branch, owner, extent, lastHour, measure, hour, hover, following, active, root);
  const within = owner?.spans ?? null, tag = owner?.key ?? null;
  const shownParts = useMemo<MeasureParts>(() => partsWithin(partsWithin(parts.geometry, [{ firstHour: extent[0], lastHour: extent[1] }]), within),
    [parts.geometry, within, extent[0], extent[1]]);
  const named = (part: StabilityPart) => parts.ready ? partName(part, measure)
    : part.term || part.id === 'remainder' || part.id === 'clamp' ? partName(part, measure) : { name: 'Not identified', recorded: false };
  const chartOwner = useMemo(() => ({ client, tag, measure, membership: JSON.stringify(within), geometry: shownParts }), [client, tag, measure, within, shownParts]);
  const until = shownUntil(playback.playhead, playback.mode, playback.afterPlayhead);
  const decimals = 5;
  const layers = useMemo(() => partLayers(shownParts, measure, tokens.categories, tokens.muted), [shownParts, measure, tokens.categories, tokens.muted]);
  const colorOf = useMemo(() => new Map(layers.flatMap(layer => layer.members.map(id => [id, layer.color] as const))), [layers]);
  const segments = useMemo(() => partsStack(shownParts, layers, decimals, extent[0], extent[1]), [shownParts, layers, extent[0], extent[1]]);
  const newest = useRef<{ geometry: MeasureParts; segments: ReturnType<typeof partsStack> } | null>(null);
  const segmentsAt = (head: number | null) => {
    if (!following) return segments;
    const geometry = parts.geometryAt(head);
    if (newest.current?.geometry !== geometry) {
      const shown = partsWithin(partsWithin(geometry, [{ firstHour: extent[0], lastHour: extent[1] }]), within);
      newest.current = { geometry, segments: partsStack(shown, layers, decimals, extent[0], extent[1]) };
    }
    return newest.current.segments;
  };
  const latest = useRef(segmentsAt); latest.current = segmentsAt;
  const chartData = useMemo<ChartData>(() => ({ revision: () => [parts.revision()], subscribe: parts.subscribe }), [parts.revision, parts.subscribe]);
  const chartLayers = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, chartTokens, state) => {
      const colors = layers.map(layer => layer.color);
      const end = Math.min(frame.x[1], state.playhead ?? frame.x[1]);
      const segments = latest.current(state.playhead === null ? null : end);
      ctx.save();
      ctx.beginPath();
      ctx.rect(frame.plotLeft, frame.plotTop, Math.max(0, frame.toX(end) - frame.plotLeft), frame.plotHeight);
      ctx.clip();
      for (const segment of segments) {
        const shown = visibleSegment(segment, frame.x[0], end);
        if (shown) drawBands(ctx, frame, chartTokens, shown.hours, shown.bands, shown.layers.map(layer => colors[layer]));
      }
      ctx.restore();
    },
    extent: x => stackRange(latest.current(null), x[0], Math.min(x[1], until ?? x[1])),
  }], [segments, layers, until]);
  const percent = (raw: string) => fixedNumber(raw, decimals) * 100;
  const drillRows = (values: MeasureParts, day: number) => {
    const known = partsAt(values, day);
    if (!known) return null;
    const sum = engineSum(known.map(item => item.raw));
    const rows: DrillPart[] = known.filter(item => item.part.id !== 'remainder').map(item => {
      const label = named(item.part);
      return { id: item.part.id, label: label.name, muted: label.recorded, value: percent(item.raw), text: formatPercent(item.raw, decimals, true), color: colorOf.get(item.part.id) };
    });
    const remainder = known.find(item => item.part.id === 'remainder');
    return { parts: rows, total: percent(sum.toString()), totalText: formatPercent(sum.toString(), decimals),
      unattributed: remainder ? { label: REMAINDER_LABEL, value: percent(remainder.raw), text: formatPercent(remainder.raw, decimals, true), color: colorOf.get('remainder') } : undefined };
  };
  const live: LiveDrillParts | undefined = useMemo(() => following && parts.live ? parts.live(values => drillRows(values, 0)
    ?? { parts: [], total: null, totalText: 'Not recorded at this hour' }) : undefined, [following, parts.live, layers, parts.ready]);
  const day = hour == null ? null : Math.floor(hour);
  const values = day == null || live ? null : drillRows(parts.at(day), day);
  const notes = useMemo(() => [...(parts.named.data?.warnings ?? []), ...(parts.error ? [parts.error.message] : [])], [parts.named.data, parts.error]);
  useEffect(() => { onDiagnostics?.(notes); return () => onDiagnostics?.([]); }, [notes, onDiagnostics]);

  return <div className="stability-drill" ref={root}>
    {onMeasure && <Segmented label="Breakdown of" value={measure} onChange={onMeasure}
      options={(['stability', 'warSupport'] as const).map(value => ({ value, label: MEASURE_NAMES[value] }))} />}
    {live ? <DrillParts parts={[]} live={live} format={value => `${value > 0 ? '+' : ''}${value.toFixed(1)}%`} totalRow={{ label: 'Total', value: '' }} />
      : day == null ? <p className="drill-empty">Choose a day.</p>
      : !parts.loadedAt(day) ? <p className="drill-empty">{parts.error ? `The breakdown couldn’t be read: ${parts.error.message}` : 'Loading the breakdown…'}</p>
      : values == null ? <p className="drill-empty">No breakdown was recorded for {dayLabel(day)}.</p>
        : <DrillParts parts={values.parts} format={value => `${value > 0 ? '+' : ''}${value.toFixed(1)}%`} total={values.total}
          unattributed={values.unattributed} totalRow={{ label: 'Total', value: values.totalText }} />}
    {!parts.ready && <p className="drill-empty">Loading names…</p>}
    <div className="stability-drill-chart">
      <TimeChart label={`${MEASURE_NAMES[measure]} parts over time`} extent={[extent[0], extent[1]]} playhead={playback.playhead} playback={playback.mode}
        afterPlayhead={playback.afterPlayhead} windowHours={playback.windowHours} liveHead={liveHead}
        hoursPerSecond={hoursPerSecond} branchStart={branchStart} axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }}
        overlay={live ? undefined : (ctx, frame, chartTokens) => { if (day != null && day >= frame.x[0] && day <= frame.x[1]) drawPlayhead(ctx, frame, chartTokens, day); }}
        onSummaryView={parts.onSummaryView} onHover={setHover}
        onNavigateHour={parts.onNavigateHour} keyboardHour={parts.keyboardHour} keyboardRevision={parts.keyboardRevision}
        layers={chartLayers} data={chartData} active={active} notRecorded={parts.unrecorded} pickOwner={chartOwner} revision={[measure, day, segments, parts.ready]}
        formatValue={value => `${Number.isInteger(value) ? value : value.toFixed(1)}%`}
        tooltip={at => {
          const known = partsAt(parts.at(at), at);
          if (!known) return fitTooltip([{ title: dayLabel(at), rows: [{ label: MEASURE_NAMES[measure], value: gate.recorded(at, within) === null || parts.loadedAt(at) ? 'Not recorded' : 'Loading…', tone: 'muted' }] }], viewport);
          const rows: TooltipRow[] = [...known].sort((a, b) => Math.abs(percent(b.raw)) - Math.abs(percent(a.raw)))
            .map(item => ({ label: named(item.part).name, swatch: colorOf.get(item.part.id), value: formatPercent(item.raw, decimals, true), short: true }));
          rows.push({ label: 'Total', value: formatPercent(engineSum(known.map(item => item.raw)).toString(), decimals), strong: true });
          return fitTooltip([{ title: dayLabel(at), rows }], viewport, { keep: 1 });
        }}
        onPick={onPick ? (picked, context?: ChartPick) => { if (context?.frame) onPick(picked); } : undefined} onContextMenu={onContextMenu} />
    </div>
    {parts.error && <WidgetState kind="error" failure={parts.error} onRetry={parts.retry} />}
  </div>;
}
