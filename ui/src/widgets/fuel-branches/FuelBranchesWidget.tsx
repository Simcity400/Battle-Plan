import { SPEC, UNRECORDED_MESSAGE } from '../../dashboard/regionTime.ts';
import { countryFromKey, isSlotKey } from '../../dashboard/countryKey.ts';
import { keyNameAt, useSlotTags } from '../../dashboard/countrySlots.ts';
import { useMemo, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import { dayLabel, drawEndLabels } from '../../charts/draw';
import { drawLineSummary, summaryExtent } from '../../charts/chartSummary';
import { jumpItem } from '../../charts/jump';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import { fitTooltip } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { runAt } from '../../charts/series';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import Legend from '../../charts/Legend';
import { motionOf } from '../../charts/easing.ts';
import CountryComparisons from '../../dashboard/CountryComparisons';
import type { ComparisonProps } from '../../dashboard/CountryComparisons';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useGate } from '../../dashboard/regionGate';
import { keyId } from '../../dashboard/keyedSeriesData';
import { useKeyedSeries, useSeriesParts } from '../../dashboard/keyedSeries';
import { usePlaybackHour } from '../../dashboard/follow/useFollow';
import { OPTIONS } from './definition';
import { FUEL_BRANCHES, FUEL_CONSUMERS, branchName, percentText, ratioAxis, validateFuelBranches } from './fuelBranchesData';
import './fuelBranches.css';
import type { FuelBranchesReply } from '../../generated/dashboard';
import { productRequest } from '../../dashboard/request.ts';

const NO_WARNINGS: readonly string[] = [];
const UNNAMED = 'Country name unavailable';
const SHARE = { kind: FUEL_CONSUMERS, measure: 'receivedShare' } as const;
const SHARE_PLACES = 4;

export default function FuelBranchesWidget(props: WidgetProps) {
  return <CountryComparisons {...props} component={FuelBranchesCountry} />;
}

const isMember = (within: ComparisonProps['within'], hour: number | null) =>
  !within || hour === null || within.some(span => span.firstHour <= hour && hour <= span.lastHour);

function FuelBranchesCountry(props: ComparisonProps) {
  const { widget, runtime } = props;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = selectedCountryKeys(widget.countries, track)[0] ?? null;
  if (!track) return <WidgetState kind="empty" icon="droplet" message="Load a campaign to see fuel." />;
  if (!tag) return <WidgetState kind="empty" message="Choose a country in widget settings." />;
  return <FuelBranchesOverTime {...props} tag={tag} />;
}

function FuelBranchesOverTime({ widget, runtime, active, onDiagnostics, within, tag }: ComparisonProps & { tag: string }) {
  const options = resolveOptions(OPTIONS, widget.options);
  const viewport = useViewportHeight();
  const menu = useContextMenu('Fuel by branch actions');
  const tokens = useChartTokens();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0])!;
  const slots = useSlotTags(runtime.client, track, active && isSlotKey(tag));
  const following = widget.time.mode === 'follow';
  const [hoverHour, setHoverHour] = useState<number | null>(null);
  const [rootElement, setRoot] = useState<HTMLDivElement | null>(null);
  const root = useMemo(() => ({ current: rootElement }), [rootElement]);
  const playbackHour = usePlaybackHour(runtime.clock, active && following);
  const window = resolveWindow(widget, track, runtime.tracks, playbackHour);
  const playhead = window && (following || widget.time.mode === 'pin') ? window.lastHour : null;
  const extent: [number, number] = widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour];
  const axisOffset = track.axisOffset ?? 0;
  const liveHead = useMemo(() => following ? { read: () => runtime.clock.getSnapshot().position + axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined,
    [following, runtime.clock, axisOffset]);
  const gate = useGate(runtime, widget, SPEC, active);
  const span = gate.span(extent[0], extent[1]);
  const parts = useSeriesParts(runtime.client, gate, track.source, track.branch.id, SHARE, useMemo(() => ({ key: tag, spans: within ?? null }), [tag, within]), span, active);
  const namesAt = gate.recorded(extent[1]);
  const names = useProduct<FuelBranchesReply>(runtime.client, namesAt === null ? null : productRequest({ sourceId: track.source.id, revision: track.source.revision, product: FUEL_BRANCHES,
    params: { branchId: track.branch.id, country: countryFromKey(tag), hour: namesAt } }), active, validateFuelBranches, track.source);
  const consumerNames = names.data?.consumerNames ?? [];
  const keys = useMemo(() => (parts.data?.parts ?? []).map(part => ({ tag, part: part.part })).sort((a, b) => Number(a.part) - Number(b.part)), [parts.data, tag]);
  const membership = useMemo(() => within ? new Map(keys.map(key => [keyId(key), within])) : undefined, [keys, within]);
  const series = useKeyedSeries(runtime.client, runtime.clock, gate, keys.length ? { source: track.source, branch: track.branch, family: SHARE, keys,
    partBits: parts.data?.partBits ?? 32, within: membership, lastHour: track.branch.lastHour } : null,
  following ? null : gate.hour, gate.recorded(hoverHour, within), following, active, root);
  useDiagnostics(onDiagnostics, names.data?.warnings ?? NO_WARNINGS);
  const lines = useMemo(() => keys.map((key, index) => ({ key, color: tokens.categories[index % tokens.categories.length],
    name: branchName(consumerNames, Number(key.part)) })), [keys, tokens.categories, consumerNames]);
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);
  const most = useMemo(() => {
    let top = 0;
    for (const line of lines) {
      const shown = series.summaryByKey.get(keyId(line.key))?.summary;
      const range = shown ? summaryExtent(shown, extent, until) : null;
      if (range && range[1] > top) top = range[1];
    }
    return top;
  }, [lines, series.summaryByKey, extent, until]);
  const axis = useMemo(() => ratioAxis(most), [most]);
  const ends = options.endLabels !== false;
  const reachedByKey = useMemo(() => new Map(series.lines.map(line => [keyId(line.key), line])), [series.lines]);
  const layers = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, chartTokens, state) => {
      const labels = lines.flatMap(line => {
        const shown = series.summaryByKey.get(keyId(line.key))?.summary;
        const end = shown ? drawLineSummary(ctx, frame, chartTokens, shown, line.color, true, state.playhead) : null;
        const run = state.playhead === null ? null : runAt(reachedByKey.get(keyId(line.key))?.runs ?? [], Math.floor(state.playhead));
        return ends && end && run ? [{ text: percentText(Number(run.valueRaw) / 10 ** SHARE_PLACES), y: end.y, from: end.y, color: line.color }] : [];
      });
      drawEndLabels(ctx, frame, chartTokens, labels);
    },
  }], [lines, series.summaryByKey, reachedByKey, ends]);

  const shownTag = keyNameAt(slots, tag, playhead ?? extent[1]);
  if (parts.error) return <WidgetState kind="error" failure={parts.error} onRetry={parts.retry} />;
  if (!parts.data) return <WidgetState kind="loading" label="Loading fuel by branch…" />;
  if (!keys.length) return parts.data.missing.length
    ? <WidgetState kind="unavailable" message={`${shownTag ?? UNNAMED} has no recorded fuel consumers here: ${parts.data.missing[0].reason}`} />
    : <WidgetState kind="empty" icon="droplet" message={`${shownTag ?? UNNAMED} requested no fuel in this time.`} />;
  if (series.error && !series.summaryByKey.size) return <WidgetState kind="error" failure={series.error} onRetry={series.retry} />;
  const tooltip = (at: number) => {
    if (!isMember(within, at)) return fitTooltip([{ title: dayLabel(at), rows: [{ label: 'Not in the group at this date', tone: 'muted' as const }] }], viewport);
    return fitTooltip([{ title: dayLabel(at), rows: lines.map(line => {
      const reached = reachedByKey.get(keyId(line.key));
      const run = runAt(reached?.runs ?? [], at);
      const missing = reached?.missing.get(at);
      const unrecorded = reached?.unrecorded.has(at) === true;
      return { label: line.name, strong: true, swatch: line.color,
        value: run ? percentText(Number(run.valueRaw) / 10 ** SHARE_PLACES) : missing ? (unrecorded ? 'Not recorded' : 'No share') : gate.recorded(at, within) === null ? UNRECORDED_MESSAGE : 'Loading…',
        detail: run ? undefined : missing, tone: run ? undefined : 'muted' as const };
    }) }], viewport);
  };
  return <div className="fuel-branches-time" ref={setRoot}>
    <Legend label="Branches" items={lines.map(line => ({ id: line.key.part, label: line.name, color: line.color }))} />
    <div className="fuel-branches-chart">
      <TimeChart label={`Fuel received by branch of ${shownTag ?? UNNAMED}`} extent={extent} playhead={playhead} liveHead={liveHead}
        playback={playback} afterPlayhead={options.afterPlayhead === true} windowHours={Number(options.windowDays ?? 365) * 24}
        hoursPerSecond={motionOf(runtime.clock.getSnapshot())} branchStart={track.branch.firstHour}
        axes={{ xFrom: null, xTo: null, yMin: 0, yMax: axis.top, log: false }} valueTicks={axis.ticks}
        formatValue={value => `${value}%`} padding={ends ? { right: 64 } : undefined}
        layers={layers} active={active} pickOwner={series.owner} notRecorded={series.unrecorded}
        onSummaryView={series.onSummaryView} onNavigateHour={series.onNavigateHour}
        keyboardHour={series.keyboardHour} keyboardRevision={series.keyboardRevision} onHover={setHoverHour}
        revision={`${track.branch.id}:${tag}:${lines.length}:${ends}:${series.states ? 'states' : 'loading'}`}
        tooltip={at => tooltip(Math.floor(at))}
        onContextMenu={(at, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, at)])} />
    </div>
    {menu.menu}
  </div>;
}
