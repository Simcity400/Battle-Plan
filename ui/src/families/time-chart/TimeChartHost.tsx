import { useCallback, useContext, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent } from 'react';
import { badgesAt, visibleBadges, type EventBadge } from '../../charts/badges.ts';
import { dayLabel, drawCrosshair, drawSelection } from '../../charts/draw.ts';
import { axisFraction, hitZone, plotFrame, pointerHour, wheelSteps, type Padding, type PlotFrame } from '../../charts/frame.ts';
import { actionHour, hoverAt, NO_HOVER, pointerMenu, pointerTarget, reduceHover, type HoverState } from '../../charts/hover.ts';
import { intervalIndexOf } from '../../charts/intervalIndex.ts';
import { jumpItem } from '../../charts/jump.ts';
import Legend, { type LegendItem } from '../../charts/Legend';
import { clusterHour, eventCount, markClusterAt, markSummaryMatches, shownClusters, type MarkCluster, type MarkSummary } from '../../charts/markSummary.ts';
import {
  TIME_CHART_DEFAULT_PADDING,
  TIME_CHART_MIN_SPAN,
  resolveTimeChartFrame,
  resolveTimeChartTargetX,
  type ChartAxisFormatterId,
  type ChartFitId,
  type ChartLayer,
} from '../../charts/renderTimeChart.ts';
import { runAt } from '../../charts/series.ts';
import { drawingSurface, sizeCanvas, toLocal } from '../../charts/surface.ts';
import { canvasMeasureText } from '../../charts/textWidth.ts';
import { CHART_LAYOUT, type ChartTokens } from '../../charts/tokens.ts';
import { hideTooltip, showTooltip, tooltipStore, tooltipText, type TooltipContent } from '../../charts/tooltip.ts';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens.ts';
import { EMPTY_VIEW, reduceView, shownUntil, type AxisSettings, type PlaybackMode, type Range, type ViewAction, type ViewState } from '../../charts/view.ts';
import { ZoomReset } from '../../charts/ZoomReset';
import { useContextMenu } from '../../components/ContextMenu';
import { CountryFlagUrl } from '../../components/CountryFlag';
import WidgetState from '../../components/WidgetState';
import { useIdentityHour } from '../../dashboard/atHead.ts';
import { useChangedState } from '../../dashboard/changedState.ts';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import { useContentRevision } from '../../dashboard/contentRevision.ts';
import { MAX_NAME_HOURS } from '../../dashboard/countryFacts.ts';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt.ts';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import { countryLineColours, uncolouredWarning } from '../../dashboard/countryLineColours.ts';
import { useCountryLooks } from '../../dashboard/countryLooks.ts';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt.ts';
import { defaultCountryKey } from '../../dashboard/countrySelection.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import type { FailureOrigin } from '../../dashboard/follow/page.ts';
import { usePlaybackHour } from '../../dashboard/follow/useFollow.ts';
import { isMember, membershipChanges, resolveCountries } from '../../dashboard/groups.ts';
import { withinParams } from '../../dashboard/keyedSeriesData.ts';
import { resolveWindow } from '../../dashboard/model';
import { nameReadMetadata } from '../../dashboard/nameReads.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { powerDecimals } from '../../dashboard/powerDecimals.ts';
import { PreparingPage } from '../../dashboard/preparing.ts';
import { loadProduct, loadTransientProduct, useFollowedProduct } from '../../dashboard/products.ts';
import { asFailure, type Failure } from '../../dashboard/queryError.ts';
import { useGate } from '../../dashboard/regionGate.ts';
import { axisAt, SPEC } from '../../dashboard/regionTime.ts';
import { forwardRequest } from '../../dashboard/request.ts';
import { AHEAD_BINS, cellBins, viewBinHours } from '../../dashboard/tiles/view.ts';
import { useCountryGroups } from '../../dashboard/useCountryGroups.ts';
import { useCountryLineStyles } from '../../dashboard/useCountryLineStyles.ts';
import { useViewRead } from '../../dashboard/viewRead.ts';
import { useWidgetSkeleton } from '../../dashboard/widgetFrame.tsx';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { holdElement } from '../../draw/bitmapSurface.ts';
import type { HeadLine, HourSpan, SeriesHeadReply } from '../../generated/dashboard';
import { perfLog } from '../../shell/perf.ts';
import BalanceChanges from '../../widgets/country-measure/BalanceChanges';
import { changesRange } from '../../widgets/country-measure/measureData.ts';
import { reachedLines, validateMeasureStates, type MeasureStates } from '../../widgets/country-measure/measureSeriesData.ts';
import '../../widgets/country-measure/measure.css';
import '../../charts/charts.css';
import type { FamilySourceSubscription } from '../contract.ts';
import {
  attachAuditWatch,
  mountFamilyCanvas,
  renderFamilyBodyState,
  reportFamilyShownHour,
  useFamilyAnnouncer,
  useFamilyAudit,
  useFamilyHold,
  useFamilyPauseFlush,
  useFamilyPlacement,
  useFamilyTooltip,
  type MountedFamilyCanvas,
} from '../host/index.ts';
import { freezeGate, type FrozenGate } from '../sources.ts';
import type { TimeChartDeclaration } from './declaration.ts';
import {
  buildAuditRows,
  seriesLayerFor,
  type TimeChartBinding,
  type TimeChartClosedTransfer,
  type TimeChartFrameField,
  type TimeChartLegendReading,
  type TimeChartSeriesMeta,
  type TimeChartViewportField,
} from './laneView.ts';
import {
  ClosedBinRing,
  RetainedClosedWindowStore,
  composeMarkSummary,
  seriesKey,
  type RetainedClosedSeries,
  type SeriesKey,
} from './tiles.ts';
import {
  closedTilesCache,
  fetchExactClosedTiles,
  initialTileBounds,
  unrecordedFromTransfers,
  type ClosedTilesRequest,
} from './tilesClient.ts';
import { buildMarkTooltip, buildMeasureTooltip, formatLegendReading } from './tooltipFormat.ts';
import './timeChartFamily.css';

const TIME_CHART_AUDIT = -1;
const NO_WARNINGS: string[] = [];
const NO_FLAGS: readonly string[] = [];
const NO_NAMES: readonly { readonly tag: string; readonly hour: number }[] = [];
const HOVER_REST_MS = 80;
const BOX_THRESHOLD = 8;

type Point = { x: number; y: number; clientX: number; clientY: number };
type Pointed = { hour: number; describes: number; badges: readonly EventBadge[]; cluster?: MarkCluster };
type Drag =
  | { mode: 'axis-x'; grab: number }
  | { mode: 'axis-y'; grab: number }
  | { mode: 'pan'; lastX: number }
  | { mode: 'box'; x0: number; y0: number; x1: number; y1: number };

interface TimeChartAuditSnapshot {
  readonly client: DashboardClient;
  readonly source: Pick<DashboardSource, 'id' | 'revision'>;
  readonly branchId: string;
  readonly lastHour: number;
  readonly declaration: TimeChartDeclaration;
  readonly tokens: ChartTokens;
  readonly layoutWidth: number;
  readonly layoutHeight: number;
  readonly extent: Range;
  readonly branchStart: number;
  readonly playback: PlaybackMode;
  readonly afterPlayhead: boolean;
  readonly windowHours: number;
  readonly paused: boolean;
  readonly playheadMode: 'follow' | 'pin' | 'range' | 'campaign';
  readonly staticPlayhead: number | null;
  readonly view: ViewState;
  readonly axes: AxisSettings;
  readonly padding?: Partial<Padding>;
  readonly formatValue?: ChartAxisFormatterId;
  readonly fitValues?: ChartFitId;
  readonly areaFill: boolean;
  readonly spendTicks: boolean;
  readonly held: boolean;
  readonly binHours: number;
  readonly series: readonly TimeChartSeriesMeta[];
  readonly tags: readonly string[];
  readonly within?: ReadonlyMap<string, readonly HourSpan[]>;
  readonly badges?: readonly EventBadge[];
  readonly gate: FrozenGate;
}

function validateSeriesHead(value: unknown): SeriesHeadReply {
  if (!value || typeof value !== 'object') throw new Error('The chart head reply is malformed.');
  const obj = value as Partial<SeriesHeadReply>;
  if (
    typeof obj.branchId !== 'string'
    || typeof obj.measure !== 'string'
    || typeof obj.decimals !== 'number'
    || typeof obj.hour !== 'number'
    || typeof obj.binHours !== 'number'
    || typeof obj.binFirstHour !== 'number'
    || typeof obj.binLastHour !== 'number'
    || !Array.isArray(obj.lines)
  ) {
    throw new Error('The chart head reply is malformed.');
  }
  return obj as SeriesHeadReply;
}

async function verifyTimeChartAuditSnapshot(hour: number, snap: TimeChartAuditSnapshot) {
  if (!perfLog) return null;
  const recorded = snap.gate.recorded(hour);
  if (recorded === null) return null;
  const within = withinParams(snap.tags, snap.within, snap.gate.span);
  const headReq = forwardRequest(snap.source, 'series-head', {
    branchId: snap.branchId,
    family: { kind: snap.declaration.family, measure: snap.declaration.measure },
    binHours: snap.binHours,
    countries: snap.tags.map(countryFromKey),
    ...within,
    ...(snap.spendTicks ? { withMarks: true } : {}),
    hour: recorded,
  });
  const head = await perfLog.exactOnce(JSON.stringify(headReq), async () =>
    validateSeriesHead((await snap.client.product(headReq, new AbortController().signal)).data),
  );
  const playhead = snap.playheadMode === 'follow'
    ? Math.max(snap.extent[0], Math.min(snap.extent[1], head.hour))
    : snap.staticPlayhead;
  const until = shownUntil(playhead, snap.playback, snap.afterPlayhead);
  const targetX = resolveTimeChartTargetX({
    extent: snap.extent,
    playhead,
    playback: snap.playback,
    windowHours: snap.windowHours,
    paused: snap.paused,
    view: snap.view,
    axes: snap.axes,
    branchStart: snap.branchStart,
  });
  const bounds = initialTileBounds(Math.floor(targetX[0]), Math.ceil(Math.min(targetX[1], until ?? targetX[1])), snap.lastHour, snap.binHours, AHEAD_BINS);
  const tileReq: ClosedTilesRequest = {
    source: snap.source,
    branchId: snap.branchId,
    kind: snap.declaration.family,
    measure: snap.declaration.measure,
    role: 'primary',
    tags: snap.tags,
    binHours: snap.binHours,
    firstTile: bounds.firstTile,
    lastTile: bounds.lastTile,
    lastHour: snap.lastHour,
    withMarks: snap.spendTicks,
    within: snap.within,
    span: snap.gate.span,
  };
  const transfers = await fetchExactClosedTiles(snap.client, tileReq, new AbortController().signal);
  const store = new RetainedClosedWindowStore();
  const retainedByKey = new Map<SeriesKey, RetainedClosedSeries | null>();
  const transfersByKey = new Map(transfers.map(t => [t.key, t]));
  const rings = new Map<SeriesKey, ClosedBinRing>();
  for (const meta of snap.series) {
    const transfer = transfersByKey.get(meta.key);
    retainedByKey.set(
      meta.key,
      transfer
        ? store.retain(meta.key, transfer.tiles, transfer.firstIndex, transfer.lastIndex, transfer.binHours, snap.held, rings.get(meta.key))
        : null,
    );
  }
  const headByKey = new Map<SeriesKey, HeadLine>();
  for (const line of head.lines) {
    headByKey.set(seriesKey(countryKey(line.country), 'primary'), line);
  }
  const single = snap.series.length === 1;
  const layers: ChartLayer[] = snap.series.map(meta =>
    seriesLayerFor(
      retainedByKey.get(meta.key) ?? null,
      head,
      headByKey.get(meta.key) ?? null,
      meta.color,
      snap.held,
      meta.dash,
      single && snap.areaFill && meta.role === 'primary',
      until,
    ),
  );
  const scratch = typeof OffscreenCanvas !== 'undefined'
    ? new OffscreenCanvas(1, 1).getContext('2d')!
    : document.createElement('canvas').getContext('2d')!;
  const resolved = resolveTimeChartFrame({
    layoutWidth: snap.layoutWidth,
    layoutHeight: snap.layoutHeight,
    tokens: snap.tokens,
    layers,
    extent: snap.extent,
    playhead,
    playback: snap.playback,
    windowHours: snap.windowHours,
    paused: snap.paused,
    view: snap.view,
    axes: snap.axes,
    branchStart: snap.branchStart,
    fitValues: snap.fitValues,
    formatValue: snap.formatValue,
    hasBadges: !!snap.badges?.length,
    padding: snap.padding,
    measure: canvasMeasureText(scratch),
  });
  const frame = resolved.frame;
  const viewFrom = Math.floor(frame.x[0]);
  const viewEnd = Math.ceil(frame.x[1]);
  const plotWidth = Math.max(1, Math.ceil(frame.plotWidth));
  const cell = cellBins(Math.max(1, viewEnd - viewFrom + 1), plotWidth, snap.binHours);
  const markSummaries: MarkSummary[] = [];
  if (snap.spendTicks) {
    for (const meta of snap.series) {
      if (meta.role !== 'primary') continue;
      const summary = composeMarkSummary(
        retainedByKey.get(meta.key) ?? null,
        head,
        headByKey.get(meta.key) ?? null,
        viewFrom,
        viewEnd,
        plotWidth,
        cell,
        until,
        single ? snap.tokens.muted : meta.color,
        meta.country,
      );
      if (summary && summary.clusters.length > 0) markSummaries.push(summary);
    }
  }
  const rows = buildAuditRows(frame, snap.binHours, until, snap.series, retainedByKey, head, headByKey, markSummaries);
  return { values: new Map(rows.map(row => [row.key, row.value ?? ''])), ordered: true };
}

function reconstructPlotFrame(field: TimeChartFrameField, width: number, height: number): PlotFrame {
  const padding: Padding = {
    left: field.plotLeft,
    top: field.plotTop,
    right: Math.max(0, width - field.plotLeft - field.plotWidth),
    bottom: Math.max(0, height - field.plotTop - field.plotHeight),
  };
  return plotFrame(width, height, padding, field.x, field.y, field.log, 0, false);
}

interface HostMetaState {
  readonly mode: string;
  readonly reason: string | null;
  readonly failureOrigin: FailureOrigin;
}

export interface TimeChartHostProps extends WidgetProps {
  readonly declaration: TimeChartDeclaration;
}

export default function TimeChartHost({ declaration, widget, runtime, active, onDiagnostics }: TimeChartHostProps) {
  useWidgetSkeleton('chart');
  const prepared = useContext(PreparingPage);
  const owner = useId();
  const tokens = useChartTokens();
  const viewportHeight = useViewportHeight();
  const options = useMemo(() => resolveOptions(declaration.options, widget.options), [declaration.options, widget.options]);
  const digits = powerDecimals(options);
  const spendTicks = declaration.spendTicks === true && options.spendTicks === true;
  const areaFill = options.areaFill === true;
  const endLabels = options.endLabels === true;
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const afterPlayhead = options.afterPlayhead === true;
  const windowHours = Number(options.windowDays ?? 365) * 24;

  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const lastHour = track?.branch.lastHour ?? 0;
  const fallback = defaultCountryKey(track);
  const groups = useCountryGroups(runtime, track, active && widget.countries.kind === 'group');
  const resolved = useMemo(
    () => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, lastHour, Number.MAX_SAFE_INTEGER),
    [widget.countries, fallback, groups.data, lastHour],
  );
  const membership = resolved.membership ?? undefined;

  const following = widget.time.mode === 'follow';
  const pinned = widget.time.mode === 'pin';
  const gate = useGate(runtime, widget, SPEC, active);
  const playbackHour = usePlaybackHour(runtime.clock, false);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const playhead = window && (following || pinned) ? window.lastHour : null;
  const staticPlayhead = following ? null : playhead;
  const extentFirst = track ? (widget.time.mode === 'range' && window ? window.firstHour : track.branch.firstHour) : 0;
  const extentLast = track ? (widget.time.mode === 'range' && window ? window.lastHour : track.branch.lastHour) : 1;
  const extent = useMemo<Range>(() => [extentFirst, extentLast], [extentFirst, extentLast]);
  const head = playhead ?? extent[1];
  const staticHeadHour = !following && track
    ? (pinned ? gate.hourAt(axisAt(playbackHour)) : gate.recorded(widget.time.mode === 'range' && window ? window.lastHour : track.branch.lastHour))
    : null;
  const until = shownUntil(playhead, playback, afterPlayhead);
  const axes = useMemo<AxisSettings>(
    () => ({
      xFrom: null,
      xTo: null,
      yMin: (options.yMin as number | null) ?? null,
      yMax: (options.yMax as number | null) ?? null,
      log: options.log === true,
    }),
    [options.yMin, options.yMax, options.log],
  );
  const padding = useMemo<Partial<Padding>>(
    () => ({ right: endLabels ? CHART_LAYOUT.measureEndLabels : CHART_LAYOUT.right }),
    [endLabels],
  );

  const [windowPaused, setWindowPaused] = useState(() => !runtime.clock.getSnapshot().playing);
  useEffect(() => {
    if (playback !== 'window') return;
    setWindowPaused(!runtime.clock.getSnapshot().playing);
    return runtime.clock.subscribe(() => {
      const next = !runtime.clock.getSnapshot().playing;
      setWindowPaused(prev => (prev === next ? prev : next));
    });
  }, [runtime.clock, playback]);
  const paused = playback === 'window' ? windowPaused : false;

  const frameRef = useRef<HTMLDivElement>(null);
  const chartHostRef = useRef<HTMLDivElement>(null);
  const surfaceHostRef = useRef<HTMLDivElement>(null);
  const overlayRef = useRef<HTMLCanvasElement>(null);
  const eventsTextRef = useRef<HTMLParagraphElement>(null);

  const [chartSize, setChartSize] = useState<{ width: number; height: number }>({ width: 0, height: 0 });
  const chartSizeRef = useRef(chartSize);
  chartSizeRef.current = chartSize;

  const [view, setView] = useState<ViewState>(EMPTY_VIEW);
  const viewRef = useRef<ViewState>(view);
  viewRef.current = view;
  const zoomed = view.x !== null || view.y !== null;

  const extentKey = `${track?.source.id ?? ''}:${track?.branch.id ?? ''}:${extent[0]}:${extent[1]}`;
  const prevExtentKeyRef = useRef(extentKey);
  useEffect(() => {
    if (prevExtentKeyRef.current !== extentKey) {
      prevExtentKeyRef.current = extentKey;
      setView(EMPTY_VIEW);
    }
  }, [extentKey]);

  const initialBinHours = useMemo(() => {
    const plotW = Math.max(1, Math.round((chartSize.width || 480) - TIME_CHART_DEFAULT_PADDING.left - (padding.right ?? CHART_LAYOUT.right)));
    const targetX = resolveTimeChartTargetX({
      extent,
      playhead,
      playback,
      windowHours,
      paused,
      view,
      axes,
      branchStart: track?.branch.firstHour ?? 0,
    });
    const span = Math.max(1, Math.ceil(targetX[1]) - Math.floor(targetX[0]) + 1);
    return viewBinHours(span, plotW);
  }, [chartSize.width, padding.right, extent, playhead, playback, windowHours, paused, view, axes, track?.branch.firstHour]);

  const [viewportBinHours, setViewportBinHours] = useState<number | null>(null);
  const activeBinHours = viewportBinHours ?? initialBinHours;
  const activeBinHoursRef = useRef(activeBinHours);
  activeBinHoursRef.current = activeBinHours;

  const [hoverHour, setHoverHour] = useState<number | null>(null);
  const [visibleFlags, setVisibleFlags] = useChangedState<readonly string[]>(NO_FLAGS);
  const onVisibleItems = useCallback((items: readonly LegendItem[]) => {
    const tags = [...new Set(items.map(item => item.id))].sort();
    setVisibleFlags(before => (before.length === tags.length && before.every((tag, index) => tag === tags[index]) ? before : tags));
  }, [setVisibleFlags]);

  const identityHour = useIdentityHour(
    runtime.client,
    track,
    resolved.tags,
    runtime.clock,
    active && following,
    useCallback(h => (track ? resolveWindow(widget, track, runtime.tracks, h)?.lastHour ?? null : null), [widget, track, runtime.tracks]),
  );
  const styleAt = following && identityHour !== null
    ? identityHour
    : playhead ?? (widget.time.mode === 'range' && window ? window.lastHour : lastHour);
  const shownAt = Math.floor(styleAt);

  const styles = useCountryLineStyles(runtime.client, gate, track, resolved.tags, styleAt, active);
  const known = styles.known;

  useEffect(() => {
    const uncoloured = uncolouredWarning(resolved.tags, known, !!styles.data);
    const warnings = [
      ...(styles.data?.warnings ?? []),
      ...(styles.error ? [`Map colours could not be loaded: ${styles.error.message}`] : []),
      ...(groups.data?.warnings ?? []),
      ...(widget.countries.kind === 'group' && groups.error ? [`Country membership labels could not be loaded: ${groups.error.message}`] : []),
      ...(uncoloured ? [uncoloured] : []),
    ];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [resolved.tags, styles.data, styles.error, known, groups.data, groups.error, widget.countries.kind, onDiagnostics]);

  const seriesItems = useMemo(() => {
    const colours = countryLineColours(resolved.tags, known, tokens.surface, tokens.countryColours === 'fixed');
    return resolved.tags.map(tag => ({
      tag,
      hex: colours.get(tag)!,
      membership: membership?.get(tag),
    }));
  }, [resolved.tags, known, tokens.surface, tokens.countryColours, membership]);

  const wantedLabels = useMemo(() => seriesItems.flatMap(item => [
    { tag: item.tag, hour: shownAt },
    ...(hoverHour === null ? [] : [{ tag: item.tag, hour: hoverHour }]),
    ...(item.membership ? membershipChanges(item.membership, extent[0], extent[1], resolved.unobserved).map(change => ({ tag: item.tag, hour: change.hour })) : []),
  ]), [seriesItems, shownAt, hoverHour, extent, resolved.unobserved]);
  const nameHours = useMemo(() => Math.max(MAX_NAME_HOURS, new Set(wantedLabels.map(item => item.hour)).size), [wantedLabels]);
  const names = useCountryNamesAt(runtime.client, track, wantedLabels, active, nameHours);
  const hoverWanted = useMemo(
    () => (hoverHour === null ? NO_NAMES : seriesItems.map(item => ({ tag: item.tag, hour: hoverHour }))),
    [hoverHour, seriesItems],
  );
  const hoverLabels = useCountryLabelsAt(
    runtime.client,
    track,
    hoverWanted,
    active,
  );
  const labelAt = useCallback((hour: number) => (tag: string) => {
    const name = names.get(nameKey(hour, tag));
    return { ...hoverLabels.labelAt(hour)(tag), ...(name ? nameReadMetadata(name) : {}) };
  }, [names, hoverLabels.labelAt]);

  const flagTags = active ? visibleFlags : NO_FLAGS;
  const legendLooks = useCountryLooks(runtime.client, track, flagTags, styleAt, active, undefined, undefined, true, flagTags);

  const badges = useMemo<EventBadge[]>(() => seriesItems.flatMap(item => (item.membership
    ? membershipChanges(item.membership, extent[0], extent[1], resolved.unobserved).map(change => ({
      hour: change.hour,
      glyph: change.change === 'joined' ? '+' : '−',
      label: `${labelAt(change.hour)(item.tag).name} ${change.change}`,
      color: item.hex,
    }))
    : [])), [seriesItems, extent, resolved.unobserved, labelAt]);

  const seriesMeta = useMemo<readonly TimeChartSeriesMeta[]>(() => seriesItems.map(item => ({
    key: seriesKey(item.tag, 'primary'),
    country: item.tag,
    role: 'primary',
    label: labelAt(shownAt)(item.tag).name,
    color: item.hex,
  })), [seriesItems, labelAt, shownAt]);

  const withinObj = useMemo(
    () => (track ? withinParams(resolved.tags, membership, gate.span) : {}),
    [track, resolved.tags, membership, gate.span],
  );

  const canQuery = active && !prepared && !!track && resolved.tags.length > 0
    && !resolved.reason
    && (widget.countries.kind !== 'group' || !!groups.data)
    && !((pinned || widget.time.mode === 'range') && !window);

  const followPlan = useMemo(() => {
    if (!following || !canQuery || !track || chartSize.width <= 0) {
      return { subs: [] as FamilySourceSubscription[], refused: null as string | null };
    }
    const rawQuery: Record<string, unknown> = {
      panel: 'series-head',
      branchId: track.branch.id,
      family: { kind: declaration.family, measure: declaration.measure },
      binHours: activeBinHours,
      countries: resolved.tags.map(countryFromKey),
      ...withinObj,
      ...(spendTicks ? { withMarks: true } : {}),
    };
    const checked = gate.checkFollow(rawQuery);
    if (checked.refused) return { subs: [], refused: checked.refused };
    if (!checked.query) return { subs: [], refused: null };
    return {
      subs: [{
        name: 'head',
        source: track.source,
        branch: track.branch.id,
        offset: track.axisOffset,
        query: checked.query,
        channel: 'panel' as const,
        product: 'series-head',
      }],
      refused: null,
    };
  }, [following, canQuery, track, chartSize.width, declaration.family, declaration.measure, activeBinHours, resolved.tags, withinObj, spendTicks, gate.checkFollow]);

  const staticHeadReq = useMemo(() => {
    if (following || !canQuery || !track || staticHeadHour === null) return null;
    return forwardRequest(track.source, 'series-head', {
      branchId: track.branch.id,
      family: { kind: declaration.family, measure: declaration.measure },
      binHours: activeBinHours,
      countries: resolved.tags.map(countryFromKey),
      ...withinObj,
      ...(spendTicks ? { withMarks: true } : {}),
      hour: staticHeadHour,
    });
  }, [following, canQuery, track, staticHeadHour, declaration.family, declaration.measure, activeBinHours, resolved.tags, withinObj, spendTicks]);

  const staticHeadProduct = useFollowedProduct<SeriesHeadReply>(
    runtime.client,
    staticHeadReq,
    ['hour'],
    active && !following && staticHeadReq !== null,
    validateSeriesHead,
    track?.source,
  );
  const staticHeld = !following && staticHeadReq !== null && staticHeadProduct.data !== undefined && staticHeadProduct.asking !== undefined;
  const staticHead = !following ? (staticHeadProduct.data ?? null) : null;

  const contentRev = useContentRevision(runtime.client);
  const stateOwnerKey = useMemo(
    () => JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, declaration.family, declaration.measure, resolved.tags, membership && [...membership]]),
    [track?.source.id, track?.source.revision, track?.branch.id, declaration.family, declaration.measure, resolved.tags, membership],
  );
  const gatedHover = gate.recorded(hoverHour);
  const hoverReads = useMemo(() => {
    if (!track || !resolved.tags.length || gatedHover === null) return [];
    return resolved.tags
      .filter(tag => isMember(membership?.get(tag), gatedHover))
      .map(tag => ({ country: countryFromKey(tag), hour: gatedHover }));
  }, [track, resolved.tags, gatedHover, membership]);
  const stateRead = useViewRead<typeof hoverReads, MeasureStates>(
    track && hoverReads.length ? JSON.stringify([stateOwnerKey, contentRev]) : null,
    hoverReads,
    JSON.stringify(hoverReads),
    active,
    (reads, signal, publish, restart) => loadProduct(
      runtime.client,
      forwardRequest(track!.source, 'series-state', {
        branchId: track!.branch.id,
        family: { kind: declaration.family, measure: declaration.measure },
        reads,
      }),
      signal,
      validateMeasureStates,
      track!.source,
      restart,
      publish,
    ),
    runtime.client,
  );
  const reachedByTag = useMemo(
    () => new Map(reachedLines(resolved.tags, stateRead.data).map(line => [line.tag, line])),
    [resolved.tags, stateRead.data],
  );
  const hourLoaded = useCallback(
    (tag: string, h: number) => stateRead.data?.states.some(state => countryKey(state.country) === tag && state.hour === h) ?? false,
    [stateRead.data],
  );

  const [decimals, setDecimals] = useState<number | null>(null);
  const effectiveDecimals = stateRead.data?.decimals ?? staticHead?.decimals ?? decimals;

  const [legendReadings, setLegendReadings] = useState<Readonly<Record<string, TimeChartLegendReading>> | null>(null);
  const [tileError, setTileError] = useState<Failure | null>(null);
  const [notRecorded, setNotRecorded] = useState<string | null>(null);
  const [navError, setNavError] = useState<Failure | null>(null);
  const [meta, setMeta] = useState<HostMetaState>({ mode: 'reading', reason: null, failureOrigin: 'local' });
  const metaSigRef = useRef('');
  const hasPictureRef = useRef(false);
  const tileBindGenRef = useRef<number | null>(null);
  const lastBindSigRef = useRef('');

  const blockedState = (() => {
    if (!track) return <WidgetState kind="empty" icon={declaration.icon} message={`Load a campaign to see ${declaration.noun}.`} />;
    if (followPlan.refused) return <WidgetState kind="unavailable" technical={followPlan.refused} />;
    if ((pinned || widget.time.mode === 'range') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
    if (widget.countries.kind === 'group' && groups.error && !groups.data) return <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
    if (resolved.reason) return <WidgetState kind="empty" message={resolved.reason} />;
    if (widget.countries.kind === 'group' && !groups.data) return <WidgetState kind="loading" label={`Loading ${declaration.noun}…`} />;
    if (!following && staticHeadProduct.error && !staticHead) {
      return <WidgetState kind="error" failure={staticHeadProduct.error} onRetry={staticHeadProduct.retry} />;
    }
    if (!following && staticHeadReq !== null && !staticHead) {
      return <WidgetState kind="loading" label={`Loading ${declaration.noun}…`} />;
    }
    return null;
  })();

  const bounds = useMemo<readonly [number, number] | null>(
    () => (track ? [track.branch.firstHour, track.branch.lastHour] : null),
    [track?.branch.firstHour, track?.branch.lastHour],
  );
  const enabled = active && !prepared && !blockedState && (following ? followPlan.subs.length > 0 : staticHead !== null);
  const auditable = !!perfLog && !blockedState && !!track && resolved.tags.length > 0;

  const placement = useFamilyPlacement({
    family: 'time-chart',
    declarationKind: declaration.kind,
    widgetId: widget.id,
    widgetKind: widget.kind,
    subject: declaration.noun,
    widgetTrackIds: widget.trackIds,
    dataContext: `${track?.branch.id ?? ''}|${resolved.tags.join(',')}|${declaration.measure}`,
    runtime,
    enabled,
    auditable,
    subscriptions: followPlan.subs,
    offset: track?.axisOffset ?? 0,
    pin: following ? null : (staticHead?.hour ?? staticHeadHour),
    bounds,
    frameRef,
    surfaceLabel: 'time-chart',
  });
  const { pipeline, pipe, sourceIdsRef, subsReady, tooltipOwner, dataContextKey, retryStream, retryLocal } = placement;

  const tooltips = useFamilyTooltip(tooltipOwner, dataContextKey);
  useFamilyHold(frameRef, staticHeld, tooltips);
  const { announce, hasFrame, liveRegionRef } = useFamilyAnnouncer('time-chart');

  const audit = useFamilyAudit<TimeChartAuditSnapshot>({
    pipeline,
    pipe,
    ownerId: TIME_CHART_AUDIT,
    auditable,
  });

  const frozenGate = useMemo(
    () => freezeGate(gate),
    [gate.open, track, gate.country?.tag, gate.country?.aliases, gate.country?.historyHeld],
  );

  const reportShown = useShownHour();
  const reportShownRef = useRef(reportShown);
  reportShownRef.current = reportShown;

  const surfaceRef = useRef<MountedFamilyCanvas | null>(null);
  const transfersRef = useRef<readonly TimeChartClosedTransfer[]>([]);
  const tilesVersionRef = useRef(0);
  const lastTileSigRef = useRef('');
  const lastViewportRef = useRef<TimeChartViewportField | null>(null);
  const lastFrameRef = useRef<TimeChartFrameField | null>(null);
  const lastLegendObjRef = useRef<unknown>(null);
  const tileFetchAbortRef = useRef<AbortController | null>(null);
  const navAbortRef = useRef<AbortController | null>(null);
  const lastAuditRef = useRef<{ id: number; sig: string } | null>(null);

  const hoverRef = useRef<HoverState<Point>>(NO_HOVER);
  const pointerRef = useRef<Point | null>(null);
  const dragRef = useRef<Drag | null>(null);
  const shownHourRef = useRef<number | null>(null);
  const hoverClusterRef = useRef<MarkCluster | null>(null);
  const pendingHoverRef = useRef<{ hour: number; since: number; timer: ReturnType<typeof setTimeout> } | null>(null);
  const keyIntentRef = useRef<boolean | { since: number }>(false);
  const pendingNavRef = useRef<{ hover: HoverState<Point>; tooltip: number } | null>(null);
  const overlayPaintedRef = useRef(false);
  const [changeList, setChangeList] = useState<{ id: number; tag: string; first: string; end?: string } | null>(null);
  const changeListIdRef = useRef(0);
  const menu = useContextMenu('Chart actions');
  const menuRef = useRef(menu);
  menuRef.current = menu;

  const latestRef = useRef({
    declaration,
    tokens,
    extent,
    branchStart: track?.branch.firstHour ?? 0,
    lastHour,
    playback,
    afterPlayhead,
    windowHours,
    paused,
    playheadMode: widget.time.mode,
    staticPlayhead,
    axes,
    padding,
    areaFill,
    endLabels,
    spendTicks,
    digits,
    effectiveDecimals,
    seriesMeta,
    seriesItems,
    badges,
    staticHead,
    track,
    resolvedTags: resolved.tags,
    membership,
    gateSpan: gate.span,
    gateWithin: gate.within,
    frozenGate,
    captureActive: audit.captureActive,
    reachedByTag,
    hourLoaded,
    labelAt,
    viewportHeight,
    shownAt,
    active,
    runtime,
    following,
    until,
  });
  latestRef.current = {
    declaration,
    tokens,
    extent,
    branchStart: track?.branch.firstHour ?? 0,
    lastHour,
    playback,
    afterPlayhead,
    windowHours,
    paused,
    playheadMode: widget.time.mode,
    staticPlayhead,
    axes,
    padding,
    areaFill,
    endLabels,
    spendTicks,
    digits,
    effectiveDecimals,
    seriesMeta,
    seriesItems,
    badges,
    staticHead,
    track,
    resolvedTags: resolved.tags,
    membership,
    gateSpan: gate.span,
    gateWithin: gate.within,
    frozenGate,
    captureActive: audit.captureActive,
    reachedByTag,
    hourLoaded,
    labelAt,
    viewportHeight,
    shownAt,
    active,
    runtime,
    following,
    until,
  };

  const bind = useCallback((tilesOnly = false) => {
    const mounted = surfaceRef.current;
    if (!mounted || !pipeline || pipe === null) return;
    const { width, height } = chartSizeRef.current;
    if (width <= 0 || height <= 0) return;
    const cur = latestRef.current;
    const headSubId = cur.following ? (sourceIdsRef.current.head ?? null) : null;

    let auditId: number | undefined;
    if (perfLog && cur.captureActive && cur.track && cur.resolvedTags.length > 0) {
      const sig = JSON.stringify([
        cur.track.source.id,
        cur.track.source.revision,
        cur.track.branch.id,
        cur.declaration.kind,
        width,
        height,
        cur.extent,
        cur.playback,
        cur.afterPlayhead,
        cur.windowHours,
        cur.paused,
        cur.playheadMode,
        cur.staticPlayhead,
        viewRef.current,
        cur.axes,
        cur.padding,
        cur.areaFill,
        cur.spendTicks,
        activeBinHoursRef.current,
        cur.seriesMeta,
        cur.badges.length,
      ]);
      if (lastAuditRef.current && lastAuditRef.current.sig === sig) {
        auditId = lastAuditRef.current.id;
      } else {
        auditId = audit.capture({
          client: cur.runtime.client,
          source: cur.track.source,
          branchId: cur.track.branch.id,
          lastHour: cur.lastHour,
          declaration: cur.declaration,
          tokens: cur.tokens,
          layoutWidth: width,
          layoutHeight: height,
          extent: cur.extent,
          branchStart: cur.branchStart,
          playback: cur.playback,
          afterPlayhead: cur.afterPlayhead,
          windowHours: cur.windowHours,
          paused: cur.paused,
          playheadMode: cur.playheadMode,
          staticPlayhead: cur.staticPlayhead,
          view: viewRef.current,
          axes: cur.axes,
          padding: cur.padding,
          formatValue: cur.declaration.formatValue,
          fitValues: cur.declaration.fitValues,
          areaFill: cur.areaFill,
          spendTicks: cur.spendTicks,
          held: cur.declaration.held === true,
          binHours: activeBinHoursRef.current,
          series: cur.seriesMeta,
          tags: cur.resolvedTags,
          within: cur.membership,
          badges: cur.badges.length ? cur.badges : undefined,
          gate: cur.frozenGate,
        }, verifyTimeChartAuditSnapshot);
        lastAuditRef.current = { id: auditId, sig };
      }
    }

    const model: TimeChartBinding = {
      family: 'time-chart',
      kind: cur.declaration.kind,
      tokens: cur.tokens,
      extent: cur.extent,
      branchStart: cur.branchStart,
      lastHour: cur.lastHour,
      playback: cur.playback,
      afterPlayhead: cur.afterPlayhead,
      windowHours: cur.windowHours,
      paused: cur.paused,
      playheadMode: cur.playheadMode,
      staticPlayhead: cur.staticPlayhead,
      view: viewRef.current,
      axes: cur.axes,
      padding: cur.padding,
      formatValue: cur.declaration.formatValue,
      fitValues: cur.declaration.fitValues,
      held: cur.declaration.held === true,
      areaFill: cur.areaFill,
      endLabels: cur.endLabels,
      endLabelFormat: {
        decimals: cur.effectiveDecimals,
        digits: cur.digits,
        unit: cur.declaration.unit,
      },
      spendTicks: cur.spendTicks,
      series: cur.seriesMeta,
      badges: cur.badges.length ? cur.badges : undefined,
      headSubId,
      staticHead: cur.following ? null : cur.staticHead,
      tilesVersion: tilesVersionRef.current,
      closedTransfers: transfersRef.current,
      ...(perfLog && cur.captureActive && auditId !== undefined ? { audit: true, auditId } : {}),
    };

    const dpr = typeof devicePixelRatio === 'number' ? devicePixelRatio : 1;
    const dated = headSubId === null && cur.staticHead
      ? { h: cur.staticHead.hour, c: cur.staticHead.hour }
      : undefined;
    const bindSig = JSON.stringify([
      width,
      height,
      dpr,
      dated ?? null,
      model.kind,
      model.tokens,
      model.extent,
      model.branchStart,
      model.lastHour,
      model.playback,
      model.afterPlayhead,
      model.windowHours,
      model.paused,
      model.playheadMode,
      model.staticPlayhead,
      model.view,
      model.axes,
      model.padding ?? null,
      model.formatValue ?? null,
      model.fitValues ?? null,
      model.held ?? false,
      model.areaFill,
      model.endLabels,
      model.endLabelFormat,
      model.spendTicks,
      model.series,
      model.badges ?? null,
      model.headSubId,
      model.staticHead ? [model.staticHead.branchId, model.staticHead.hour, model.staticHead.binHours, model.staticHead.binFirstHour] : null,
      model.tilesVersion,
      model.audit ?? false,
      model.auditId ?? null,
    ]);
    if (bindSig === lastBindSigRef.current) return;
    lastBindSigRef.current = bindSig;

    mounted.canvas.style.width = `${width}px`;
    mounted.canvas.style.height = `${height}px`;
    pipeline.present(pipe, 'time-chart');
    tileBindGenRef.current = tilesOnly && hasPictureRef.current ? pipeline.binding(pipe) + 1 : null;
    pipeline.bind(
      pipe,
      [mounted.surface],
      {
        surfaces: [{ width, height, dpr }],
        ...(dated ? { dated } : {}),
      },
      {},
      model,
    );
  }, [pipeline, pipe, audit.capture, sourceIdsRef]);
  const bindRef = useRef(bind);
  bindRef.current = bind;

  const ensureTilesForViewport = useCallback((vp: { readonly binHours: number; readonly firstTile: number; readonly lastTile: number }) => {
    const cur = latestRef.current;
    if (!cur.track || !cur.resolvedTags.length || !cur.active) return;
    const req: ClosedTilesRequest = {
      source: cur.track.source,
      branchId: cur.track.branch.id,
      kind: cur.declaration.family,
      measure: cur.declaration.measure,
      role: 'primary',
      tags: cur.resolvedTags,
      binHours: vp.binHours,
      firstTile: vp.firstTile,
      lastTile: vp.lastTile,
      lastHour: cur.lastHour,
      withMarks: cur.spendTicks,
      within: cur.membership,
      span: cur.gateSpan,
    };
    const cache = closedTilesCache(cur.runtime.client);
    const collected = cache.collect(req);
    if (collected.signature !== lastTileSigRef.current) {
      lastTileSigRef.current = collected.signature;
      transfersRef.current = collected.transfers;
      tilesVersionRef.current += 1;
      const firstDecimals = collected.transfers[0]?.tiles[0]?.decimals;
      if (firstDecimals !== undefined) setDecimals(prev => (prev === null ? firstDecimals : prev));
      setNotRecorded(collected.missing.size === 0 ? unrecordedFromTransfers(collected.transfers) : null);
      bindRef.current(true);
    }
    if (collected.missing.size === 0) return;
    tileFetchAbortRef.current?.abort();
    const controller = new AbortController();
    tileFetchAbortRef.current = controller;
    void cache.fetchMissing(cur.runtime.client, req, collected.missing, controller.signal).then(() => {
      if (controller.signal.aborted) return;
      const after = cache.collect(req);
      setTileError(null);
      if (after.signature !== lastTileSigRef.current) {
        lastTileSigRef.current = after.signature;
        transfersRef.current = after.transfers;
        tilesVersionRef.current += 1;
        const firstDecimals = after.transfers[0]?.tiles[0]?.decimals;
        if (firstDecimals !== undefined) setDecimals(prev => (prev === null ? firstDecimals : prev));
        setNotRecorded(after.missing.size === 0 ? unrecordedFromTransfers(after.transfers) : null);
        bindRef.current(true);
      }
    }).catch(error => {
      if (controller.signal.aborted) return;
      setTileError(asFailure(error));
    });
  }, []);
  const ensureTilesRef = useRef(ensureTilesForViewport);
  ensureTilesRef.current = ensureTilesForViewport;

  useEffect(() => () => {
    tileFetchAbortRef.current?.abort();
    navAbortRef.current?.abort();
  }, []);

  const syncOverlayAndTooltip = useCallback(() => {
    const cur = latestRef.current;
    const hostEl = chartHostRef.current;
    const overlayEl = overlayRef.current;
    const frameField = lastFrameRef.current;
    const { width, height } = chartSizeRef.current;
    if (!hostEl || !overlayEl || !frameField || width <= 0 || height <= 0) return;

    const plot = reconstructPlotFrame(frameField, width, height);
    const dpr = typeof devicePixelRatio === 'number' ? devicePixelRatio : 1;
    const sheet = drawingSurface(overlayEl);
    const octx = sheet.context;
    const pw = Math.max(1, Math.round(width * dpr));
    const ph = Math.max(1, Math.round(height * dpr));
    if (sizeCanvas(sheet.canvas, pw, ph)) overlayPaintedRef.current = false;
    if (octx && overlayPaintedRef.current) {
      octx.setTransform(1, 0, 0, 1, 0, 0);
      octx.clearRect(0, 0, pw, ph);
      overlayPaintedRef.current = false;
    }

    const drag = dragRef.current;
    const pointer = pointerRef.current;
    const currentUntil = frameField.until;

    const pointedAt = (point: { x: number; y: number }): Pointed | null => {
      let cluster: MarkCluster | null = null;
      for (const summary of frameField.markSummaries) {
        if (!markSummaryMatches(summary, plot, currentUntil)) continue;
        const next = markClusterAt(point, plot, summary, currentUntil);
        if (next && (!cluster || Math.abs(plot.toX(clusterHour(next)) - point.x) < Math.abs(plot.toX(clusterHour(cluster)) - point.x))) {
          cluster = next;
        }
      }
      const badgeHits = cur.badges.length ? badgesAt(point, plot, cur.badges, currentUntil) : [];
      if (cluster && (!badgeHits.length || Math.abs(plot.toX(clusterHour(cluster)) - point.x) <= Math.abs(plot.toX(badgeHits[0]!.hour) - point.x))) {
        return { hour: clusterHour(cluster), describes: cluster.firstHour, badges: cluster.badge ? [cluster.badge] : [], cluster };
      }
      if (badgeHits.length) return { hour: badgeHits[0]!.hour, describes: badgeHits[0]!.hour, badges: badgeHits };
      const zone = hitZone(point, plot);
      if (zone !== 'plot') return null;
      const target = pointerTarget(zone, pointerHour(plot.hourAt(point.x), plot.x, currentUntil), plot.x, currentUntil, cur.extent);
      return target ? { hour: target.at, describes: target.describes, badges: [] } : null;
    };

    if (drag?.mode === 'box' && octx) {
      octx.setTransform(pw / width, 0, 0, ph / height, 0, 0);
      drawSelection(octx, cur.tokens, drag);
      overlayPaintedRef.current = true;
    }

    const zone = pointer ? hitZone(pointer, plot) : null;
    const hit = hoverAt(hoverRef.current, plot.x, currentUntil, point => (!drag || drag.mode !== 'box' ? pointedAt(point) : null));
    const pointed = hit?.by === 'pointer' ? hit as Pointed : null;
    hoverClusterRef.current = pointed?.cluster ?? null;

    const eventHour = hit?.by === 'keyboard' && hit.hour >= plot.x[0] && hit.hour <= Math.min(plot.x[1], currentUntil ?? plot.x[1])
      ? hit.hour
      : null;
    const keyedBadges = eventHour !== null && cur.badges.length
      ? intervalIndexOf(cur.badges, () => cur.badges.map(badge => ({ firstHour: badge.hour, lastHour: badge.hour, badge }))).at(eventHour).map(entry => entry.badge)
      : [];
    const at = hit ? { hour: hit.hour, describes: pointed?.describes ?? hit.hour, badges: pointed?.badges ?? keyedBadges } : null;
    const nextShownHour = at ? Math.floor(at.describes) : null;
    shownHourRef.current = nextShownHour;

    if (cur.active) {
      const pending = pendingHoverRef.current;
      if (nextShownHour === null) {
        if (pending) { clearTimeout(pending.timer); pendingHoverRef.current = null; }
        setHoverHour(prev => (prev === null ? prev : null));
      } else if (hit?.by === 'pointer') {
        if (!pending || pending.hour !== nextShownHour) {
          if (pending) clearTimeout(pending.timer);
          pendingHoverRef.current = {
            hour: nextShownHour,
            since: performance.now(),
            timer: setTimeout(() => {
              pendingHoverRef.current = null;
              setHoverHour(nextShownHour);
            }, HOVER_REST_MS),
          };
        }
      } else {
        if (pending) { clearTimeout(pending.timer); pendingHoverRef.current = null; }
        setHoverHour(prev => (prev === nextShownHour ? prev : nextShownHour));
      }
    }

    if (at) {
      if (octx) {
        octx.setTransform(pw / width, 0, 0, ph / height, 0, 0);
        drawCrosshair(octx, plot, cur.tokens, at.hour);
        overlayPaintedRef.current = true;
      }
      const hour = Math.floor(at.describes);
      const content: TooltipContent = pointed?.cluster
        ? buildMarkTooltip({
          cluster: pointed.cluster,
          lineByTag: cur.reachedByTag,
          labelAt: cur.labelAt,
          decimals: cur.effectiveDecimals,
          digits: cur.digits,
        })
        : buildMeasureTooltip({
          hour,
          series: cur.seriesItems,
          lineByTag: cur.reachedByTag,
          hourLoaded: cur.hourLoaded,
          labelAt: cur.labelAt,
          decimals: cur.effectiveDecimals,
          digits: cur.digits,
          unit: cur.declaration.unit,
          badges: at.badges,
          viewportHeight: cur.viewportHeight,
        });

      let pt: { clientX: number; clientY: number } | null = pointed ? pointer : null;
      if (!pt) {
        const rect = hostEl.getBoundingClientRect();
        pt = {
          clientX: rect.left + plot.toX(at.hour) * rect.width / width,
          clientY: rect.top + (plot.plotTop + 8) * rect.height / height,
        };
      }
      const held = tooltipStore.get();
      const intent = keyIntentRef.current;
      const fresh = intent === true || (intent !== false && tooltipStore.revision() === intent.since);
      if (pointed || fresh || held == null || held.owner === owner) {
        showTooltip(owner, content, pt.clientX, pt.clientY, `${hour}:${at.badges.map(b => `${b.hour}|${b.label}`).join(',')}`);
        if (hit?.by === 'keyboard') announce(tooltipText(content));
      }
    } else {
      hideTooltip(owner);
    }
    keyIntentRef.current = false;

    hostEl.style.cursor = drag?.mode === 'pan'
      ? 'grabbing'
      : pointed?.badges.length
        ? 'pointer'
        : zone === 'x-axis'
          ? 'ew-resize'
          : zone === 'y-axis'
            ? 'ns-resize'
            : zone === 'plot'
              ? 'crosshair'
              : 'default';
  }, [owner, announce]);
  const syncOverlayRef = useRef(syncOverlayAndTooltip);
  syncOverlayRef.current = syncOverlayAndTooltip;

  useEffect(() => {
    if (hoverRef.current.pointer || hoverRef.current.keyed !== null) {
      syncOverlayRef.current();
    }
  }, [reachedByTag, labelAt, effectiveDecimals, digits, viewportHeight]);

  useEffect(() => {
    if (!active) {
      hideTooltip(owner);
      setHoverHour(null);
    }
    return () => {
      if (pendingHoverRef.current) {
        clearTimeout(pendingHoverRef.current.timer);
        pendingHoverRef.current = null;
      }
      hideTooltip(owner);
    };
  }, [active, owner]);

  useFamilyPauseFlush(runtime.clock, () => {
    const vp = lastViewportRef.current;
    if (vp) ensureTilesRef.current(vp);
    bindRef.current();
  });

  useLayoutEffect(() => {
    const hostEl = chartHostRef.current;
    if (!hostEl) return;
    const readSize = () => {
      const w = Math.round(hostEl.clientWidth);
      const h = Math.round(hostEl.clientHeight);
      if (w !== chartSizeRef.current.width || h !== chartSizeRef.current.height) {
        chartSizeRef.current = { width: w, height: h };
        setChartSize({ width: w, height: h });
      }
    };
    const observer = new ResizeObserver(entries => {
      const rect = entries[0]?.contentRect;
      const w = Math.round(rect ? rect.width : hostEl.clientWidth);
      const h = Math.round(rect ? rect.height : hostEl.clientHeight);
      if (w !== chartSizeRef.current.width || h !== chartSizeRef.current.height) {
        chartSizeRef.current = { width: w, height: h };
        setChartSize({ width: w, height: h });
      }
    });
    observer.observe(hostEl);
    readSize();
    return () => observer.disconnect();
  }, [blockedState !== null]);

  useLayoutEffect(() => {
    const surfaceHost = surfaceHostRef.current;
    if (!surfaceHost || !pipeline || pipe === null) return;
    lastViewportRef.current = null;
    lastFrameRef.current = null;
    lastLegendObjRef.current = null;
    hasPictureRef.current = false;
    lastBindSigRef.current = '';
    let lastEventsMarks: readonly MarkSummary[] | null = null;
    let lastEventsBadgeCount = -1;

    const mounted = mountFamilyCanvas(surfaceHost, pipeline, pipe, 'time-chart', 'tc-canvas', 'tc-extent');
    surfaceRef.current = mounted;

    const auditWatch = attachAuditWatch(pipeline, pipe, {
      isAuditable: () => !!perfLog && !!latestRef.current.track && latestRef.current.resolvedTags.length > 0,
      onCaptureChange: activeAudit => { latestRef.current.captureActive = activeAudit; },
      bindNow: () => bindRef.current(),
    });

    const install = () => {
      const shown = pipeline.view(pipe);
      const frameNode = frameRef.current;
      const visible = shown?.mode === 'picture';
      if (visible) hasPictureRef.current = true;
      const keepPrevious = !visible && hasPictureRef.current && shown?.mode === 'reading';
      const tileRefreshOnly = !!(
        visible
        && shown?.dim
        && tileBindGenRef.current !== null
        && shown.binding === tileBindGenRef.current
        && shown.installedBinding !== undefined
        && shown.installedBinding < shown.binding
      );
      if (visible && tileBindGenRef.current !== null && shown?.installedBinding !== undefined && shown.installedBinding >= tileBindGenRef.current) {
        tileBindGenRef.current = null;
      }
      holdElement(frameNode, (!!shown?.dim && !tileRefreshOnly) || keepPrevious);
      surfaceHost.toggleAttribute('data-tc-visible', visible || keepPrevious);
      surfaceHost.toggleAttribute('data-tc-pending', !visible && !keepPrevious);

      const mode = keepPrevious ? 'picture' : (shown?.mode ?? 'reading');
      const reason = shown?.reason ?? null;
      const failureOrigin: FailureOrigin = shown?.failureOrigin ?? 'local';
      reportFamilyShownHour(reportShownRef.current, visible, shown);

      auditWatch.settleIfReady(visible, mode, shown?.installedBinding, shown?.fields);

      const nextMetaSig = `${mode}|${reason ?? ''}|${failureOrigin}`;
      if (nextMetaSig !== metaSigRef.current) {
        metaSigRef.current = nextMetaSig;
        setMeta({ mode, reason, failureOrigin });
      }

      if (!visible || !shown?.fields) return;

      const vpField = shown.fields.viewport as TimeChartViewportField | undefined;
      if (vpField && vpField !== lastViewportRef.current) {
        lastViewportRef.current = vpField;
        if (vpField.binHours !== activeBinHoursRef.current) {
          setViewportBinHours(vpField.binHours);
        }
        ensureTilesRef.current(vpField);
      }

      const frameField = shown.fields.frame as TimeChartFrameField | undefined;
      if (frameField && frameField !== lastFrameRef.current) {
        lastFrameRef.current = frameField;
        const cur = latestRef.current;
        if (
          eventsTextRef.current
          && (cur.badges.length !== lastEventsBadgeCount || frameField.markSummaries !== lastEventsMarks)
        ) {
          lastEventsBadgeCount = cur.badges.length;
          lastEventsMarks = frameField.markSummaries;
          const drawnBadges = cur.badges.length ? visibleBadges(cur.badges, frameField.x, frameField.until) : [];
          const spokenBadges = drawnBadges.map(b => `${dayLabel(b.hour)}, ${b.label}`).join('; ');
          const clusterText = frameField.markSummaries
            .flatMap(summary => shownClusters(summary, frameField.until))
            .map(cluster => cluster.badge
              ? `${dayLabel(cluster.badge.hour)}, ${cluster.badge.label}`
              : `${dayLabel(cluster.firstHour) === dayLabel(cluster.lastHour) ? dayLabel(cluster.firstHour) : `${dayLabel(cluster.firstHour)} to ${dayLabel(cluster.lastHour)}`}, ${eventCount(cluster.count)}`)
            .join('; ');
          const text = spokenBadges && clusterText ? `${spokenBadges}; ${clusterText}` : spokenBadges || clusterText;
          if (eventsTextRef.current.textContent !== (text ? `Events: ${text}.` : '')) {
            eventsTextRef.current.textContent = text ? `Events: ${text}.` : '';
          }
        }
        if (hoverRef.current.pointer || hoverRef.current.keyed !== null || dragRef.current) {
          syncOverlayRef.current();
        }
      }

      if (latestRef.current.seriesItems.length >= 2) {
        const legendField = shown.fields.legend as Readonly<Record<string, TimeChartLegendReading>> | undefined;
        if (legendField && legendField !== lastLegendObjRef.current) {
          lastLegendObjRef.current = legendField;
          setLegendReadings(legendField);
        }
      }
    };

    install();
    const stopInstall = pipeline.onInstall(pipe, install);
    bindRef.current();

    return () => {
      auditWatch.dispose();
      stopInstall();
      mounted.unmount();
      surfaceRef.current = null;
      holdElement(frameRef.current, false);
    };
  }, [pipeline, pipe]);

  useLayoutEffect(() => {
    if (pipe !== null && enabled && chartSize.width > 0 && chartSize.height > 0) {
      bind();
    }
  }, [
    pipe,
    enabled,
    subsReady,
    chartSize.width,
    chartSize.height,
    tokens,
    extent,
    playback,
    afterPlayhead,
    windowHours,
    paused,
    staticPlayhead,
    view,
    axes,
    padding,
    areaFill,
    endLabels,
    spendTicks,
    digits,
    effectiveDecimals,
    seriesMeta,
    badges,
    staticHead,
    bind,
  ]);

  useEffect(() => {
    if (!canQuery || !track || chartSize.width <= 0) return;
    const targetX = resolveTimeChartTargetX({
      extent,
      playhead,
      playback,
      windowHours,
      paused,
      view,
      axes,
      branchStart: track.branch.firstHour,
    });
    const to = Math.ceil(Math.min(targetX[1], until ?? targetX[1]));
    const tileBounds = initialTileBounds(Math.floor(targetX[0]), to, lastHour, activeBinHours, AHEAD_BINS);
    ensureTilesForViewport({ binHours: activeBinHours, firstTile: tileBounds.firstTile, lastTile: tileBounds.lastTile });
  }, [canQuery, track, chartSize.width, extent, playhead, playback, windowHours, paused, view, axes, until, lastHour, activeBinHours, resolved.tags, spendTicks, membership, ensureTilesForViewport]);

  const dispatchView = useCallback((action: ViewAction) => {
    const frameField = lastFrameRef.current;
    if (!frameField) return;
    const next = reduceView(viewRef.current, action, {
      x: frameField.x,
      y: frameField.y,
      extent: latestRef.current.extent,
      minSpan: TIME_CHART_MIN_SPAN,
      log: frameField.log,
    });
    viewRef.current = next;
    setView(next);
  }, []);

  useEffect(() => {
    const hostEl = chartHostRef.current;
    if (!hostEl) return;

    const local = (event: PointerEvent | WheelEvent | MouseEvent): Point => {
      const point = toLocal(event.clientX, event.clientY, hostEl.getBoundingClientRect(), hostEl.clientWidth, hostEl.clientHeight);
      return { ...point, clientX: event.clientX, clientY: event.clientY };
    };

    const currentPlot = (): PlotFrame | null => {
      const f = lastFrameRef.current;
      const { width, height } = chartSizeRef.current;
      return f && width > 0 && height > 0 ? reconstructPlotFrame(f, width, height) : null;
    };

    const down = (event: PointerEvent) => {
      pendingNavRef.current = null;
      hoverRef.current = reduceHover(hoverRef.current, { type: 'pointer-down' });
      const plot = currentPlot();
      if (!plot || (event.button !== 0 && event.button !== 1)) return;
      const point = local(event);
      const zone = hitZone(point, plot);
      if (!zone) return;
      if (event.button === 0 && zone === 'x-axis') dragRef.current = { mode: 'axis-x', grab: plot.hourAt(point.x) };
      else if (event.button === 0 && zone === 'y-axis') dragRef.current = { mode: 'axis-y', grab: plot.valueAt(point.y) };
      else if (zone === 'plot' && (event.button === 1 || event.shiftKey)) dragRef.current = { mode: 'pan', lastX: point.x };
      else if (zone === 'plot') dragRef.current = { mode: 'box', x0: point.x, y0: point.y, x1: point.x, y1: point.y };
      else return;
      event.preventDefault();
      hostEl.focus({ preventScroll: true });
      hostEl.setPointerCapture(event.pointerId);
      syncOverlayRef.current();
    };

    const move = (event: PointerEvent) => {
      const point = local(event);
      pointerRef.current = point;
      hoverRef.current = reduceHover(hoverRef.current, { type: 'pointer', point });
      const plot = currentPlot();
      const drag = dragRef.current;
      if (drag && plot) {
        if (drag.mode === 'axis-x') dispatchView({ type: 'axis-x', grab: drag.grab, fraction: axisFraction(point, plot, 'x') });
        else if (drag.mode === 'axis-y') dispatchView({ type: 'axis-y', grab: drag.grab, fraction: axisFraction(point, plot, 'y') });
        else if (drag.mode === 'pan') {
          dispatchView({ type: 'pan', hours: (drag.lastX - point.x) / plot.plotWidth * (plot.x[1] - plot.x[0]) });
          drag.lastX = point.x;
        } else {
          drag.x1 = point.x;
          drag.y1 = point.y;
        }
      }
      syncOverlayRef.current();
    };

    const up = () => {
      const finished = dragRef.current;
      dragRef.current = null;
      const plot = currentPlot();
      if (finished?.mode === 'box' && plot && Math.abs(finished.x1 - finished.x0) > BOX_THRESHOLD) {
        dispatchView({
          type: 'box',
          x: [plot.hourAt(finished.x0), plot.hourAt(finished.x1)],
          y: Math.abs(finished.y1 - finished.y0) > BOX_THRESHOLD ? [plot.valueAt(finished.y1), plot.valueAt(finished.y0)] : undefined,
        });
      }
      syncOverlayRef.current();
    };

    const cancel = () => {
      dragRef.current = null;
      syncOverlayRef.current();
    };

    const leave = () => {
      pendingNavRef.current = null;
      pointerRef.current = null;
      if (pendingHoverRef.current) {
        clearTimeout(pendingHoverRef.current.timer);
        pendingHoverRef.current = null;
      }
      hoverRef.current = reduceHover(hoverRef.current, { type: 'pointer-leave' });
      syncOverlayRef.current();
    };

    const wheel = (event: WheelEvent) => {
      const plot = currentPlot();
      if (!plot) return;
      const point = local(event);
      const steps = wheelSteps(event.deltaY, event.deltaMode);
      const zone = hitZone(point, plot);
      if (zone !== 'plot' || !steps) return;
      event.preventDefault();
      dispatchView({ type: 'wheel', at: plot.hourAt(point.x), steps });
    };

    const doubleClick = (event: MouseEvent) => {
      const plot = currentPlot();
      if (!plot) return;
      const zone = hitZone(local(event), plot);
      dispatchView({ type: zone === 'x-axis' ? 'reset-x' : zone === 'y-axis' ? 'reset-y' : 'reset' });
    };

    const openContextMenu = (hour: number, clientX: number, clientY: number) => {
      const cur = latestRef.current;
      if (!cur.track) return;
      const items = [jumpItem(cur.runtime, cur.track.id, hour)];
      if (cur.declaration.balanceChanges && cur.seriesItems.length > 0) {
        items.push({
          label: 'Balance changes',
          run: () => {
            const cluster = hoverClusterRef.current;
            const tag = cluster?.tag ?? cur.seriesItems[0]!.tag;
            const f = lastFrameRef.current;
            const summary = f?.markSummaries.find(s => s.clusters.some(c => c.tag === tag)) ?? null;
            const drawnUntil = f?.until ?? cur.until;
            const { first, end } = changesRange(summary, cluster, drawnUntil);
            changeListIdRef.current += 1;
            setChangeList({ id: changeListIdRef.current, tag, first, end });
          },
        });
      }
      menuRef.current.open(clientX, clientY, items);
    };

    const onMenu = (event: MouseEvent) => {
      const fromPointer = pointerMenu(hoverRef.current);
      hoverRef.current = reduceHover(hoverRef.current, { type: 'contextmenu' });
      if (!fromPointer) { event.preventDefault(); return; }
      const plot = currentPlot();
      if (!plot) return;
      const point = local(event);
      const zone = hitZone(point, plot);
      if (zone !== 'plot') return;
      const currentUntil = lastFrameRef.current?.until ?? null;
      const target = pointerTarget(zone, pointerHour(plot.hourAt(point.x), plot.x, currentUntil), plot.x, currentUntil, latestRef.current.extent);
      if (!target) return;
      event.preventDefault();
      openContextMenu(Math.floor(target.describes), event.clientX, event.clientY);
    };

    hostEl.addEventListener('pointerdown', down);
    hostEl.addEventListener('pointermove', move);
    hostEl.addEventListener('pointerup', up);
    hostEl.addEventListener('pointercancel', cancel);
    hostEl.addEventListener('pointerleave', leave);
    hostEl.addEventListener('wheel', wheel, { passive: false });
    hostEl.addEventListener('dblclick', doubleClick);
    hostEl.addEventListener('contextmenu', onMenu);
    return () => {
      hostEl.removeEventListener('pointerdown', down);
      hostEl.removeEventListener('pointermove', move);
      hostEl.removeEventListener('pointerup', up);
      hostEl.removeEventListener('pointercancel', cancel);
      hostEl.removeEventListener('pointerleave', leave);
      hostEl.removeEventListener('wheel', wheel);
      hostEl.removeEventListener('dblclick', doubleClick);
      hostEl.removeEventListener('contextmenu', onMenu);
    };
  }, [dispatchView, blockedState !== null]);

  const navigateHour = useCallback((key: 'ArrowLeft' | 'ArrowRight' | 'Home' | 'End', from: number | null, x: Range, activeUntil: number | null) => {
    const cur = latestRef.current;
    if (!cur.track || !cur.resolvedTags.length || !cur.active) return;
    navAbortRef.current?.abort();
    const controller = new AbortController();
    navAbortRef.current = controller;
    const range = cur.gateSpan(x[0], Math.min(x[1], activeUntil ?? x[1]));
    if (!range) return;
    const since = tooltipStore.revision();
    const expectedHover = hoverRef.current;
    pendingNavRef.current = { hover: expectedHover, tooltip: since };
    keyIntentRef.current = { since };
    void loadTransientProduct<{ hour: unknown }>(
      cur.runtime.client,
      forwardRequest(cur.track.source, 'series-next-change', {
        branchId: cur.track.branch.id,
        family: { kind: cur.declaration.family, measure: cur.declaration.measure },
        countries: cur.resolvedTags.map(countryFromKey),
        ...withinParams(cur.resolvedTags, cur.membership, cur.gateSpan),
        key,
        from: cur.gateWithin(from),
        ...range,
      }),
      controller.signal,
    ).then(reply => {
      if (controller.signal.aborted) return;
      if (reply.hour !== null && (typeof reply.hour !== 'number' || !Number.isFinite(reply.hour))) {
        throw new Error('The keyboard reply is malformed.');
      }
      const pending = pendingNavRef.current;
      if (!pending || pending.hover !== hoverRef.current) return;
      pendingNavRef.current = null;
      keyIntentRef.current = { since: pending.tooltip };
      hoverRef.current = reduceHover(hoverRef.current, { type: 'key', hour: reply.hour as number | null });
      syncOverlayRef.current();
    }).catch(error => {
      if (!controller.signal.aborted) setNavError(asFailure(error));
    });
  }, []);

  const clearKeyed = useCallback(() => {
    pendingNavRef.current = null;
    hoverRef.current = reduceHover(hoverRef.current, { type: 'key-clear' });
    syncOverlayRef.current();
  }, []);

  const onKeyDown = useCallback((event: ReactKeyboardEvent) => {
    const frameField = lastFrameRef.current;
    if ((event.key === 'ArrowLeft' || event.key === 'ArrowRight' || event.key === 'Home' || event.key === 'End') && frameField) {
      event.preventDefault();
      if (Math.min(frameField.x[1], frameField.until ?? frameField.x[1]) < frameField.x[0]) {
        pendingNavRef.current = null;
        hoverRef.current = reduceHover(hoverRef.current, { type: 'key', hour: null });
        syncOverlayRef.current();
        return;
      }
      navigateHour(event.key, hoverRef.current.keyed ?? shownHourRef.current, frameField.x, frameField.until);
      return;
    }
    if (event.key === 'Escape') {
      pendingNavRef.current = null;
      if (hoverRef.current.keyed !== null) {
        event.preventDefault();
        clearKeyed();
        return;
      }
      if (viewRef.current.x !== null || viewRef.current.y !== null) {
        event.preventDefault();
        dispatchView({ type: 'reset' });
        return;
      }
    }
    const cur = latestRef.current;
    const moment = actionHour(shownHourRef.current, hoverRef.current, frameField?.playhead ?? cur.staticPlayhead);
    if ((event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) && moment !== null && cur.track) {
      pendingNavRef.current = null;
      event.preventDefault();
      hoverRef.current = reduceHover(hoverRef.current, { type: 'menu-key' });
      const boundsRect = chartHostRef.current?.getBoundingClientRect();
      if (!boundsRect) return;
      const items = [jumpItem(cur.runtime, cur.track.id, Math.floor(moment))];
      if (cur.declaration.balanceChanges && cur.seriesItems.length > 0) {
        items.push({
          label: 'Balance changes',
          run: () => {
            const tag = cur.seriesItems[0]!.tag;
            const summary = frameField?.markSummaries.find(s => s.clusters.some(c => c.tag === tag)) ?? null;
            const { first, end } = changesRange(summary, null, frameField?.until ?? cur.until);
            changeListIdRef.current += 1;
            setChangeList({ id: changeListIdRef.current, tag, first, end });
          },
        });
      }
      menuRef.current.open(boundsRect.left + boundsRect.width / 2, boundsRect.top + boundsRect.height / 2, items);
    }
  }, [navigateHour, clearKeyed, dispatchView]);

  const legendItems = useMemo<LegendItem[]>(() => {
    if (seriesItems.length < 2) return [];
    const at = Math.floor(head);
    return seriesItems.map(item => {
      const reading = legendReadings?.[item.tag];
      let value: string;
      if (following) {
        value = formatLegendReading(reading, effectiveDecimals, digits, legendReadings !== null);
      } else {
        const line = staticHead?.lines.find(entry => countryKey(entry.country) === item.tag);
        const staticReading: TimeChartLegendReading | undefined = line
          ? {
            valueRaw: line.value.valueRaw,
            value: line.value.value,
            missing: line.value.missing ?? null,
            notApplicable: line.value.notApplicable ?? null,
          }
          : undefined;
        const raw = isMember(item.membership, at) ? runAt(reachedByTag.get(item.tag)?.runs ?? [], at)?.valueRaw ?? null : null;
        if (staticReading) {
          value = formatLegendReading(staticReading, effectiveDecimals, digits, true);
        } else if (raw !== null && effectiveDecimals !== null) {
          value = formatLegendReading({ valueRaw: raw, value: null, missing: null, notApplicable: null }, effectiveDecimals, digits, true);
        } else {
          value = 'Loading…';
        }
      }
      return {
        id: item.tag,
        label: labelAt(shownAt)(item.tag).name,
        color: item.hex,
        value,
        flag: (
          <CountryFlagUrl
            className="country-flag"
            url={legendLooks.flag(item.tag, shownAt)}
            pending={legendLooks.flagPending(item.tag, shownAt)}
            detail={legendLooks.flagDetail?.(item.tag, shownAt)}
            reason={legendLooks.flagReason(item.tag, shownAt)}
          />
        ),
      };
    });
  }, [seriesItems, head, legendReadings, following, effectiveDecimals, digits, staticHead, reachedByTag, labelAt, shownAt, legendLooks]);

  const retryAll = useCallback(() => {
    setTileError(null);
    setNavError(null);
    lastTileSigRef.current = '';
    lastBindSigRef.current = '';
    const vp = lastViewportRef.current;
    if (vp) ensureTilesForViewport(vp);
    retryLocal();
  }, [ensureTilesForViewport, retryLocal]);

  if (blockedState) return blockedState;

  const bodyState = renderFamilyBodyState({
    mode: meta.mode,
    reason: meta.reason,
    failureOrigin: meta.failureOrigin,
    empty: null,
    subject: declaration.noun,
    loadingLabel: `Loading ${declaration.noun}…`,
    pipeline,
    pipe,
    onRetryStream: retryStream,
    onRetryLocal: retryAll,
  });

  const bannerError = tileError ?? navError;
  const chartLabel = `${declaration.title} of ${seriesItems.map(item => labelAt(shownAt)(item.tag).name).join(', ')}`;

  return (
    <div className="country-measure" ref={frameRef}>
      <div className="country-measure-view">
        {bannerError && <WidgetState kind="error" scale="section" failure={bannerError} onRetry={retryAll} />}
        <Legend label="Countries" items={legendItems} windowed onVisibleItems={onVisibleItems} />
        <div className="country-measure-chart">
          <div
            className="time-chart tc-host"
            ref={chartHostRef}
            tabIndex={0}
            role="img"
            aria-label={chartLabel}
            aria-describedby={`${owner}-events`}
            aria-keyshortcuts="ArrowLeft ArrowRight Home End"
            onKeyDown={onKeyDown}
            onBlur={clearKeyed}
          >
            <div className="tc-surface" ref={surfaceHostRef} />
            <canvas ref={overlayRef} className="time-chart-overlay" aria-hidden="true" />
            <p ref={eventsTextRef} id={`${owner}-events`} className="sr-only" />
            {!hasFrame && <div ref={liveRegionRef} className="tc-live-region" aria-live="polite" />}
            {zoomed && <ZoomReset onReset={() => dispatchView({ type: 'reset' })} />}
            {bodyState && <div className="tc-state-overlay">{bodyState}</div>}
            {!bodyState && notRecorded && (
              <div className="time-chart-state">
                <WidgetState kind="unavailable" message="Not recorded" detail={notRecorded} />
              </div>
            )}
          </div>
        </div>
        {changeList && track && (
          <BalanceChanges
            key={changeList.id}
            client={runtime.client}
            gate={gate}
            source={track.source}
            branchId={track.branch.id}
            measure={declaration.measure}
            tags={resolved.tags}
            initialTag={changeList.tag}
            firstOrdinal={changeList.first}
            endOrdinal={changeList.end}
            digits={digits}
            unit={declaration.unit}
            visibleThrough={until ?? extent[1]}
            label={tag => labelAt(shownAt)(tag).name}
            onClose={() => setChangeList(null)}
            onPick={(_tag, h) => jumpItem(runtime, track.id, h).run()}
          />
        )}
        {menu.menu}
      </div>
    </div>
  );
}
