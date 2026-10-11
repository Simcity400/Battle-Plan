import { useMemo } from 'react';
import { Segmented } from '../../components/Segmented';
import DrillParts from '../../charts/drill/DrillParts';
import type { DrillPart } from '../../charts/drill/DrillParts';
import { dayLabel, drawBands, drawPlayhead } from '../../charts/draw';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer, ChartPick } from '../../charts/TimeChart';
import type { ChartTokens } from '../../charts/tokens';
import { fitTooltip } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import type { TooltipRow } from '../../charts/tooltip';
import { fixedNumber } from '../../charts/valueAxis';
import type { StabilityPart, StabilityPartsReply } from '../../generated/dashboard';
import { formatDateTime } from '../../dashboard/calendar';
import type { HourSpan } from '../../generated/dashboard';
import { engineSum, partLayers, partName, partsAt, partsStack, partsWithin, REMAINDER_LABEL, stackRange, visibleSegment } from './partsData';
import { formatPercent } from './stabilityData';
import type { Measure } from './stabilityData';

export const MEASURE_NAMES: Record<Measure, string> = { stability: 'Stability', warSupport: 'War support' };

/** The drill view of Stability & War support: the chosen
 * measure's parts at one day as bars with their share and the total, and the
 * same parts over time as a stacked chart whose click picks another day. */
export default function StabilityDrill({ reply, numbers, pickOwner, presentationReady, measure, onMeasure, hour, onPick, onContextMenu, extent, playback, branchStart, hoursPerSecond, active, tokens, within, readAt }: {
  /** The widget's own playback: its playhead and settings, so the parts chart
   * shows the same hours as the main chart (grow to now, the whole campaign up to
   * the playhead or past it, or a recent window). */
  playback: { playhead: number | null; mode: PlaybackMode; afterPlayhead: boolean; windowHours: number };
  /** Right-click on a day: the widget's Jump to. */
  onContextMenu?: (hour: number, clientX: number, clientY: number) => void;
  /** No tabs without it: a pinned breakdown shows the measure its Show setting names. */
  reply: StabilityPartsReply; numbers: StabilityPartsReply; pickOwner: unknown; presentationReady: boolean; measure: Measure; onMeasure?: (measure: Measure) => void;
  /** Hours the country is shown (a group's membership); null for all. */
  within: readonly HourSpan[] | null;
  /** The hour of the read behind an hour's values, when it was carried from an earlier one. */
  readAt?: (hour: number) => number | null;
  /** The day shown, or null when none is chosen yet. */
  hour: number | null; onPick?: (hour: number) => void;
  extent: readonly [number, number]; branchStart: number; hoursPerSecond: number; active: boolean; tokens: ChartTokens;
}) {
  const viewport = useViewportHeight();
  const names = useMemo(() => new Map(reply[measure].parts.map(part => [part.id, part])), [reply, measure]);
  const named = (part: StabilityPart) => presentationReady ? partName(names.get(part.id) ?? part, measure)
    : part.term || part.id === 'remainder' || part.id === 'clamp' ? partName(part, measure) : { name: 'Not identified', recorded: false };
  const chartOwner = useMemo(() => ({ parent: pickOwner, numbers, actor: reply.tag, measure, membership: JSON.stringify(within) }),
    [pickOwner, numbers, reply.tag, measure, within]);
  const until = shownUntil(playback.playhead, playback.mode, playback.afterPlayhead);
  // Only the hours the widget shows the country: its range, and a group's membership.
  // The playback cutoff is the chart's own, so playback never rebuilds the stack.
  const parts = useMemo(() => partsWithin(partsWithin(numbers[measure], [{ firstHour: extent[0], lastHour: extent[1] }]), within),
    [numbers, measure, within, extent[0], extent[1]]);
  const decimals = numbers.decimals;
  const layers = useMemo(() => partLayers(parts, measure, tokens.categories, tokens.muted), [parts, measure, tokens.categories, tokens.muted]);
  const colorOf = useMemo(() => new Map(layers.flatMap(layer => layer.members.map(id => [id, layer.color] as const))), [layers]);
  // The whole branch is stacked once; the chart clips it to its view.
  const segments = useMemo(() => partsStack(parts, layers, decimals, extent[0], extent[1]), [parts, layers, decimals, extent[0], extent[1]]);
  const chartLayers = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, chartTokens, state) => {
      const colors = layers.map(layer => layer.color);
      const end = Math.min(frame.x[1], state.playhead ?? frame.x[1]);
      // The sample kept past the cutoff (so a band reaches it) is clipped away.
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
    extent: x => stackRange(segments, x[0], Math.min(x[1], until ?? x[1])),
  }], [segments, layers, until]);

  const day = hour == null ? null : Math.floor(hour);
  const values = day == null ? null : partsAt(parts, day);
  const percent = (raw: string) => fixedNumber(raw, decimals) * 100;
  const sum = values ? engineSum(values.map(item => item.raw)) : 0n;
  const shown: DrillPart[] = (values ?? []).filter(item => item.part.id !== 'remainder').map(item => {
    const label = named(item.part);
    return { id: item.part.id, label: label.name, muted: label.recorded, value: percent(item.raw), text: formatPercent(item.raw, decimals, true), color: colorOf.get(item.part.id) };
  });
  const remainder = values?.find(item => item.part.id === 'remainder');
  // Values carried from an earlier read say when they were read, as the hover does.
  const read = day != null && values && readAt ? readAt(day) : null;

  return <div className="stability-drill">
    {onMeasure && <Segmented label="Breakdown of" value={measure} onChange={onMeasure}
      options={(['stability', 'warSupport'] as const).map(value => ({ value, label: MEASURE_NAMES[value] }))} />}
    {day == null ? <p className="drill-empty">Choose a day.</p>
      : values == null ? <p className="drill-empty">No breakdown was recorded for {dayLabel(day)}.</p>
        : <DrillParts parts={shown} format={value => `${value > 0 ? '+' : ''}${value.toFixed(1)}%`} total={percent(sum.toString())}
          unattributed={remainder ? { label: REMAINDER_LABEL, value: percent(remainder.raw), text: formatPercent(remainder.raw, decimals, true), color: colorOf.get('remainder') } : undefined}
          totalRow={{ label: 'Total', value: formatPercent(sum.toString(), decimals) }} />}
    {read != null && day != null && read < day && <p className="drill-empty">As of {formatDateTime(read)}.</p>}
    {!presentationReady && <p className="drill-empty">Loading names…</p>}
    <div className="stability-drill-chart">
      <TimeChart label={`${MEASURE_NAMES[measure]} parts over time`} extent={[extent[0], extent[1]]} playhead={playback.playhead} playback={playback.mode}
        afterPlayhead={playback.afterPlayhead} windowHours={playback.windowHours}
        hoursPerSecond={hoursPerSecond} branchStart={branchStart} axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }}
        overlay={(ctx, frame, chartTokens) => { if (day != null && day >= frame.x[0] && day <= frame.x[1]) drawPlayhead(ctx, frame, chartTokens, day); }}
        layers={chartLayers} active={active} pickOwner={chartOwner} revision={`${measure}:${day}:${segments.length}`}
        formatValue={value => `${Number.isInteger(value) ? value : value.toFixed(1)}%`}
        tooltip={at => {
          const known = partsAt(parts, at);
          if (!known) return fitTooltip([{ title: dayLabel(at), rows: [{ label: MEASURE_NAMES[measure], value: 'Not recorded', tone: 'muted' }] }], viewport);
          const rows: TooltipRow[] = [...known].sort((a, b) => Math.abs(percent(b.raw)) - Math.abs(percent(a.raw)))
            .map(item => ({ label: named(item.part).name, swatch: colorOf.get(item.part.id), value: formatPercent(item.raw, decimals, true), short: true }));
          const total = engineSum(known.map(item => item.raw));
          rows.push({ label: 'Total', value: formatPercent(total.toString(), decimals), strong: true });
          // Keep the date of carried parts.
          const readThen = readAt ? readAt(at) : at;
          return fitTooltip([{ title: dayLabel(at), rows, note: readThen !== null && readThen < at ? `As of ${formatDateTime(readThen)}.` : undefined }], viewport, { keep: 1 });
        }}
        onPick={onPick ? (picked, context?: ChartPick) => { if (context?.frame) onPick(picked); } : undefined} onContextMenu={onContextMenu} />
    </div>
  </div>;
}
