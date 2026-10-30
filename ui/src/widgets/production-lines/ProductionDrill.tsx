import { useMemo } from 'react';
import { Segmented } from '../../components/Segmented';
import DrillParts from '../../charts/drill/DrillParts';
import { dayLabel, drawBands, drawPlayhead } from '../../charts/draw';
import { runAt } from '../../charts/series';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import { fitTooltip } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { fixedNumber, formatAmount } from '../../charts/valueAxis';
import type { ProductionLineBreakdownReply } from '../../generated/dashboard';
import { modifierTooltip } from './productionData';

/** One part set serves the hover, bars and history. Negative corrections remain
 * exact rows; only positive amounts draw stacked bands. Picking a day pins it. */
export default function ProductionDrill({ data, selected, onFactor, hour, extent, playhead, active, hoursPerSecond, onPick }: {
  data: ProductionLineBreakdownReply; selected: string; onFactor: (factor: string) => void; hour: number | null;
  extent: [number, number]; playhead: number | null; active: boolean; hoursPerSecond: number | undefined; onPick: (hour: number) => void;
}) {
  const tokens = useChartTokens();
  const height = useViewportHeight();
  const groups = useMemo(() => data.groups.map(group => ({ value: group.id, label: group.label })), [data]);
  const shown = selected;
  const pickOwner = useMemo(() => ({ data, shown }), [data, shown]);
  const colorOf = useMemo(() => {
    const ids = new Set(data.runs.flatMap(run => run.groups.find(group => group.id === shown)?.parts.map(part => part.id) ?? []));
    return new Map([...ids].map((id, index) => [id, id === 'other' ? tokens.muted : tokens.categories[index % tokens.categories.length]]));
  }, [data, shown, tokens.categories, tokens.muted]);
  const segments = useMemo(() => data.runs.flatMap(run => {
    const group = run.groups.find(group => group.id === shown);
    if (!group || group.totalRaw == null) return [];
    let total = 0;
    const bands = group.parts.filter(part => BigInt(part.raw) > 0n).map(part => {
      const low = total;
      total += fixedNumber(part.raw, group.decimals);
      return { low: [low, low], high: [total, total] };
    });
    return [{ hours: [run.firstHour, run.lastHour + 1], bands, total,
      colors: group.parts.filter(part => BigInt(part.raw) > 0n).map(part => colorOf.get(part.id) ?? tokens.muted) }];
  }), [data, shown, colorOf, tokens.muted]);
  const layers = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, chartTokens, state) => {
      const end = Math.min(frame.x[1], state.playhead ?? frame.x[1]);
      ctx.save(); ctx.beginPath(); ctx.rect(frame.plotLeft, frame.plotTop, Math.max(0, frame.toX(end) - frame.plotLeft), frame.plotHeight); ctx.clip();
      for (const segment of segments) if (segment.hours[0] <= end && segment.hours[1] >= frame.x[0]) {
        drawBands(ctx, frame, chartTokens, segment.hours, segment.bands, segment.colors);
      }
      ctx.restore();
    },
    extent: x => {
      const values = segments.filter(segment => segment.hours[0] <= x[1] && segment.hours[1] >= x[0]).map(segment => segment.total);
      return values.length ? [0, values.reduce((maximum, value) => Math.max(maximum, value), 0)] : null;
    },
  }], [segments]);
  const run = hour == null ? null : runAt(data.runs, hour);
  const group = run?.groups.find(group => group.id === shown);
  const unit = group?.unit ?? data.runs.flatMap(run => run.groups).find(group => group.id === shown)?.unit ?? '';
  const valueText = (raw: string) => `${formatAmount(raw, 5, 5)} ${unit}`;
  return <div className="production-drill">
    <Segmented label="Production factor" value={shown} onChange={onFactor} options={groups} />
    {!group ? <p className="drill-empty">No breakdown was recorded for this day.</p>
      : <DrillParts parts={group.parts.map(part => ({ id: part.id, label: part.label ?? 'Other',
        value: fixedNumber(part.raw, group.decimals), raw: BigInt(part.raw), text: valueText(part.raw), muted: part.unknown != null, color: colorOf.get(part.id) }))}
        total={group.totalRaw == null ? null : fixedNumber(group.totalRaw, group.decimals)} format={value => `${value.toFixed(5)} ${unit}`}
        totalRow={{ label: group.label, value: group.totalRaw == null ? 'Not recorded' : valueText(group.totalRaw) }} positiveBars showShares={false} virtual active={active} />}
    <div className="production-drill-chart"><TimeChart label="Production modifier parts over time" extent={extent} playhead={playhead} playback="cursor"
      branchStart={extent[0]} hoursPerSecond={hoursPerSecond} active={active} layers={layers} revision={`${shown}:${hour}`} pickOwner={pickOwner} keyHours={data.reads}
      axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }} formatValue={value => `${value.toFixed(1)} ${unit}`}
      overlay={(ctx, frame, chartTokens) => { if (hour != null) drawPlayhead(ctx, frame, chartTokens, hour); }}
      tooltip={at => {
        const group = runAt(data.runs, at)?.groups.find(group => group.id === shown);
        const forms = modifierTooltip(group, dayLabel(at), id => colorOf.get(id));
        return fitTooltip(forms, height, { full: forms[0] });
      }} onPick={onPick} /></div>
  </div>;
}
