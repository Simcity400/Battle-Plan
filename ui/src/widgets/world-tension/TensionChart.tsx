import type { SeriesGate } from '../../dashboard/keyedSeries';
import { useCallback, useEffect, useMemo, useRef } from 'react';
import type { ReactNode } from 'react';
import TimeChart from '../../charts/TimeChart';
import type { ChartData, ChartLayer, LiveHead, TimeChartProps } from '../../charts/TimeChart';
import type { EventBadge } from '../../charts/badges';
import type { ChartSummary } from '../../charts/chartSummary';
import { drawLineSummary, extentOfSummary, summaryStack } from '../../charts/chartSummary';
import { dayLabel, drawBands, drawPlayhead } from '../../charts/draw';
import { motionOf } from '../../charts/easing.ts';
import { compactStackExtent, forEachCompactStackStretch } from '../../charts/stack';
import type { TooltipContent } from '../../charts/tooltip';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { OptionValue } from '../../dashboard/options';
import type { Live, LiveValues } from '../../charts/live';
import { followAtHead } from '../../dashboard/tiles/head.ts';
import { useMeasureSeries } from '../country-measure/measureSeries';
import { CONTRIBUTION_DECIMALS, SUM_CELL, TENSION_CELL, TENSION_DECIMALS, WORLD, contributionCell, shareSummaries } from './tensionData.ts';
import type { TensionContribution } from '../../generated/dashboard';

const WORLD_TAGS = [WORLD];
const formatAxis = (value: number) => `${Math.round(value * 10) / 10}%`;

export interface TensionChartProps {
  runtime: WidgetRuntime; gate: SeriesGate; track: BoundTrack; active: boolean; options: Readonly<Record<string, OptionValue>>;
  label: string; extent: [number, number]; playhead: number | null; following: boolean;
  layers: readonly TensionContribution[];
  colours: readonly string[]; otherColour: string;
  share: boolean;
  live: Live<LiveValues<number>> | null;
  badges?: readonly EventBadge[];
  tooltip: (hour: number, badges: readonly EventBadge[]) => TooltipContent | null;
  marker?: number | null;
  onPick?: (hour: number) => void;
  onHover?: (hour: number | null) => void;
  onContextMenu?: (hour: number, x: number, y: number) => void;
  revision?: unknown;
  children?: ReactNode;
}

export default function TensionChart({ runtime, gate, track, active, options, label, extent, playhead, following, layers, colours, otherColour, share, live,
  badges, tooltip, marker = null, onPick, onHover, onContextMenu, revision, children }: TensionChartProps) {
  const tags = useMemo(() => layers.flatMap(layer => layer.tag ? [layer.tag] : []), [layers]);
  const lineKey = [...tags].sort().join('\u0000');
  const lineTags = useMemo(() => lineKey ? lineKey.split('\u0000') : [], [lineKey]);
  const lastHour = track.branch.lastHour;
  const parts = useMeasureSeries(runtime.client, gate, track.source, track.branch, 'threat-contributions', 'contribution', lineTags, null, null, active, false, undefined, lastHour, following);
  const tension = useMeasureSeries(runtime.client, gate, track.source, track.branch, 'world-tension', 'tension', WORLD_TAGS, null, null, active, false, undefined, lastHour, following);
  const sum = useMeasureSeries(runtime.client, gate, track.source, track.branch, 'threat-sum', 'sum', WORLD_TAGS, null, null, active, false, undefined, lastHour, following);
  useEffect(() => {
    if (!live) return undefined;
    const cell = (key: string, decimals: number) => () => { const value = live.read().get(key); return value === undefined ? null : value / 10 ** decimals; };
    const stops = [followAtHead(tension.head(WORLD), cell(TENSION_CELL, TENSION_DECIMALS)), followAtHead(sum.head(WORLD), cell(SUM_CELL, CONTRIBUTION_DECIMALS)),
      ...layers.flatMap(layer => layer.tag ? [followAtHead(parts.head(layer.tag), cell(contributionCell(layer.slot), CONTRIBUTION_DECIMALS))] : [])];
    return () => stops.forEach(stop => stop());
  }, [live, layers, parts.head, tension.head, sum.head]);
  const readParts = parts.lineOf, readTension = tension.lineOf, readSum = sum.lineOf;
  const shares = useMemo(() => shareSummaries(), []);
  const summaries = useCallback((): { parts: ChartSummary[]; tension: ChartSummary; sum: ChartSummary } | null => {
    const t = readTension(WORLD), s = readSum(WORLD);
    if (!t || !s) return null;
    const out: ChartSummary[] = [];
    for (const tag of tags) {
      const line = readParts(tag);
      if (!line || line.buckets.length !== t.buckets.length) return null;
      out.push(share ? shares(line, t, s) : line);
    }
    return s.buckets.length === t.buckets.length ? { parts: out, tension: t, sum: s } : null;
  }, [tags, share, shares, readParts, readTension, readSum]);
  const held = useRef<{ key: readonly ChartSummary[]; stack: ReturnType<typeof summaryStack> } | null>(null);
  const stackOf = useCallback(() => {
    const now = summaries();
    if (!now) return null;
    const key = [...now.parts, now.tension, now.sum];
    if (!held.current || held.current.key.length !== key.length || held.current.key.some((line, at) => line !== key[at]))
      held.current = { key, stack: summaryStack(now.parts, share ? now.tension : now.sum) };
    return { stack: held.current.stack, tension: now.tension };
  }, [summaries, share]);
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);
  const bandColours = useMemo(() => [...colours, otherColour], [colours, otherColour]);
  const layersDrawn = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, chartTokens, state) => {
      const drawn = stackOf();
      if (!drawn) return;
      const to = Math.min(frame.x[1], state.playhead ?? frame.x[1]);
      ctx.save();
      ctx.globalAlpha = 0.85;
      forEachCompactStackStretch(drawn.stack.model, frame.x[0], to, (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth),
        segment => drawBands(ctx, frame, chartTokens, segment.hours, segment.bands, bandColours, segment), drawn.stack.sample);
      ctx.restore();
      drawLineSummary(ctx, frame, chartTokens, drawn.tension, chartTokens.text, false, state.playhead, { endDot: false });
      if (marker !== null) drawPlayhead(ctx, frame, chartTokens, marker);
    },
    extent: x => {
      const drawn = stackOf();
      const end = Math.min(x[1], until ?? x[1]);
      if (!drawn || end < x[0]) return null;
      const bands = compactStackExtent(drawn.stack.model, x[0], end), line = extentOfSummary(drawn.tension, x[0], end);
      return bands && line ? [Math.min(bands[0], line[0]), Math.max(bands[1], line[1])] : bands ?? line;
    },
  }], [stackOf, bandColours, until, marker]);
  const subscribers = [parts.subscribe, tension.subscribe, sum.subscribe];
  const data = useMemo<ChartData>(() => ({ revision: () => { const now = summaries(); return now ? [...now.parts, now.tension, now.sum] : []; },
    subscribe: wake => { const stops = subscribers.map(subscribe => subscribe(wake)); return () => stops.forEach(stop => stop()); } }),
  [summaries, ...subscribers]);
  const onSummaryView = useCallback<NonNullable<TimeChartProps['onSummaryView']>>(view => {
    parts.onSummaryView(view); tension.onSummaryView(view); sum.onSummaryView(view);
  }, [parts.onSummaryView, tension.onSummaryView, sum.onSummaryView]);
  const offset = track.axisOffset;
  const liveHead = useMemo<LiveHead | undefined>(() => following ? { read: () => runtime.clock.getSnapshot().position + offset, subscribe: runtime.clock.subscribeFrame } : undefined,
    [following, runtime.clock, offset]);
  const failure = parts.error ?? tension.error ?? sum.error;
  return <div className="world-tension-chart">
    <TimeChart label={label} extent={extent} playhead={playhead} playback={playback} afterPlayhead={options.afterPlayhead === true}
      windowHours={Number(options.windowDays ?? 365) * 24} hoursPerSecond={motionOf(runtime.clock.getSnapshot())} branchStart={track.branch.firstHour}
      liveHead={liveHead} axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }}
      fitValues={shown => [Math.min(0, shown?.[0] ?? 0), Math.max(100, shown?.[1] ?? 100)]}
      layers={layersDrawn} active={active} notRecorded={tension.notRecorded} data={data} revision={revision} formatValue={formatAxis}
      badges={badges?.length ? badges : undefined} onSummaryView={onSummaryView}
      onNavigateHour={tension.onNavigateHour} keyboardHour={tension.keyboardHour} keyboardRevision={tension.keyboardRevision}
      onHover={onHover}
      tooltip={(hour, _frame, hovered) => tooltip(hour, hovered) ?? { title: dayLabel(hour), rows: [] }}
      onPick={onPick ? hour => onPick(Math.floor(hour)) : undefined} onContextMenu={onContextMenu} />
    {failure ? <p className="chart-line-key" role="status">{`The chart could not be read: ${failure.message}`}</p> : null}
    {children}
  </div>;
}
