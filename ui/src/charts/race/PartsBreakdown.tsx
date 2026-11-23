import type { GatedHour } from '../../dashboard/regionTime';
import type { SeriesGate } from '../../dashboard/keyedSeries';
import type { Gate } from '../../dashboard/regionGate';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import TimeChart from '../TimeChart';
import type { ChartData, ChartLayer, LiveHead, TimeChartProps } from '../TimeChart';
import DrillParts from '../drill/DrillParts';
import type { DrillPart, LiveDrillParts } from '../drill/DrillParts';
import { drawBands, dayLabel } from '../draw';
import type { ChartSummary } from '../chartSummary';
import { binAt, binsLabel, summaryStack } from '../chartSummary';
import { compactStackExtent, forEachCompactStackStretch } from '../stack';
import { shownUntil } from '../view';
import type { PlaybackMode } from '../view';
import { motionOf } from '../easing.ts';
import { useChartTokens } from '../useChartTokens';
import { partColours } from './countryRace';
import type { DashboardClient } from '../../dashboard/client';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { OptionValue } from '../../dashboard/options';
import { ownerKeys, seriesOwner, useKeyedSeries, useSeriesParts, type SeriesOwner } from '../../dashboard/keyedSeries';
import { keyId } from '../../dashboard/keyedSeriesData.ts';
import { followAtHead } from '../../dashboard/tiles/head.ts';
import { useTiles } from '../../dashboard/tiles/useTiles';
import { mostNamed } from '../../dashboard/tiles/view.ts';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { asFailure } from '../../dashboard/queryError.ts';
import type { MeasureParts, PartValues } from './measureParts';
import { drawnItems } from './measureParts.ts';
import type { PartsHead } from './PartFollows';

class PartLines {
  readonly lines = new Map<string, { read: () => ChartSummary | undefined; subscribe: (wake: () => void) => () => void; onView: TimeChartProps['onSummaryView'] }>();
  private wakes = new Set<() => void>();
  set(id: string, line: PartLines['lines'] extends Map<string, infer L> ? L : never) { this.lines.set(id, line); this.wake(); }
  drop(id: string) { this.lines.delete(id); this.wake(); }
  wake = () => { for (const wake of [...this.wakes]) wake(); };
  subscribe = (wake: () => void) => { this.wakes.add(wake); return () => { this.wakes.delete(wake); }; };
}

function PartTile({ client, track, family, owner, span, id, active, live, lines, head, unrecorded }: { client: DashboardClient; track: BoundTrack; family: Record<string, unknown>;
  owner: SeriesOwner; span: SeriesGate['span']; id: string; active: boolean; live: boolean; lines: PartLines; head: (() => number | null) | null;
  unrecorded: (id: string, reason: string | null | undefined) => void }) {
  const tag = owner.key;
  const within = useMemo(() => owner.spans ? new Map([[tag, owner.spans]]) : undefined, [tag, owner.spans]);
  const tiles = useTiles(client, { source: track.source, branchId: track.branch.id, family, tags: [tag], within, span, lastHour: track.branch.lastHour }, active, live);
  const { read, subscribe, onView } = tiles;
  useEffect(() => {
    lines.set(id, { read: () => read()?.get(tag)?.summary, subscribe, onView: view => { if (view.to >= view.from) onView(view); } });
    const stop = subscribe(lines.wake);
    return () => { stop(); lines.drop(id); };
  }, [lines, id, tag, read, subscribe, onView]);
  useEffect(() => head ? followAtHead(tiles.head(tag), head) : undefined, [tiles.head, tag, head]);
  useEffect(() => { unrecorded(id, tiles.unrecorded); return () => unrecorded(id, undefined); }, [unrecorded, id, tiles.unrecorded]);
  return null;
}

export interface PartsBreakdownProps {
  runtime: WidgetRuntime; track: BoundTrack; active: boolean; options: Readonly<Record<string, OptionValue>>;
  parts: MeasureParts;
  tag: string; name: string;
  extent: [number, number]; playhead: number | null;
  hour: number;
  gate: SeriesGate & { readonly country?: Gate['country'] };
  at: GatedHour | null;
  head: PartsHead | null;
  values: PartValues | undefined;
  totalLabel: string; total?: number | null;
  format: (value: number) => string;
  error?: unknown; onRetry?: () => void;
  onPick?: (hour: number) => void;
}

export default function PartsBreakdown({ runtime, gate, at, track, active, options, parts, tag, name, extent, playhead, hour, head, values,
  totalLabel, total, format, error, onRetry, onPick }: PartsBreakdownProps) {
  const tokens = useChartTokens();
  const shown = useShownHour();
  const following = head !== null;
  const lines = useMemo(() => new PartLines(), []);
  const span = useMemo(() => gate.span(extent[0], extent[1]), [extent[0], extent[1], gate.span]);
  const keyedFamily = parts.kind === 'keyed' ? parts.family as unknown as Record<string, unknown> : null;
  const owner = useMemo(() => seriesOwner(gate.country, tag), [gate.country?.history, tag]);
  const held = useSeriesParts(runtime.client, gate, track.source, track.branch.id, keyedFamily, keyedFamily ? owner : null, keyedFamily ? span : null, active);
  const { keys, within } = useMemo(() => ownerKeys(keyedFamily ? owner : null, (held.data?.parts ?? []).map(part => part.part)), [keyedFamily, held.data, owner]);
  const root = useRef<HTMLDivElement>(null);
  const keyed = useKeyedSeries(runtime.client, runtime.clock, gate, keyedFamily && keys.length ? { source: track.source, branch: track.branch, family: keyedFamily,
    keys, within, partBits: parts.kind === 'keyed' ? parts.partBits : 0, lastHour: track.branch.lastHour } : null, following ? null : at, null, following, active, root);
  const items = useMemo(() => parts.kind === 'measures' ? drawnItems(parts.items).map(item => ({ id: item.id, label: item.label }))
    : keys.map(key => ({ id: key.part, label: parts.name(key.part) })), [parts, keys]);
  const colours = useMemo(() => partColours(items, tokens.categories), [items, tokens.categories]);
  const [partReasons, setPartReasons] = useState<ReadonlyMap<string, string>>(new Map());
  const partUnrecorded = useCallback((id: string, reason: string | null | undefined) => setPartReasons(before => {
    if ((before.get(id) ?? null) === (reason ?? null)) return before;
    const next = new Map(before);
    if (reason) next.set(id, reason); else next.delete(id);
    return next;
  }), []);
  const notRecorded = parts.kind === 'keyed' ? keyed.unrecorded
    : items.length && items.every(item => partReasons.has(item.id)) ? mostNamed(items.map(item => partReasons.get(item.id)!)) : null;
  const summaries = useCallback((): ChartSummary[] | null => {
    const out: ChartSummary[] = [];
    for (const item of items) {
      const summary = parts.kind === 'measures' ? lines.lines.get(item.id)?.read() : keyed.summaryOf(keyId({ tag: owner?.key ?? tag, part: item.id }));
      if (!summary) return null;
      out.push(summary);
    }
    return out.length && out.every(summary => summary.buckets.length === out[0].buckets.length) ? out : null;
  }, [items, parts.kind, lines, keyed.summaryOf, tag, owner?.key]);
  const stacked = useRef<{ summaries: readonly ChartSummary[]; stack: ReturnType<typeof summaryStack> } | null>(null);
  const stackOf = useCallback(() => {
    const now = summaries();
    if (!now) return null;
    if (!stacked.current || stacked.current.summaries.length !== now.length || stacked.current.summaries.some((held, index) => held !== now[index])) {
      stacked.current = { summaries: now, stack: summaryStack(now) };
    }
    return stacked.current.stack;
  }, [summaries]);
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);
  const colourList = items.map(item => colours.get(item.id) ?? tokens.muted);
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
  const data = useMemo<ChartData>(() => ({ revision: () => summaries() ?? [], subscribe: wake => {
    const stops = [lines.subscribe(wake), keyed.subscribe(wake)];
    return () => { for (const stop of stops) stop(); };
  } }), [summaries, lines, keyed.subscribe]);
  const onSummaryView = useCallback<NonNullable<TimeChartProps['onSummaryView']>>(view => {
    if (view.to < view.from) return;
    if (parts.kind === 'keyed') keyed.onSummaryView(view);
    else for (const line of lines.lines.values()) line.onView?.(view);
  }, [parts.kind, keyed.onSummaryView, lines]);
  const offset = track.axisOffset;
  const liveHead = useMemo<LiveHead | undefined>(() => following ? { read: () => runtime.clock.getSnapshot().position + offset, subscribe: runtime.clock.subscribeFrame } : undefined,
    [following, runtime.clock, offset]);
  const rowsOf = useCallback((at: PartValues | undefined): DrillPart[] => items.map(item => {
    const value = at?.get(item.id);
    return { id: item.id, label: item.label, value: value ?? null, color: colours.get(item.id), ...(value == null ? { text: 'Not known' } : {}) };
  }), [items, colours]);
  const catalogued = !keyedFamily || held.data !== undefined;
  const headValues = useCallback(() => head?.values([tag]).get(tag), [head, tag]);
  const live = useMemo<LiveDrillParts | undefined>(() => {
    if (!head) return undefined;
    let made: { version: number; parts: ReturnType<LiveDrillParts['read']> } | null = null;
    return { read: () => {
      if (made?.version !== head.version()) {
        const now = rowsOf(headValues());
        const sum = catalogued && now.every(part => part.value !== null) ? now.reduce((total, part) => total + part.value!, 0) : null;
        made = { version: head.version(), parts: { parts: now, total: sum, totalText: sum === null ? 'Not known' : format(sum) } };
      }
      shown(Math.floor(runtime.clock.getHour()) + offset);
      return made.parts;
    }, subscribe: head.subscribe };
  }, [head, headValues, rowsOf, format, runtime.clock, offset, shown, catalogued]);
  useEffect(() => { if (!head) shown(values ? hour : null); }, [head, values, hour, shown]);
  const partHead = useCallback((id: string) => head ? () => head.values([tag]).get(tag)?.get(id) ?? null : null, [head, tag]);
  const atHour = rowsOf(values);
  const sum = total !== undefined ? total : atHour.every(part => part.value !== null) && values ? atHour.reduce((all, part) => all + part.value!, 0) : null;
  const [failure, retryFailure] = error ? [error, onRetry] : held.error ? [held.error, held.retry] : keyed.error ? [keyed.error, keyed.retry] : [undefined, undefined];
  return <div className="parts-race-breakdown" ref={root}>
    {parts.kind === 'measures' && drawnItems(parts.items).map(item => <PartTile key={item.id} client={runtime.client} track={track} family={item.measure as unknown as Record<string, unknown>}
      owner={owner ?? { key: tag, spans: null }} span={gate.span} id={item.id} active={active} live={following} lines={lines} head={partHead(item.id)} unrecorded={partUnrecorded} />)}
    {failure ? <WidgetState kind="error" what="the breakdown" failure={asFailure(failure)} onRetry={retryFailure} /> : null}
    <div className="country-race-chart">
      <TimeChart label={`${name} by ${parts.label.toLowerCase()}`} extent={extent} playhead={playhead} playback={playback} afterPlayhead={options.afterPlayhead === true}
        windowHours={Number(options.windowDays ?? 365) * 24} hoursPerSecond={motionOf(runtime.clock.getSnapshot())} branchStart={track.branch.firstHour}
        liveHead={liveHead} axes={{ xFrom: null, xTo: null, yMin: (options.yMin as number | null) ?? null, yMax: (options.yMax as number | null) ?? null, log: false }}
        layers={layers} active={active} notRecorded={notRecorded} revision={`${tag}:${items.map(item => item.id).join(',')}`} data={data} onSummaryView={onSummaryView}
        formatValue={format} onPick={onPick ? picked => onPick(Math.floor(picked)) : undefined}
        tooltip={at => {
          const drawn = summaries();
          const bins = items.map((_, index) => binAt(drawn?.[index], at));
          return { title: `${name}, ${binsLabel(at, bins)}`, rows: items.map((item, index) => {
            const bucket = bins[index];
            return { label: item.label, swatch: colours.get(item.id), value: bucket ? format(bucket.lastValue) : 'Not recorded',
              ...(bucket ? {} : { tone: 'muted' as const }) };
          }) };
        }} />
    </div>
    <div className="parts-race-details">
      {!following && <p className="drill-day">{dayLabel(hour)}</p>}
      <DrillParts parts={atHour} live={live} format={format} active={active} positiveBars
        total={following ? undefined : sum} totalRow={{ label: totalLabel, value: sum === null ? 'Not known' : format(sum) }} />
    </div>
  </div>;
}
