import type { Gate } from '../../dashboard/regionGate';
import { useMemo, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import { drawLineSummary } from '../../charts/chartSummary';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import TimeChart from '../../charts/TimeChart';
import type { ChartData, ChartLayer } from '../../charts/TimeChart';
import { useChartTokens } from '../../charts/useChartTokens';
import type { WidgetProps } from '../../dashboard/definitions';
import { useKeyedSeries } from '../../dashboard/keyedSeries';
import { keyId } from '../../dashboard/keyedSeriesData';
import { contentKey, resolveOptions } from '../../dashboard/options';
import { OPTIONS } from './definition';
import { fieldText, LABELS, measureField, pairOf } from './intelCoverageData';
import type { Basis, ChosenPair, Measure } from './intelCoverageData';

export default function PairHistory({ widget, runtime, active, pair, gate, hour }: WidgetProps & { pair: ChosenPair | null; gate: Gate; hour: number | null }) {
  const tokens = useChartTokens();
  const root = useRef<HTMLDivElement>(null);
  const menu = useContextMenu('Intel network history');
  const options = resolveOptions(OPTIONS, widget.options);
  const chosen = pair ?? pairOf(options.owner ? contentKey(String(options.owner)) : null, options.target ? contentKey(String(options.target)) : null);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const field = measureField((options.measure as Measure) ?? 'covered', (options.basis as Basis) ?? 'controlled');
  const keys = useMemo(() => chosen ? [{ tag: chosen.owner, part: String(chosen.targetId) }] : [], [chosen?.owner, chosen?.targetId]);
  const family = useMemo(() => ({ kind: 'intel-networks', measure: field }), [field]);
  const [hoverHour, setHoverHour] = useState<number | null>(null);
  const series = useKeyedSeries(runtime.client, runtime.clock, gate, track && keys.length ? { source: track.source, branch: track.branch, family, keys, partBits: 32,
    lastHour: track.branch.lastHour } : null, gate.recorded(hour), gate.recorded(hoverHour), false, active, root);
  const layers = useMemo<ChartLayer[]>(() => keys.map(key => ({
    draw: (ctx, frame, chartTokens, state) => {
      const summary = series.summaryOf(keyId(key));
      if (summary) drawLineSummary(ctx, frame, chartTokens, summary, tokens.accent, true, state.playhead ?? null);
    },
  })), [keys, series.summaryOf, tokens.accent]);
  const chartData = useMemo<ChartData>(() => ({ revision: () => [series.revision()], subscribe: series.subscribe }), [series.revision, series.subscribe]);
  if (!track) return <WidgetState kind="empty" icon="eye" message="Load a campaign to see intel networks." />;
  if (!chosen) return <WidgetState kind="empty" icon="eye" message="Choose a pair of countries: click a cell of the matrix." />;
  if (series.error) return <WidgetState kind="error" failure={series.error} onRetry={series.retry} />;
  const valueAt = (at: number) => {
    const line = series.lines.find(item => keyId(item.key) === keyId(keys[0]));
    const run = line?.runs.find(item => item.firstHour <= at && at <= item.lastHour);
    return run ? fieldText(field, Number(run.valueRaw)) : 'Not recorded';
  };
  return <div ref={root} className="intel-pair-history" onContextMenu={event => {
    if (hoverHour === null) return;
    event.preventDefault();
    menu.open(event.clientX, event.clientY, [jumpItem(runtime, track.id, hoverHour)]);
  }}>
    <TimeChart label={`${LABELS[field]} over time`} extent={[track.branch.firstHour, Math.max(track.branch.firstHour + 1, track.branch.lastHour)]}
      playhead={hour} playback="cursor" branchStart={track.branch.firstHour} axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }}
      layers={layers} active={active} notRecorded={series.unrecorded} revision={`${chosen.owner}|${chosen.targetId}|${field}:${series.summaryByKey.size}`} data={chartData}
      onSummaryView={series.onSummaryView} onHover={setHoverHour}
      tooltip={at => ({ title: dayLabel(at), rows: [{ label: LABELS[field], value: valueAt(Math.floor(at)), swatch: tokens.accent }] })} />
    {menu.menu}
  </div>;
}
