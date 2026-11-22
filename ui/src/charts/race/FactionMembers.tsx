import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { GatedHour } from '../../dashboard/regionTime.ts';
import type { SeriesGate } from '../../dashboard/keyedSeries';
import { useCallback, useMemo, useRef } from 'react';
import WidgetState from '../../components/WidgetState';
import TimeChart from '../TimeChart';
import type { ChartData, ChartLayer, LiveHead, TimeChartProps } from '../TimeChart';
import DrillParts from '../drill/DrillParts';
import type { DrillPart, LiveDrillParts } from '../drill/DrillParts';
import { dayLabel, drawBands } from '../draw';
import type { ChartSummary } from '../chartSummary';
import { binAt, binsLabel, summaryStack } from '../chartSummary';
import { compactStackExtent, forEachCompactStackStretch } from '../stack';
import { shownUntil } from '../view';
import type { PlaybackMode } from '../view';
import { motionOf } from '../easing.ts';
import { useChartTokens } from '../useChartTokens';
import type { Live } from '../live';
import { useCountryLooks } from '../../dashboard/countryLooks';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { OptionValue } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import type { FactionGroup, HourSpan, SeriesStateReply } from '../../generated/dashboard';
import { useMeasureSeries } from '../../widgets/country-measure/measureSeries';
import { partColours } from './countryRace';
import { factionMembersAt, factionMemberSpans, stateValues } from './factionTotalsData.ts';
import { productRequest } from '../../dashboard/request.ts';

export interface FactionMembersProps {
  runtime: WidgetRuntime; track: BoundTrack; active: boolean; options: Readonly<Record<string, OptionValue>>;
  kind: string; measure: string; label: string; format: (value: number) => string;
  faction: FactionGroup | null;
  extent: [number, number]; playhead: number | null;
  hour: number;
  gate: SeriesGate;
  at: GatedHour | null;
  head: Live<ReadonlyMap<string, number>> | null;
  onPick?: (hour: number) => void;
}

function validateStates(value: unknown): SeriesStateReply {
  const reply = value as SeriesStateReply;
  if (!reply || typeof reply !== 'object' || !Array.isArray(reply.states) || typeof reply.decimals !== 'number') throw new Error('The member state reply is malformed.');
  return reply;
}

export default function FactionMembers({ runtime, gate, at: askAt, track, active, options, kind, measure, label, format, faction, extent, playhead, hour, head, onPick }: FactionMembersProps) {
  const tokens = useChartTokens();
  const spans = useMemo(() => faction ? factionMemberSpans(faction) : new Map<string, HourSpan[]>(), [faction]);
  const tags = useMemo(() => [...spans.keys()].sort(), [spans]);
  const at = useMemo(() => faction ? factionMembersAt(faction, hour) : [], [faction, hour]);
  const atKey = at.join(',');
  const lines = useMeasureSeries(runtime.client, gate, track.source, track.branch, kind, measure, tags, null, null, active, false, spans, track.branch.lastHour, head !== null);
  const looks = useCountryLooks(runtime.client, track, tags, hour, active);
  const colours = useMemo(() => partColours(tags.map(tag => ({ id: tag, label: tag })), tokens.categories), [tags, tokens.categories]);
  const asked = useProduct<SeriesStateReply>(runtime.client, head === null && at.length && askAt !== null ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'series-state',
    params: { branchId: track.branch.id, family: { kind, measure }, reads: at.map(tag => ({ country: countryFromKey(tag), hour: askAt })) } }) : null, active, validateStates, track.source);
  const summaries = useCallback((): ChartSummary[] | null => {
    const out: ChartSummary[] = [];
    for (const tag of tags) {
      const line = lines.lineOf(tag);
      if (!line) return null;
      out.push(line);
    }
    return out.length && out.every(line => line.buckets.length === out[0].buckets.length) ? out : null;
  }, [tags, lines.lineOf]);
  const held = useRef<{ key: readonly ChartSummary[]; stack: ReturnType<typeof summaryStack> } | null>(null);
  const stackOf = useCallback(() => {
    const now = summaries();
    if (!now) return null;
    if (!held.current || held.current.key.length !== now.length || held.current.key.some((line, index) => line !== now[index]))
      held.current = { key: now, stack: summaryStack(now) };
    return held.current.stack;
  }, [summaries]);
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);
  const colourList = tags.map(tag => colours.get(tag) ?? tokens.muted);
  const layers = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, chartTokens, state) => {
      const stack = stackOf();
      if (!stack) return;
      forEachCompactStackStretch(stack.model, frame.x[0], Math.min(frame.x[1], state.playhead ?? frame.x[1]), (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth),
        segment => drawBands(ctx, frame, chartTokens, segment.hours, segment.bands, colourList, segment), stack.sample);
    },
    extent: x => {
      const stack = stackOf();
      const end = Math.min(x[1], until ?? x[1]);
      return stack && end >= x[0] ? compactStackExtent(stack.model, x[0], end) : null;
    },
  }], [stackOf, colourList.join('|'), until]);
  const data = useMemo<ChartData>(() => ({ revision: () => summaries() ?? [], subscribe: lines.subscribe }), [summaries, lines.subscribe]);
  const onSummaryView = useCallback<NonNullable<TimeChartProps['onSummaryView']>>(view => lines.onSummaryView(view), [lines.onSummaryView]);
  const liveHead = useMemo<LiveHead | undefined>(() => head ? { read: () => runtime.clock.getSnapshot().position + track.axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined,
    [head, runtime.clock, track.axisOffset]);
  const rowsOf = useCallback((values: ReadonlyMap<string, number> | undefined): DrillPart[] => at.map(tag => {
    const value = values?.get(tag);
    return { id: tag, label: looks.name(tag), value: value ?? null, color: colours.get(tag), icon: looks.flag(tag), ...(value === undefined ? { text: 'Not known' } : {}) };
  }), [atKey, looks, colours]);
  const live = useMemo<LiveDrillParts | undefined>(() => head ? {
    read: () => {
      const parts = rowsOf(head.read());
      const total = parts.every(part => part.value !== null) ? parts.reduce((sum, part) => sum + part.value!, 0) : null;
      return { parts, total, totalText: total === null ? 'Not known' : format(total) };
    },
    subscribe: head.subscribe,
  } : undefined, [head, rowsOf, format]);
  const atHour = rowsOf(head ? undefined : stateValues(asked.data, hour));
  const total = atHour.every(part => part.value !== null) ? atHour.reduce((sum, part) => sum + part.value!, 0) : null;
  if (!faction) return <WidgetState kind="unavailable" message="Faction unavailable at the selected time." />;
  return <div className="parts-race-breakdown">
    {lines.error ? <WidgetState kind="error" what="the members' lines" failure={lines.error} onRetry={lines.retry} /> : null}
    <div className="country-race-chart">
      <TimeChart label={`${label} by member`} extent={extent} playhead={playhead} playback={playback} afterPlayhead={options.afterPlayhead === true}
        windowHours={Number(options.windowDays ?? 365) * 24} hoursPerSecond={motionOf(runtime.clock.getSnapshot())} branchStart={track.branch.firstHour}
        liveHead={liveHead} axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }} layers={layers} active={active} notRecorded={lines.notRecorded}
        revision={`${faction.id}:${tags.join(',')}`} data={data} onSummaryView={onSummaryView} formatValue={format}
        onPick={onPick ? picked => onPick(Math.floor(picked)) : undefined}
        tooltip={time => {
          const drawn = summaries();
          const bins = tags.map((_, index) => binAt(drawn?.[index], time));
          return { title: binsLabel(time, bins), rows: tags.flatMap((tag, index) => {
            const bucket = bins[index];
            return bucket ? [{ label: looks.name(tag), swatch: colours.get(tag), value: format(bucket.lastValue) }] : [];
          }) };
        }} />
    </div>
    <div className="parts-race-details">
      {!head && <p className="drill-day">{dayLabel(hour)}</p>}
      {!at.length ? <p className="drill-empty">No member on {dayLabel(hour)}.</p>
        : <DrillParts parts={atHour} live={live} format={format} active={active} positiveBars
          total={head ? undefined : total} totalRow={{ label: 'Faction total', value: total === null ? 'Not known' : format(total) }} />}
      {asked.error ? <WidgetState kind="error" what="the members' values" failure={asked.error} onRetry={asked.retry} /> : null}
    </div>
  </div>;
}
