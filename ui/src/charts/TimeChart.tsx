import { useEffect, useId, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent } from 'react';
import { ZoomReset } from './ZoomReset';
import { setStyle } from './domText.ts';
import WidgetState from '../components/WidgetState';
import { inFrame } from '../dashboard/frameUpdates';
import { badgesAt, visibleBadges } from './badges';
import type { EventBadge } from './badges';
import { clusterHour, eventCount, markClusterAt, markSummaryMatches, shownClusters } from './markSummary';
import type { MarkCluster, MarkSummary } from './markSummary';
import { dayLabel, drawCrosshair, drawSelection } from './draw';
import { approach, easeTau } from './easing';
import { useMotionNow } from './liveMotion.ts';
import { keyTarget } from './rowNavigation';
import { axisFraction, hitZone, keyedHour, pointerHour, wheelSteps } from './frame';
import { actionHour, hoverAt, keyedStop, keyStopMemo, NO_HOVER, pointerMenu, pointerTarget, reduceHover } from './hover';
import type { HoverEvent, HoverState, KeyStop } from './hover';
import { intervalIndexOf } from './intervalIndex';
import { lostAt, lostTooltip } from './gaps';
import type { LostSpan } from './gaps';
import type { ChartStrip } from './strip';
import type { Padding, PlotFrame } from './frame';
import { ChartPicks } from './pick';
import type { ChartPick } from './pick';
import { scheduler } from './scheduler';
import type { FrameInfo, FrameTask } from './scheduler';
import { drawingSurface, sizeCanvas, toLocal, watchCanvasSize } from './surface';
import { readTokens, subscribeTokens } from './tokens';
import type { ChartTokens } from './tokens';
import { hideTooltip, showTooltip, tooltipStore, tooltipText } from './tooltip';
import type { TooltipContent } from './tooltip';
import { EMPTY_VIEW, reduceView, shownUntil } from './view';
import type { AxisSettings, PlaybackMode, Range, ViewAction, ViewState } from './view';
import type { Live } from './live';
import { canvasMeasureText, scratchMeasureText } from './textWidth';
import { attachTimeChartWorker, supportsTimeChartWorker } from './timeChartWorkerClient';
import {
  TIME_CHART_MIN_SPAN,
  drawTimeChartScene,
  fitTimeChartFrame,
  normalizeEndLabels,
  resolveTimeChartPadding,
  resolveTimeChartTargetX,
  resolveTimeChartTargetY,
  type AxisValueFormatter,
  type ChartAxisFormatterId,
  type ChartLayer as PureChartLayer,
  type DeclarativeLayer,
  type EndLabelsResolver,
} from './renderTimeChart';
import './charts.css';

export type ChartLayer = PureChartLayer<CanvasRenderingContext2D>;

export type { ChartStrip } from './strip';

export type { ChartPick } from './pick';

export interface TimeChartProps {
  rows?: { labels: readonly string[]; labelWidth: number; tooltip: (row: number, hour: number) => TooltipContent | null };
  label: string;
  extent: Range;
  playhead: number | null;
  liveHead?: LiveHead;
  playback: PlaybackMode;
  afterPlayhead?: boolean;
  windowHours?: number;
  hoursPerSecond?: number;
  axes: AxisSettings;
  fitValues?: (shown: Range | null) => Range | null;
  branchStart: number;
  padding?: Partial<Padding>;
  layers: readonly ChartLayer[];
  gaps?: readonly LostSpan[];
  strip?: ChartStrip;
  formatValue?: ChartAxisFormatterId | AxisValueFormatter;
  valueTicks?: readonly number[];
  endLabels?: EndLabelsResolver;
  badges?: readonly EventBadge[];
  markSummary?: MarkSummary;
  markSummaries?: readonly MarkSummary[] | (() => readonly MarkSummary[]);
  markTooltip?: (cluster: MarkCluster) => TooltipContent | null;
  onMarkHover?: (cluster: MarkCluster | null) => void;
  badgesAtHour?: (hour: number) => readonly EventBadge[];
  onSummaryView?: (view: { from: number; to: number; end: number; width: number }) => void;
  onNavigateHour?: (key: 'ArrowLeft' | 'ArrowRight' | 'Home' | 'End', from: number | null, x: Range, until: number | null) => void;
  keyboardHour?: number | null;
  keyboardRevision?: unknown;
  tooltip?: (hour: number, frame: PlotFrame, badges: readonly EventBadge[]) => TooltipContent | null;
  onHover?: (hour: number | null) => void;
  snap?: (hour: number, frame: PlotFrame) => number;
  keyHours?: readonly number[];
  overlay?: (ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens) => void;
  onViewChange?: (x: Range) => void;
  onContextMenu?: (hour: number, clientX: number, clientY: number) => void;
  onPick?: (hour: number, context?: ChartPick) => void;
  pickOwner?: unknown;
  active: boolean;
  notRecorded?: string | null;
  revision: unknown;
  data?: ChartData;
}

export interface ChartData { revision(): readonly unknown[]; subscribe(listener: () => void): () => void }

const marksOf = (p: TimeChartProps): readonly MarkSummary[] => [...(p.markSummary ? [p.markSummary] : []),
  ...(typeof p.markSummaries === 'function' ? p.markSummaries() : p.markSummaries ?? [])];

const MIN_SPAN = TIME_CHART_MIN_SPAN;
const SUMMARY_INTERVAL_MS = 150;
const HOVER_REST_MS = 80;
const BOX_THRESHOLD = 8;

type Pointed = { hour: number; describes: number; badges: readonly EventBadge[]; cluster?: MarkCluster };

type Drag = { mode: 'axis-x'; grab: number } | { mode: 'axis-y'; grab: number } | { mode: 'pan'; lastX: number } | { mode: 'box'; x0: number; y0: number; x1: number; y1: number; timeOnly: boolean };

export type LiveHead = Live<number>;

function withLiveHead(props: TimeChartProps): TimeChartProps {
  const live = props.liveHead;
  if (!live || props.playhead == null) return props;
  const head = Math.max(props.extent[0], Math.min(props.extent[1], live.read()));
  return head === props.playhead ? props : { ...props, playhead: head };
}

function canDrawInWorker(p: TimeChartProps): boolean {
  return p.layers.every(layer => typeof layer.describe === 'function')
    && !p.strip
    && !p.overlay
    && typeof p.formatValue !== 'function';
}

function sampleWorkerLayers(layers: readonly ChartLayer[], drawUntil: number): {
  descriptors: DeclarativeLayer[];
  headValues: (number | null)[];
} | null {
  const descriptors: DeclarativeLayer[] = [];
  const headValues: (number | null)[] = [];
  for (const layer of layers) {
    const desc = layer.describe?.();
    if (!desc) return null;
    descriptors.push(desc);
    const tiled = desc.kind === 'line-summary' ? desc.summary?.tiles : undefined;
    headValues.push(tiled ? tiled.valueAt(drawUntil) : null);
  }
  return { descriptors, headValues };
}

export default function TimeChart(props: TimeChartProps) {
  const owner = useId();
  const canvas = useRef<HTMLCanvasElement>(null);
  const overlay = useRef<HTMLCanvasElement>(null);
  const latest = useRef(props);
  latest.current = props;
  const motionNow = useMotionNow();
  const [zoomed, setZoomed] = useState(false);
  const [workerFailed, setWorkerFailed] = useState(false);
  const useWorker = canDrawInWorker(props) && !workerFailed && supportsTimeChartWorker();
  const viewRef = useRef<ViewState>(EMPTY_VIEW);
  const easedRef = useRef<{ x: [number, number]; y: [number, number] } | null>(null);
  const zoomedForRef = useRef('');
  const eventsText = useRef<HTMLParagraphElement>(null);
  type Point = { x: number; y: number; clientX: number; clientY: number };
  const hover = useRef<HoverState<Point>>(NO_HOVER);
  const pendingNavigation = useRef<{ hover: HoverState<Point>; owner: unknown; tooltip: number } | null>(null);
  const shownHour = useRef<number | null>(null);
  const hoverObserver = useRef<{ callback: NonNullable<TimeChartProps['onHover']>;
    pickOwner: unknown; branchStart: number; first: number; last: number; hour: number | null } | undefined>(undefined);
  const rowRef = useRef(0);
  const keyedStopRef = useRef<KeyStop | null>(null);
  const keyStopsFor = useRef(keyStopMemo()).current;
  const [keySpoken, setKeySpoken] = useState('');
  const keyText = useRef('');
  const keyIntent = useRef<boolean | { since: number }>(false);
  const shown = useRef<{ x: Range; until: number | null } | null>(null);
  const picks = useRef(new ChartPicks()).current;
  picks.update({ owner: props.pickOwner, extent: props.extent, branchStart: props.branchStart, active: props.active, onPick: props.onPick });
  const pickFrame = useRef<ReturnType<ChartPicks['drawn']> | null>(null);
  const task = useRef<FrameTask | null>(null);
  const controls = useRef<{ dispatch: (action: ViewAction) => void } | null>(null);

  useEffect(() => {
    const element = canvas.current!;
    type PaintedSnapshot = {
      readonly frame: PlotFrame;
      readonly layoutWidth: number;
      readonly layoutHeight: number;
      readonly backingWidth: number;
      readonly backingHeight: number;
      readonly scaleX: number;
      readonly scaleY: number;
      readonly until: number | null;
      readonly extent: Range;
      readonly badges: readonly EventBadge[] | undefined;
      readonly markSummaries: readonly MarkSummary[];
      readonly rows: TimeChartProps['rows'];
      readonly gaps: TimeChartProps['gaps'];
      readonly strip: TimeChartProps['strip'];
      readonly snap: TimeChartProps['snap'];
      readonly tooltip: TimeChartProps['tooltip'];
      readonly markTooltip: TimeChartProps['markTooltip'];
      readonly badgesAtHour: TimeChartProps['badgesAtHour'];
      readonly onMarkHover: TimeChartProps['onMarkHover'];
      readonly onContextMenu: TimeChartProps['onContextMenu'];
      readonly revision: TimeChartProps['revision'];
      readonly pick: ReturnType<ChartPicks['drawn']>;
    };
    let lastPostedFrameId = 0;
    let lastPaintedFrameId = 0;
    const postedFrames = new Map<number, PaintedSnapshot>();
    let paintedState: PaintedSnapshot | null = null;
    let drag: Drag | null = null;
    let wakeRef = () => {};
    const worker = useWorker ? attachTimeChartWorker(element, {
      onFallback: () => { setWorkerFailed(true); },
      onPainted: (frameId: number) => {
        const entry = postedFrames.get(frameId);
        if (!entry || frameId <= lastPaintedFrameId) return;
        lastPaintedFrameId = frameId;
        for (const key of postedFrames.keys()) if (key <= frameId) postedFrames.delete(key);
        paintedState = entry;
        pickFrame.current = entry.pick;
        shown.current = { x: entry.frame.x, until: entry.until };
        if (frameId >= lastPostedFrameId && task.current) scheduler.setAsyncPending(task.current, false);
        if (hover.current !== NO_HOVER || drag != null) wakeRef();
      },
    }) : null;
    if (useWorker && !worker) {
      setWorkerFailed(true);
      return;
    }
    const drawing = worker ? null : drawingSurface(element);
    const context = drawing?.context ?? null;
    const measure = worker ? (scratchMeasureText() ?? (() => 0)) : canvasMeasureText(context!);
    const sheet = overlay.current ? drawingSurface(overlay.current) : null, overlayContext = sheet?.context ?? null;
    let tokens = readTokens();
    let spokenFor: readonly EventBadge[] = [];
    let spokenBadges = '', spokenText = '';
    let spokenMarks: readonly (readonly MarkCluster[])[] = [];
    let clusterText = '';
    let reported: { handler: (x: Range) => void; x: Range } | null = null;
    let telling: { handler: (x: Range) => void; x: Range } | null = null;
    let painted: readonly unknown[] | null = null;
    let overlaid = true;
    const surface = watchCanvasSize(element, () => wake());
    let pointer: { x: number; y: number; clientX: number; clientY: number } | null = null;
    let hovered: { hour: number; row: number; badges: readonly EventBadge[]; cluster?: MarkCluster; markTooltip: TimeChartProps['markTooltip']; rows: TimeChartProps['rows']; tooltip: TimeChartProps['tooltip']; revision: unknown; content: TooltipContent | null; key: string } | null = null;
    let keyedBadges: { source: TimeChartProps['badges']; hour: number; badges: readonly EventBadge[] } | null = null;
    let markHovered: MarkCluster | null | undefined;
    let pendingHover: { hour: number; since: number; timer: ReturnType<typeof setTimeout> } | null = null;
    type SummaryView = { callback: TimeChartProps['onSummaryView']; from: number; to: number; end: number; width: number };
    let summaryView: SummaryView | null = null, summaryLatest: SummaryView | null = null;
    let summaryAt = -Infinity, summaryTimer: ReturnType<typeof setTimeout> | null = null;
    const sendSummary = () => {
      summaryTimer = null;
      if (!summaryLatest) return;
      const sent = summaryView = summaryLatest; summaryAt = performance.now();
      inFrame(() => sent.callback?.({ from: sent.from, to: sent.to, end: sent.end, width: sent.width }));
    };

    const activeSnapshot = () => paintedState;

    const dispatch = (action: ViewAction) => {
      const snapshot = activeSnapshot();
      if (!snapshot) return;
      const currentFrame = snapshot.frame;
      viewRef.current = reduceView(viewRef.current, action, { x: currentFrame.x, y: currentFrame.y, extent: snapshot.extent, minSpan: MIN_SPAN, log: currentFrame.log });
      setZoomed(viewRef.current.x != null || viewRef.current.y != null);
      wake();
    };
    controls.current = { dispatch };

    const draw = (info: FrameInfo): boolean => {
      const p = withLiveHead(latest.current);
      if (worker && !canDrawInWorker(p)) return false;
      const { layoutWidth, layoutHeight, backing: size } = surface.read();
      if (!layoutWidth || !layoutHeight) return false;
      if (drawing) sizeCanvas(drawing.canvas, size.width, size.height);
      if (sheet && sizeCanvas(sheet.canvas, size.width, size.height)) overlaid = false;

      const strip = p.strip ? Math.max(0, Math.round(p.strip.height(layoutHeight))) : 0;
      const marks = marksOf(p);
      const hasBadgeSummary = marks.some(summary => summary.placement !== 'ticks');
      const padding = resolveTimeChartPadding({
        layoutWidth,
        tokens,
        stripHeight: strip,
        hasBadges: !!p.badges,
        hasBadgeSummary,
        padding: p.padding,
        rowLabelWidth: p.rows?.labelWidth,
      });
      const extentKey = `${p.extent[0]}:${p.extent[1]}:${p.branchStart}`;
      if (extentKey !== zoomedForRef.current) {
        if (zoomedForRef.current && (viewRef.current.x || viewRef.current.y)) { viewRef.current = EMPTY_VIEW; setZoomed(false); }
        if (zoomedForRef.current) easedRef.current = null;
        zoomedForRef.current = extentKey;
      }
      const view = viewRef.current;
      const eased = easedRef.current;
      const motion = motionNow(p.hoursPerSecond);
      const targetX = resolveTimeChartTargetX({
        extent: p.extent,
        playhead: p.playhead,
        playback: p.playback,
        windowHours: p.windowHours,
        paused: motion === undefined,
        view,
        axes: p.axes,
        branchStart: p.branchStart,
      });
      const tau = drag && drag.mode !== 'box' ? 0 : easeTau(tokens.motion, motion, tokens.reducedMotion);
      const glide = (current: number, target: number) => target + approach(current - target, 0, info.dt, tau);
      const x: [number, number] = eased ? [glide(eased.x[0], targetX[0]), glide(eased.x[1], targetX[1])] : [targetX[0], targetX[1]];
      const plotWidth = Math.max(1, layoutWidth - padding.left - padding.right);
      const plotHeight = Math.max(1, layoutHeight - padding.top - padding.bottom);
      if ((Math.abs(x[0] - targetX[0]) + Math.abs(x[1] - targetX[1])) * plotWidth < 0.5 * Math.max(1e-9, Math.abs(targetX[1] - targetX[0]))) {
        x[0] = targetX[0]; x[1] = targetX[1];
      }
      const targetY = resolveTimeChartTargetY(p.layers, x, view, p.axes, p.fitValues);
      const y: [number, number] = eased ? [glide(eased.y[0], targetY.range[0]), glide(eased.y[1], targetY.range[1])] : [targetY.range[0], targetY.range[1]];
      const ySpan = Math.abs(targetY.range[1] - targetY.range[0]) || Math.max(1, Math.abs(targetY.range[1]));
      if ((Math.abs(y[0] - targetY.range[0]) + Math.abs(y[1] - targetY.range[1])) * plotHeight < 0.5 * ySpan) {
        y[0] = targetY.range[0]; y[1] = targetY.range[1];
      }
      easedRef.current = { x, y };
      const frame = fitTimeChartFrame({
        layoutWidth,
        layoutHeight,
        padding,
        x,
        y,
        log: targetY.log,
        stripHeight: strip,
        hasRows: !!p.rows,
        hasExplicitLeftPadding: p.padding?.left != null,
        valueTicks: p.valueTicks,
        tokens,
        formatValue: p.formatValue,
        measure,
      });

      const until = shownUntil(p.playhead, p.playback, p.afterPlayhead === true);
      const summaries = marks.filter(summary => markSummaryMatches(summary, frame, until));
      const sampled = worker ? sampleWorkerLayers(p.layers, until ?? frame.x[1]) : null;
      if (worker && !sampled) return false;
      const makeSnapshot = (): PaintedSnapshot => ({
        frame,
        layoutWidth,
        layoutHeight,
        backingWidth: size.width,
        backingHeight: size.height,
        scaleX: size.scaleX,
        scaleY: size.scaleY,
        until,
        extent: [p.extent[0], p.extent[1]],
        badges: p.badges,
        markSummaries: summaries,
        rows: p.rows,
        gaps: p.gaps,
        strip: p.strip,
        snap: p.snap,
        tooltip: p.tooltip,
        markTooltip: p.markTooltip,
        badgesAtHour: p.badgesAtHour,
        onMarkHover: p.onMarkHover,
        onContextMenu: p.onContextMenu,
        revision: p.revision,
        pick: picks.drawn({ frame, until }, p.onPick),
      });
      const baseKey = [size.width, size.height, size.scaleX, size.scaleY, frame.plotLeft, frame.plotTop, frame.plotWidth, frame.plotHeight, frame.x[0], frame.x[1], frame.y[0], frame.y[1],
        frame.log, tokens, p.layers, p.revision, until, p.strip, p.playhead, p.playback, p.badges, p.overlay, p.rows, p.formatValue, p.valueTicks, p.gaps, p.endLabels,
        ...(sampled ? sampled.headValues : []),
        ...(p.data?.revision() ?? []), ...summaries];
      if (!painted || painted.length !== baseKey.length || !painted.every((value, index) => Object.is(value, baseKey[index]))) {
        const resolvedEnds = p.endLabels ? normalizeEndLabels(p.endLabels(frame, until)) : undefined;
        if (worker) {
          const frameId = worker.nextFrameId();
          const snapshot = makeSnapshot();
          postedFrames.set(frameId, snapshot);
          lastPostedFrameId = frameId;
          const posted = worker.draw({
            frameId,
            backingWidth: size.width,
            backingHeight: size.height,
            layoutWidth,
            layoutHeight,
            scaleX: size.scaleX,
            scaleY: size.scaleY,
            frame,
            layers: p.layers,
            descriptors: sampled!.descriptors,
            headValues: sampled!.headValues,
            gaps: p.gaps,
            playhead: p.playhead,
            playback: p.playback,
            afterPlayhead: p.afterPlayhead,
            until,
            markSummaries: summaries,
            badges: p.badges,
            rowLabels: p.rows?.labels,
            formatValue: typeof p.formatValue === 'string' ? p.formatValue : undefined,
            endLabels: resolvedEnds?.ends,
            endLabelGap: resolvedEnds?.gap,
          });
          if (!posted) {
            postedFrames.delete(frameId);
            scheduler.setAsyncPending(current, false);
            setWorkerFailed(true);
            return false;
          }
          scheduler.setAsyncPending(current, true);
        } else {
          drawTimeChartScene(
            context!,
            frame,
            tokens,
            {
              layoutWidth,
              layoutHeight,
              scaleX: size.scaleX,
              scaleY: size.scaleY,
              layers: p.layers,
              gaps: p.gaps,
              strip: p.strip,
              playhead: p.playhead,
              playback: p.playback,
              afterPlayhead: p.afterPlayhead,
              until,
              markSummaries: summaries,
              markSummariesMatched: true,
              badges: p.badges,
              rowLabels: p.rows?.labels,
              formatValue: p.formatValue,
              overlay: p.overlay,
              endLabels: resolvedEnds?.ends,
              endLabelGap: resolvedEnds?.gap,
            },
            measure,
          );
          const snapshot = makeSnapshot();
          paintedState = snapshot;
          pickFrame.current = snapshot.pick;
          shown.current = { x, until };
        }
        painted = baseKey;
      } else if (!worker) {
        const snapshot = makeSnapshot();
        paintedState = snapshot;
        pickFrame.current = snapshot.pick;
        shown.current = { x, until };
      }
      const visible = paintedState;
      if (overlayContext && overlaid) {
        overlayContext.setTransform(1, 0, 0, 1, 0, 0);
        overlayContext.clearRect(0, 0, size.width, size.height);
        overlaid = false;
      }
      if (overlayContext) {
        const overlayScaleX = visible ? size.width / visible.layoutWidth : size.scaleX;
        const overlayScaleY = visible ? size.height / visible.layoutHeight : size.scaleY;
        overlayContext.setTransform(overlayScaleX, 0, 0, overlayScaleY, 0, 0);
      }
      if (p.onViewChange && (p.onViewChange !== reported?.handler || x[0] !== reported.x[0] || x[1] !== reported.x[1])) {
        reported = { handler: p.onViewChange, x };
        if (telling) telling = reported;
        else {
          telling = reported;
          inFrame(() => { const told = telling; telling = null; if (told) told.handler(told.x); });
        }
      }
      const summaryRequest = { callback: p.onSummaryView, from: Math.floor(frame.x[0]),
        to: Math.floor(Math.min(frame.x[1], until ?? frame.x[1])), end: Math.floor(frame.x[1]), width: Math.ceil(frame.plotWidth) };
      const sameView = (view: SummaryView | null) => !!view && view.callback === summaryRequest.callback && view.from === summaryRequest.from
        && view.to === summaryRequest.to && view.end === summaryRequest.end && view.width === summaryRequest.width;
      if (summaryRequest.to >= summaryRequest.from && !sameView(summaryLatest)) {
        summaryLatest = summaryRequest;
        if (sameView(summaryView)) { if (summaryTimer) clearTimeout(summaryTimer); summaryTimer = null; }
        else if (!summaryView || summaryView.callback !== summaryRequest.callback || summaryView.width !== summaryRequest.width
          || performance.now() - summaryAt >= SUMMARY_INTERVAL_MS) { if (summaryTimer) clearTimeout(summaryTimer); sendSummary(); }
        else summaryTimer ??= setTimeout(sendSummary, SUMMARY_INTERVAL_MS - (performance.now() - summaryAt));
      }
      const drawn = p.badges ? visibleBadges(p.badges, frame.x, until) : [];
      if (drawn.length !== spokenFor.length || drawn.some((badge, index) => badge !== spokenFor[index])) {
        spokenFor = drawn;
        spokenBadges = drawn.map(badge => `${dayLabel(badge.hour)}, ${badge.label}`).join('; ');
      }
      const marked = summaries.map(summary => shownClusters(summary, until));
      if (marked.length !== spokenMarks.length || marked.some((clusters, index) => clusters !== spokenMarks[index])) {
        spokenMarks = marked;
        clusterText = marked.flatMap(clusters => clusters.map(cluster => cluster.badge ? `${dayLabel(cluster.badge.hour)}, ${cluster.badge.label}`
          : `${dayLabel(cluster.firstHour) === dayLabel(cluster.lastHour) ? dayLabel(cluster.firstHour) : `${dayLabel(cluster.firstHour)} to ${dayLabel(cluster.lastHour)}`}, ${eventCount(cluster.count)}`)).join('; ');
      }
      const eventText = spokenBadges && clusterText ? `${spokenBadges}; ${clusterText}` : spokenBadges || clusterText;
      if (eventText !== spokenText && eventsText.current) { spokenText = eventText; eventsText.current.textContent = eventText ? `Events: ${eventText}.` : ''; }
      if (drag?.mode === 'box' && overlayContext) { drawSelection(overlayContext, tokens, drag); overlaid = true; }

      const plot = visible?.frame ?? null;
      const visibleUntil = visible ? visible.until : null;
      const zone = pointer && plot ? hitZone(pointer, plot) : null;
      const hit = visible ? hoverAt(hover.current, visible.frame.x, visible.until, point => !drag || drag.mode !== 'box' ? pointedAt(point, visible) : null) : null;
      const pointed = hit?.by === 'pointer' ? hit as unknown as Pointed : null;
      if ((pointed?.cluster ?? null) !== markHovered) { markHovered = pointed?.cluster ?? null; (visible?.onMarkHover ?? p.onMarkHover)?.(markHovered); }
      const stop = keyedStopRef.current;
      const keyed = hit?.by === 'keyboard' ? (stop && stop.at === hit.hour && stop.describes <= (visibleUntil ?? Infinity) ? stop.describes : hit.hour) : null;
      const eventHour = plot && keyed != null && keyed >= plot.x[0] && keyed <= Math.min(plot.x[1], visibleUntil ?? plot.x[1]) ? keyed : null;
      if (eventHour != null && visible && (!keyedBadges || keyedBadges.source !== visible.badges || keyedBadges.hour !== eventHour)) {
        const source = visible.badges;
        keyedBadges = { source, hour: eventHour, badges: source ? intervalIndexOf(source, () => source.map(badge => ({
          firstHour: badge.hour, lastHour: badge.hour, badge,
        }))).at(eventHour).map(entry => entry.badge) : [] };
      }
      const at = hit ? { hour: hit.hour, describes: pointed?.describes ?? keyed ?? hit.hour,
        badges: pointed?.badges ?? (eventHour != null ? keyedBadges!.badges : []) } : null;
      shownHour.current = at ? Math.floor(at.describes) : null;
      if (p.active && p.onHover) {
        const previous = hoverObserver.current;
        const moved = !!previous && (!Object.is(previous.pickOwner, p.pickOwner) || previous.branchStart !== p.branchStart
          || previous.first !== p.extent[0] || previous.last !== p.extent[1]);
        if (moved) { const callback = previous!.callback; inFrame(() => callback(null)); }
        const told = !previous || moved ? undefined : previous.hour, hour = shownHour.current;
        let tell = hour !== told;
        if (!tell && pendingHover) { clearTimeout(pendingHover.timer); pendingHover = null; }
        if (tell && hour !== null && told !== undefined && hit?.by === 'pointer') {
          if (pendingHover?.hour !== hour) {
            if (pendingHover) clearTimeout(pendingHover.timer);
            pendingHover = { hour, since: info.now, timer: setTimeout(() => wake(), HOVER_REST_MS) };
          }
          tell = info.now - pendingHover.since >= HOVER_REST_MS;
        }
        if (tell && pendingHover) { clearTimeout(pendingHover.timer); pendingHover = null; }
        hoverObserver.current = { callback: p.onHover, pickOwner: p.pickOwner,
          branchStart: p.branchStart, first: p.extent[0], last: p.extent[1], hour: tell ? hour : told ?? null };
        if (tell) { const callback = p.onHover; inFrame(() => callback(hour)); }
      }
      if (at && plot && visible) {
        if (visible.rows && hit?.by === 'pointer' && pointer) rowRef.current = Math.max(0, Math.min(visible.rows.labels.length - 1,
          Math.floor((pointer.y - plot.plotTop) / plot.plotHeight * visible.rows.labels.length)));
        const row = Math.max(0, Math.min((visible.rows?.labels.length ?? 1) - 1, rowRef.current));
        if (overlayContext) { drawCrosshair(overlayContext, plot, tokens, at.hour); overlaid = true; }
        const hour = Math.floor(at.describes);
        const lost = lostAt(visible.gaps ?? [], hour);
        if (!hovered || hovered.hour !== hour || hovered.row !== row || hovered.rows !== visible.rows || hovered.tooltip !== visible.tooltip || hovered.revision !== visible.revision || hovered.cluster !== pointed?.cluster || hovered.markTooltip !== visible.markTooltip || !sameBadges(hovered.badges, at.badges)) {
          hovered = { hour, row, badges: at.badges, cluster: pointed?.cluster, markTooltip: visible.markTooltip, rows: visible.rows, tooltip: visible.tooltip, revision: visible.revision,
            content: pointed?.cluster ? visible.markTooltip?.(pointed.cluster) ?? null : lost ? lostTooltip(lost, dayLabel)
              : visible.rows ? visible.rows.tooltip(row, hour) : visible.tooltip?.(hour, plot, visible.badgesAtHour?.(hour) ?? at.badges) ?? null,
            key: `${hour}:${at.badges.map(badge => `${badge.hour}|${badge.label}`).join(',')}` };
        }
        let point: { clientX: number; clientY: number } | null = pointed ? pointer : null;
        if (!point) {
          const rect = element.getBoundingClientRect();
          point = { clientX: rect.left + plot.toX(at.hour) * rect.width / visible.layoutWidth, clientY: rect.top + (plot.plotTop + 8) * rect.height / visible.layoutHeight };
        }
        const held = tooltipStore.get(), intent = keyIntent.current;
        const fresh = intent === true || (intent !== false && tooltipStore.revision() === intent.since);
        if (pointed || fresh || held == null || held.owner === owner) {
          if (hovered.content) showTooltip(owner, hovered.content, point.clientX, point.clientY, hovered.key); else hideTooltip(owner);
          const text = hit?.by === 'keyboard' && hovered.content ? tooltipText(hovered.content) : keyText.current;
          if (text !== keyText.current) { keyText.current = text; setKeySpoken(text); }
        }
      } else { hovered = null; hideTooltip(owner); }
      keyIntent.current = false;
      setStyle(element, 'cursor', drag?.mode === 'pan' ? 'grabbing' : pointed?.badges.length ? 'pointer' : zone === 'x-axis' ? 'ew-resize'
        : zone === 'y-axis' ? 'ns-resize' : zone === 'plot' || zone === 'strip' ? 'crosshair' : 'default');

      const settled = Math.abs(x[0] - targetX[0]) + Math.abs(x[1] - targetX[1]) < 1e-3
        && Math.abs(y[0] - targetY.range[0]) + Math.abs(y[1] - targetY.range[1]) < 1e-5 * ySpan;
      return !settled || drag != null;
    };
    const pointedAt = (point: { x: number; y: number }, snapshot: PaintedSnapshot): Pointed | null => {
      const plot = snapshot.frame;
      const until = snapshot.until;
      let cluster: MarkCluster | null = null;
      for (const summary of snapshot.markSummaries) {
        const next = markClusterAt(point, plot, summary, until);
        if (next && (!cluster || Math.abs(plot.toX(clusterHour(next)) - point.x) < Math.abs(plot.toX(clusterHour(cluster)) - point.x))) cluster = next;
      }
      const badges = snapshot.badges ? badgesAt(point, plot, snapshot.badges, until) : [];
      if (cluster && (!badges.length || Math.abs(plot.toX(clusterHour(cluster)) - point.x) <= Math.abs(plot.toX(badges[0].hour) - point.x)))
        return { hour: clusterHour(cluster), describes: cluster.firstHour, badges: cluster.badge ? [cluster.badge] : [], cluster };
      if (badges.length) return { hour: badges[0].hour, describes: badges[0].hour, badges };
      const zone = hitZone(point, plot);
      if (zone !== 'plot' && zone !== 'strip') return null;
      const target = pointerTarget(zone, pointerHour(plot.hourAt(point.x), plot.x, until), plot.x, until, snapshot.extent,
        { snap: snapshot.snap && (hour => snapshot.snap!(hour, plot)), describe: snapshot.strip?.describe && (hour => snapshot.strip!.describe!(point, plot, until) ?? hour) });
      return target ? { hour: target.at, describes: target.describes, badges: [] } : null;
    };
    const sameBadges = (a: readonly EventBadge[], b: readonly EventBadge[]) => a.length === b.length && a.every((badge, index) => badge === b[index]);
    const current: FrameTask = { frame: draw };
    task.current = current;
    const wake = () => scheduler.wake(current);
    wakeRef = wake;
    const remove = scheduler.add(current, element);
    const unsubscribe = subscribeTokens(next => { tokens = next; wake(); });

    const local = (event: PointerEvent | WheelEvent | MouseEvent, snapshot = activeSnapshot()) => {
      const width = snapshot?.layoutWidth ?? element.clientWidth;
      const height = snapshot?.layoutHeight ?? element.clientHeight;
      const point = toLocal(event.clientX, event.clientY, element.getBoundingClientRect(), width, height);
      return { ...point, clientX: event.clientX, clientY: event.clientY };
    };
    let secondPress = false;
    const down = (event: PointerEvent) => {
      pendingNavigation.current = null;
      hover.current = reduceHover(hover.current, { type: 'pointer-down' });
      secondPress = event.detail >= 2;
      const snapshot = activeSnapshot();
      if (!snapshot || (event.button !== 0 && event.button !== 1)) return;
      const currentFrame = snapshot.frame;
      const point = local(event, snapshot);
      const zone = hitZone(point, currentFrame);
      const badges = event.button === 0 && snapshot.badges ? badgesAt(point, currentFrame, snapshot.badges, snapshot.until) : [];
      if (badges.length && snapshot.pick.onPick) {
        if (!secondPress) pick(snapshot.pick, badges[0].hour);
        return;
      }
      if (!zone) return;
      if (event.button === 0 && zone === 'x-axis') drag = { mode: 'axis-x', grab: currentFrame.hourAt(point.x) };
      else if (event.button === 0 && zone === 'y-axis' && !snapshot.rows) drag = { mode: 'axis-y', grab: currentFrame.valueAt(point.y) };
      else if ((zone === 'plot' || zone === 'strip') && (event.button === 1 || event.shiftKey)) drag = { mode: 'pan', lastX: point.x };
      else if (zone === 'plot' || zone === 'strip') drag = { mode: 'box', x0: point.x, y0: point.y, x1: point.x, y1: point.y, timeOnly: zone === 'strip' || !!snapshot.rows };
      else return;
      event.preventDefault();
      element.focus({ preventScroll: true });
      element.setPointerCapture(event.pointerId);
      wake();
    };
    const move = (event: PointerEvent) => {
      const snapshot = activeSnapshot();
      pointer = local(event, snapshot);
      hover.current = reduceHover(hover.current, { type: 'pointer', point: pointer });
      const currentFrame = snapshot?.frame ?? null;
      if (drag && currentFrame) {
        if (drag.mode === 'axis-x') dispatch({ type: 'axis-x', grab: drag.grab, fraction: axisFraction(pointer, currentFrame, 'x') });
        else if (drag.mode === 'axis-y') dispatch({ type: 'axis-y', grab: drag.grab, fraction: axisFraction(pointer, currentFrame, 'y') });
        else if (drag.mode === 'pan') {
          dispatch({ type: 'pan', hours: (drag.lastX - pointer.x) / currentFrame.plotWidth * (currentFrame.x[1] - currentFrame.x[0]) });
          drag.lastX = pointer.x;
        } else { drag.x1 = pointer.x; drag.y1 = pointer.y; }
      }
      wake();
    };
    const pick = (drawn: ReturnType<ChartPicks['drawn']>, hour: number, point?: { x: number; y: number }) => {
      picks.pick(point ? { ...drawn, context: { ...drawn.context, point } } : drawn, hour);
    };
    const up = () => {
      const finished = drag;
      drag = null;
      const snapshot = activeSnapshot();
      const currentFrame = snapshot?.frame ?? null;
      if (finished?.mode === 'box' && currentFrame && Math.abs(finished.x1 - finished.x0) > BOX_THRESHOLD) {
        dispatch({ type: 'box', x: [currentFrame.hourAt(finished.x0), currentFrame.hourAt(finished.x1)],
          y: !finished.timeOnly && Math.abs(finished.y1 - finished.y0) > BOX_THRESHOLD ? [currentFrame.valueAt(finished.y1), currentFrame.valueAt(finished.y0)] : undefined });
      } else if (finished?.mode === 'box' && snapshot && Math.abs(finished.y1 - finished.y0) <= BOX_THRESHOLD && snapshot.pick.onPick) {
        const pointed = pointedAt({ x: finished.x0, y: finished.y0 }, snapshot);
        if (pointed && !secondPress) pick(snapshot.pick, pointed.describes, !finished.timeOnly ? { x: finished.x0, y: finished.y0 } : undefined);
      }
      wake();
    };
    const cancel = () => { drag = null; wake(); };
    const leave = () => {
      pendingNavigation.current = null; pointer = null;
      if (pendingHover) { clearTimeout(pendingHover.timer); pendingHover = null; }
      hover.current = reduceHover(hover.current, { type: 'pointer-leave' }); hideTooltip(owner); wake();
    };
    const wheel = (event: WheelEvent) => {
      const snapshot = activeSnapshot();
      if (!snapshot) return;
      const currentFrame = snapshot.frame;
      const point = local(event, snapshot);
      const steps = wheelSteps(event.deltaY, event.deltaMode);
      const zone = hitZone(point, currentFrame);
      if ((zone !== 'plot' && zone !== 'strip') || !steps) return;
      event.preventDefault();
      dispatch({ type: 'wheel', at: currentFrame.hourAt(point.x), steps });
    };
    const doubleClick = (event: MouseEvent) => {
      const snapshot = activeSnapshot();
      if (!snapshot) return;
      const currentFrame = snapshot.frame;
      const zone = hitZone(local(event, snapshot), currentFrame);
      if (snapshot.pick.onPick && (zone === 'plot' || zone === 'strip')) return;
      dispatch({ type: zone === 'x-axis' ? 'reset-x' : zone === 'y-axis' ? 'reset-y' : 'reset' });
    };
    const menu = (event: MouseEvent) => {
      const fromPointer = pointerMenu(hover.current);
      hover.current = reduceHover(hover.current, { type: 'contextmenu' });
      if (!fromPointer) { event.preventDefault(); return; }
      const snapshot = activeSnapshot();
      const handler = snapshot?.onContextMenu;
      if (!handler || !snapshot) return;
      const pointed = pointedAt(local(event, snapshot), snapshot);
      if (!pointed) return;
      event.preventDefault();
      handler(Math.floor(pointed.describes), event.clientX, event.clientY);
    };
    element.addEventListener('pointerdown', down);
    element.addEventListener('pointermove', move);
    element.addEventListener('pointerup', up);
    element.addEventListener('pointercancel', cancel);
    element.addEventListener('pointerleave', leave);
    element.addEventListener('wheel', wheel, { passive: false });
    element.addEventListener('dblclick', doubleClick);
    element.addEventListener('contextmenu', menu);
    return () => {
      if (summaryTimer) clearTimeout(summaryTimer);
      if (pendingHover) clearTimeout(pendingHover.timer);
      if (overlayContext && sheet) {
        overlayContext.setTransform(1, 0, 0, 1, 0, 0);
        overlayContext.clearRect(0, 0, sheet.canvas.width, sheet.canvas.height);
      }
      remove(); unsubscribe(); surface.disconnect(); worker?.detach(); hideTooltip(owner);
      element.removeEventListener('pointerdown', down);
      element.removeEventListener('pointermove', move);
      element.removeEventListener('pointerup', up);
      element.removeEventListener('pointercancel', cancel);
      element.removeEventListener('pointerleave', leave);
      element.removeEventListener('wheel', wheel);
      element.removeEventListener('dblclick', doubleClick);
      element.removeEventListener('contextmenu', menu);
      pickFrame.current = null;
      task.current = null;
      controls.current = null;
    };
  }, [owner, picks, useWorker]);

  useEffect(() => {
    const { pickOwner, branchStart } = props;
    const [first, last] = props.extent;
    const clear = () => {
      const observed = hoverObserver.current;
      if (observed && (!Object.is(observed.pickOwner, pickOwner) || observed.branchStart !== branchStart
        || observed.first !== first || observed.last !== last)) return;
      shownHour.current = null;
      hoverObserver.current = undefined;
      (observed?.callback ?? latest.current.onHover)?.(null);
    };
    if (!props.active) clear();
    return clear;
  }, [props.active, props.pickOwner, props.branchStart, props.extent[0], props.extent[1], props.onHover !== undefined]);

  useEffect(() => {
    if (task.current) scheduler.setVisible(task.current, props.active);
    if (!props.active) hideTooltip(owner);
  }, [props.active, owner, useWorker]);
  useEffect(() => { if (task.current) scheduler.wake(task.current); });
  useEffect(() => props.liveHead?.subscribe(() => { if (task.current) scheduler.wake(task.current); }), [props.liveHead]);
  useEffect(() => props.data?.subscribe(() => { if (task.current) scheduler.wake(task.current); }), [props.data]);
  useEffect(() => {
    if (!props.active) props.onMarkHover?.(null);
    return () => props.onMarkHover?.(null);
  }, [props.active, props.pickOwner, props.onMarkHover]);
  useEffect(() => {
    if (props.keyboardHour !== undefined) {
      const pending = pendingNavigation.current;
      if (props.onNavigateHour && (!pending || pending.hover !== hover.current || !Object.is(pending.owner, props.pickOwner))) return;
      pendingNavigation.current = null;
      keyedStopRef.current = null;
      keyIntent.current = pending ? { since: pending.tooltip } : true;
      hover.current = reduceHover(hover.current, { type: 'key', hour: props.keyboardHour });
      if (task.current) scheduler.wake(task.current);
    }
  }, [props.keyboardHour, props.keyboardRevision, props.pickOwner]);

  const wake = () => { if (task.current) scheduler.wake(task.current); };
  const send = (event: HoverEvent<Point>) => { hover.current = reduceHover(hover.current, event); wake(); };
  const clearKeyed = () => { keyedStopRef.current = null; send({ type: 'key-clear' }); keyText.current = ''; setKeySpoken(''); };
  const keyDown = (event: ReactKeyboardEvent) => {
    if ((event.key === 'ArrowUp' || event.key === 'ArrowDown') && props.rows && shown.current) {
      event.preventDefault();
      keyIntent.current = true;
      rowRef.current = keyTarget(event.key, rowRef.current, props.rows.labels.length, 1) ?? 0;
      send({ type: 'key', hour: hover.current.keyed ?? shownHour.current ?? props.playhead ?? props.extent[1] });
      return;
    }
    if ((event.key === 'ArrowLeft' || event.key === 'ArrowRight' || event.key === 'Home' || event.key === 'End') && shown.current && (props.tooltip || props.rows)) {
      event.preventDefault();
      keyIntent.current = true;
      const { x, until } = shown.current;
      if (props.onNavigateHour && Math.min(x[1], until ?? x[1]) < x[0]) {
        pendingNavigation.current = null;
        keyedStopRef.current = null;
        send({ type: 'key', hour: null });
        return;
      }
      if (props.onNavigateHour) {
        const since = tooltipStore.revision();
        pendingNavigation.current = { hover: hover.current, owner: props.pickOwner, tooltip: since };
        keyIntent.current = { since };
        props.onNavigateHour(event.key, hover.current.keyed ?? shownHour.current, x, until);
        return;
      }
      const strip = props.strip;
      const stops = keyStopsFor(props.keyHours, strip, () => strip?.keyTargets?.(x, until) ?? [], x, until);
      const from = hover.current.keyed ?? shownHour.current;
      if (stops) {
        const own = keyedStopRef.current;
        const current = own && hover.current.keyed === own.at ? own : from == null ? null : { at: from };
        keyedStopRef.current = keyedStop(event.key, current, stops);
        send({ type: 'key', hour: keyedStopRef.current?.at ?? null });
      } else {
        keyedStopRef.current = null;
        send({ type: 'key', hour: keyedHour(event.key, from, x, until) });
      }
      return;
    }
    if (event.key === 'Escape') pendingNavigation.current = null;
    if (event.key === 'Escape' && hover.current.keyed != null) { event.preventDefault(); clearKeyed(); return; }
    if (event.key === 'Escape' && zoomed) { event.preventDefault(); controls.current?.dispatch({ type: 'reset' }); }
    const moment = actionHour(shownHour.current, hover.current, props.playhead);
    if ((event.key === 'Enter' || event.key === ' ') && props.onPick) {
      pendingNavigation.current = null;
      event.preventDefault();
      if (pickFrame.current) picks.pick(pickFrame.current, moment ?? props.extent[1]);
    }
    if ((event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) && props.onContextMenu && moment != null) {
      pendingNavigation.current = null;
      event.preventDefault();
      send({ type: 'menu-key' });
      const bounds = canvas.current!.getBoundingClientRect();
      props.onContextMenu(Math.floor(moment), bounds.left + bounds.width / 2, bounds.top + bounds.height / 2);
    }
  };
  return <div className="time-chart" data-detail={props.badges?.flatMap(badge => badge.failure ? [badge.failure.message] : []).join("\n") || undefined}>
    <canvas key={useWorker ? 'worker' : 'sync'} ref={canvas} tabIndex={0} role="img" aria-label={props.label} aria-describedby={`${owner}-events`}
      aria-keyshortcuts={props.rows ? 'ArrowUp ArrowDown ArrowLeft ArrowRight Home End' : props.tooltip ? 'ArrowLeft ArrowRight Home End' : undefined} onKeyDown={keyDown} onBlur={clearKeyed} />
    <p ref={eventsText} id={`${owner}-events`} className="sr-only" />
    <p className="sr-only" aria-live="polite">{keySpoken}</p>
    <canvas ref={overlay} className="time-chart-overlay" aria-hidden="true" />
    {zoomed && <ZoomReset onReset={() => controls.current?.dispatch({ type: 'reset' })} />}
    {props.notRecorded && <div className="time-chart-state"><WidgetState kind="unavailable" message="Not recorded" detail={props.notRecorded} /></div>}
  </div>;
}
