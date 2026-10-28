import { useEffect, useId, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent } from 'react';
import { ZoomReset } from './ZoomReset';
import { BADGE_STRIP, badgesAt, drawBadges, visibleBadges } from './badges';
import type { EventBadge } from './badges';
import { clusterHour, drawMarkSummary, eventCount, markClusterAt, markSummaryMatches } from './markSummary';
import type { MarkCluster, MarkSummary } from './markSummary';
import { clipToPlot, dayLabel, drawAxes, drawCrosshair, drawPlayhead, drawSelection, VALUE_LABEL_GAP, valueLabelWidth } from './draw';
import { approach, easeTau } from './easing';
import { keyTarget } from './rowNavigation';
import { axisFraction, hitZone, keyedHour, plotFrame, pointerHour, STRIP_GAP, wheelSteps } from './frame';
import { actionHour, hoverAt, keyedStop, keyStopMemo, NO_HOVER, pointerMenu, pointerTarget, reduceHover } from './hover';
import type { HoverEvent, HoverState, KeyStop } from './hover';
import { intervalIndexOf } from './intervalIndex';
import type { ChartStrip } from './strip';
import type { Padding, PlotFrame } from './frame';
import { ChartPicks } from './pick';
import type { ChartPick } from './pick';
import { scheduler } from './scheduler';
import type { FrameInfo, FrameTask } from './scheduler';
import { toLocal, watchCanvasSize } from './surface';
import { CHART_LAYOUT, readTokens, subscribeTokens } from './tokens';
import type { ChartTokens } from './tokens';
import { hideTooltip, showTooltip, tooltipText } from './tooltip';
import type { TooltipContent } from './tooltip';
import { formatAxisValue } from './valueAxis';
import { EMPTY_VIEW, playbackDomain, reduceView, resolveX, resolveY } from './view';
import type { AxisSettings, PlaybackMode, Range, ViewAction, ViewState } from './view';
import { shownUntil } from './view';
import './charts.css';

/** One thing a chart draws inside its plot, such as a line per country. */
export interface ChartLayer {
  /** Draws inside the plot; the canvas is clipped to it. */
  /** `state.playhead` is the last hour to draw (null: the whole range). */
  draw(ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens, state: { playhead: number | null }): void;
  /** The values this layer shows inside a time range, for auto-fitting the value axis. */
  extent?(x: Range): Range | null;
}

export type { ChartStrip } from './strip';

export type { ChartPick } from './pick';

export interface TimeChartProps {
  /** A categorical row plane, using the same time gestures and hover as lines.
   * Up/down steps rows; left/right still steps keyHours. Dates sit above it. */
  rows?: { labels: readonly string[]; labelWidth: number; tooltip: (row: number, hour: number) => TooltipContent | null };
  /** Names the chart for screen readers. */
  label: string;
  /** Every hour the data can cover: the branch, or the widget's range. */
  extent: Range;
  /** The playback hour while following; null shows the whole extent. */
  playhead: number | null;
  playback: PlaybackMode;
  /** In whole-campaign (cursor) playback, draws, marks and hovers past the playhead too. */
  afterPlayhead?: boolean;
  windowHours?: number;
  hoursPerSecond?: number;
  axes: AxisSettings;
  /** Fits the value axis to the values drawn in view (null: none), both bounds,
   * where the axis settings leave it open; without it an open axis starts at zero. */
  fitValues?: (shown: Range | null) => Range | null;
  /** Axis settings store offsets from this hour. */
  branchStart: number;
  /** Room around the plot. Without a `left`, the left side grows to fit the widest value label. */
  padding?: Partial<Padding>;
  layers: readonly ChartLayer[];
  /** A strip under the plot; hovering it hovers the hour above. */
  strip?: ChartStrip;
  /** A value-axis label; `ticks` is every tick drawn with it, for a precision that keeps
   * them apart. Without it, `formatAxisValue` writes plain amounts. */
  formatValue?: (value: number, ticks: readonly number[]) => string;
  /** Fixed semantic value ticks instead of automatically spaced ticks. */
  valueTicks?: readonly number[];
  /** Event glyphs along the top edge. Hovering one snaps to its hour. */
  badges?: readonly EventBadge[];
  /** Prepared screen-width marks. Individual events are fetched by stable range,
   * so drawing and spoken text never materialise campaign-long event rows. */
  markSummary?: MarkSummary;
  markSummaries?: readonly MarkSummary[];
  markTooltip?: (cluster: MarkCluster) => TooltipContent | null;
  onMarkHover?: (cluster: MarkCluster | null) => void;
  /** Reached exact event rows only, for pointer/keyboard hour tooltips. */
  badgesAtHour?: (hour: number) => readonly EventBadge[];
  /** Requests display summaries at actual layout width and playback cutoff. */
  onSummaryView?: (view: { from: number; to: number; width: number }) => void;
  /** Prepared navigation queries the next exact stop without retaining all dates. */
  onNavigateHour?: (key: 'ArrowLeft' | 'ArrowRight' | 'Home' | 'End', from: number | null, x: Range, until: number | null) => void;
  /** The result of the latest owner-scoped navigation request. */
  keyboardHour?: number | null;
  keyboardRevision?: unknown;
  /** Tooltip content for a pointer or keyboard hour, including its visible event
   * badges; null shows none. */
  tooltip?: (hour: number, frame: PlotFrame, badges: readonly EventBadge[]) => TooltipContent | null;
  /** Requests dated facts at the resolved pointer or keyboard hour; null clears them. */
  onHover?: (hour: number | null) => void;
  /** Moves an hour hovered over the plot onto something nearby, such as an event tick
   * (never over a strip, which has its own `describe`). */
  snap?: (hour: number, frame: PlotFrame) => number;
  /** Hours the arrow keys step the keyboard hover through, ascending (such as every
   * hour a value changes); without them the keys step a day. Pass them all, not only
   * those in view: a value going on into the view from an earlier one is reachable at
   * the view's start (`keyStops`). */
  keyHours?: readonly number[];
  /** Drawn after the plot, unclipped: end labels and markers. */
  overlay?: (ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens) => void;
  /** The drawn time range, including zoom/pan, for labels dated to visible ends; called when it changes. */
  onViewChange?: (x: Range) => void;
  onContextMenu?: (hour: number, clientX: number, clientY: number) => void;
  /** A click (not a drag) on the plot picks that hour, e.g. to open or move a breakdown. */
  onPick?: (hour: number, context?: ChartPick) => void;
  /** Stable source/selection/numeric-model token. Change it when a pending pick
   * would refer to different actors/data; exclude playback and label updates. */
  pickOwner?: unknown;
  /** Off-screen charts stop drawing. */
  active: boolean;
  /** Changes whenever the data changes, so the chart redraws. */
  revision: unknown;
}

const DEFAULT_PADDING: Padding = { left: 44, right: CHART_LAYOUT.right, top: 12, bottom: 24 };
/** Room kept between the canvas edge and the widest value label. */
const LABEL_MARGIN = 4;
/** The narrowest time span a zoom may reach: two weeks. */
const MIN_SPAN = 14 * 24;
/** The least time between summary requests while the view keeps moving. */
const SUMMARY_INTERVAL_MS = 150;
/** How long a pointer rests on an hour before the widget hears it. */
const HOVER_REST_MS = 80;
const BOX_THRESHOLD = 8;

/** What a pointer is over: the crosshair's hour, the hour it is about and any badges. */
type Pointed = { hour: number; describes: number; badges: readonly EventBadge[]; cluster?: MarkCluster };

/** A box started in a strip zooms time only: the strip has no value axis. */
type Drag = { mode: 'axis-x'; grab: number } | { mode: 'axis-y'; grab: number } | { mode: 'pan'; lastX: number } | { mode: 'box'; x0: number; y0: number; x1: number; y1: number; timeOnly: boolean };

/** A canvas time chart: axes from the recording's own dates, zoom that holds during
 * playback, live hover, and redraws only while something moves. */
export default function TimeChart(props: TimeChartProps) {
  const owner = useId();
  const canvas = useRef<HTMLCanvasElement>(null);
  const latest = useRef(props);
  latest.current = props;
  const [zoomed, setZoomed] = useState(false);
  // The event badges drawn right now, as text for assistive technology: they follow
  // the view (window, zoom, playhead), and change only when the drawn set does.
  const [spoken, setSpoken] = useState('');
  // Who drives the hover (the pointer, or the keyboard's hour) and the hour it shows,
  // which the keys act on; what the keyboard hover shows is read out as it changes.
  type Point = { x: number; y: number; clientX: number; clientY: number };
  const hover = useRef<HoverState<Point>>(NO_HOVER);
  const pendingNavigation = useRef<{ hover: HoverState<Point>; owner: unknown } | null>(null);
  const shownHour = useRef<number | null>(null);
  // The last notified callback owns any outstanding hover facts, even after a render changes the owner.
  const hoverObserver = useRef<{ callback: NonNullable<TimeChartProps['onHover']>;
    pickOwner: unknown; branchStart: number; first: number; last: number; hour: number | null } | undefined>(undefined);
  const rowRef = useRef(0);
  // The keyboard's stop, beside the hover state's hour: two stops can share an hour.
  const keyedStopRef = useRef<KeyStop | null>(null);
  // The keyboard's stops, built again only when the reads, the strip or the view change.
  const keyStopsFor = useRef(keyStopMemo()).current;
  const [keySpoken, setKeySpoken] = useState('');
  // The text last read out, so an unchanged tooltip is not announced again.
  const keyText = useRef('');
  // The time range drawn and the last hour shown, for the keyboard hover.
  const shown = useRef<{ x: Range; until: number | null } | null>(null);
  const picks = useRef(new ChartPicks()).current;
  picks.update({ owner: props.pickOwner, extent: props.extent, branchStart: props.branchStart, active: props.active, onPick: props.onPick });
  const pickFrame = useRef<(ReturnType<ChartPicks['drawn']> & { props: TimeChartProps }) | null>(null);
  const task = useRef<FrameTask | null>(null);
  const controls = useRef<{ dispatch: (action: ViewAction) => void } | null>(null);

  useEffect(() => {
    const element = canvas.current!;
    const context = element.getContext('2d')!;
    let tokens = readTokens();
    let view: ViewState = EMPTY_VIEW;
    // The badges last described, compared one by one (a count and range could repeat after a pan).
    let spokenFor: readonly EventBadge[] = [];
    let spokeSummary = false;
    // The view last reported: a handler hears only a new range (or, when it changes, the current one).
    let reported: { handler: (x: Range) => void; x: Range } | null = null;
    let eased: { x: [number, number]; y: [number, number] } | null = null;
    let frame: PlotFrame | null = null;
    // While the pointer is over the chart, its drawing without the crosshair is kept, so a
    // pointer move repaints only the crosshair, not every series. Dropped when it leaves.
    let base: { canvas: HTMLCanvasElement; key: readonly unknown[] } | null = null;
    const surface = watchCanvasSize(element, () => wake());
    let pointer: { x: number; y: number; clientX: number; clientY: number } | null = null;
    let drag: Drag | null = null;
    // The extent the zoom belongs to: another branch or range starts unzoomed.
    let zoomedFor = '';
    // Tooltip content for the hovered hour, rebuilt only when the hour, the hovered
    // badges or the data change.
    let hovered: { hour: number; row: number; badges: readonly EventBadge[]; cluster?: MarkCluster; markTooltip: TimeChartProps['markTooltip']; rows: TimeChartProps['rows']; tooltip: TimeChartProps['tooltip']; revision: unknown; content: TooltipContent | null; key: string } | null = null;
    let keyedBadges: { source: TimeChartProps['badges']; hour: number; badges: readonly EventBadge[] } | null = null;
    // The cluster `onMarkHover` last heard about, and a pointer hour waiting to rest.
    let markHovered: MarkCluster | null | undefined;
    let pendingHover: { hour: number; since: number; timer: ReturnType<typeof setTimeout> } | null = null;
    type SummaryView = { callback: TimeChartProps['onSummaryView']; from: number; to: number; width: number };
    // The view last asked for, when, and a newer one waiting for the interval.
    let summaryView: SummaryView | null = null, summaryLatest: SummaryView | null = null;
    let summaryAt = -Infinity, summaryTimer: ReturnType<typeof setTimeout> | null = null;
    const sendSummary = () => {
      summaryTimer = null;
      if (!summaryLatest) return;
      summaryView = summaryLatest; summaryAt = performance.now();
      summaryView.callback?.({ from: summaryView.from, to: summaryView.to, width: summaryView.width });
    };

    const dispatch = (action: ViewAction) => {
      if (!frame) return;
      view = reduceView(view, action, { x: frame.x, y: frame.y, extent: latest.current.extent, minSpan: MIN_SPAN, log: frame.log });
      setZoomed(view.x != null || view.y != null);
      wake();
    };
    controls.current = { dispatch };

    const draw = (info: FrameInfo): boolean => {
      const p = latest.current;
      const { layoutWidth, layoutHeight, backing: size } = surface.read();
      if (!layoutWidth || !layoutHeight) return false;
      if (element.width !== size.width) element.width = size.width;
      if (element.height !== size.height) element.height = size.height;

      // Badges sit above the plot, so a chart with them keeps room there; a strip
      // sits between the plot and the dates.
      const strip = p.strip ? Math.max(0, Math.round(p.strip.height(layoutHeight))) : 0;
      const hasBadgeSummary = p.markSummary?.placement !== 'ticks' && p.markSummary || p.markSummaries?.some(summary => summary.placement !== 'ticks');
      const padding = { ...DEFAULT_PADDING, ...(p.badges || hasBadgeSummary ? { top: BADGE_STRIP } : {}), ...p.padding };
      if (p.rows) {
        padding.left = Math.min(layoutWidth * 0.45, p.rows.labelWidth);
        padding.top = p.padding?.top ?? tokens.fontSize.small * 2 + 4;
        padding.bottom = p.padding?.bottom ?? 8;
      }
      if (strip) padding.bottom += strip + STRIP_GAP;
      const extentKey = `${p.extent[0]}:${p.extent[1]}:${p.branchStart}`;
      if (extentKey !== zoomedFor) {
        if (zoomedFor && (view.x || view.y)) { view = EMPTY_VIEW; setZoomed(false); }
        if (zoomedFor) eased = null;
        zoomedFor = extentKey;
      }
      const playback = p.playhead == null ? p.extent : playbackDomain(p.playback, p.extent, p.playhead, p.windowHours ?? 365 * 24, MIN_SPAN);
      const targetX = resolveX(view, p.axes, p.branchStart, playback);
      const tau = drag && drag.mode !== 'box' ? 0 : easeTau(tokens.motion, p.hoursPerSecond ?? 24, tokens.reducedMotion);
      // Ease the distance to the target, not the absolute value: engine hours are large
      // enough that a relative snap threshold would jump the last tens of hours.
      const glide = (current: number, target: number) => target + approach(current - target, 0, info.dt, tau);
      const x: [number, number] = eased ? [glide(eased.x[0], targetX[0]), glide(eased.x[1], targetX[1])] : [targetX[0], targetX[1]];
      // Within half a pixel of its target an eased range looks settled: snap to it, so the
      // chart stops redrawing instead of easing invisibly for another second or two.
      const plotWidth = Math.max(1, layoutWidth - padding.left - padding.right);
      const plotHeight = Math.max(1, layoutHeight - padding.top - padding.bottom);
      if ((Math.abs(x[0] - targetX[0]) + Math.abs(x[1] - targetX[1])) * plotWidth < 0.5 * Math.max(1e-9, Math.abs(targetX[1] - targetX[0]))) {
        x[0] = targetX[0]; x[1] = targetX[1];
      }
      let data: [number, number] | null = null;
      for (const layer of p.layers) {
        const extent = layer.extent?.(x);
        if (extent) data = data ? [Math.min(data[0], extent[0]), Math.max(data[1], extent[1])] : [extent[0], extent[1]];
      }
      const fitted = p.fitValues?.(data) ?? null;
      const targetY = resolveY(view, fitted ? { ...p.axes, yMin: p.axes.yMin ?? fitted[0], yMax: p.axes.yMax ?? fitted[1] } : p.axes, data);
      const y: [number, number] = eased ? [glide(eased.y[0], targetY.range[0]), glide(eased.y[1], targetY.range[1])] : [targetY.range[0], targetY.range[1]];
      const ySpan = Math.abs(targetY.range[1] - targetY.range[0]) || Math.max(1, Math.abs(targetY.range[1]));
      // The final scheduled frame must paint the fitted scale exactly.
      if ((Math.abs(y[0] - targetY.range[0]) + Math.abs(y[1] - targetY.range[1])) * plotHeight < 0.5 * ySpan) {
        y[0] = targetY.range[0]; y[1] = targetY.range[1];
      }
      eased = { x, y };
      frame = plotFrame(layoutWidth, layoutHeight, padding, x, y, targetY.log, strip, !!p.rows);
      frame.valueTicks = p.valueTicks;
      // The value labels get the room they need: the left padding grows to the widest
      // one (ticks depend only on the value range and plot height, so they stay the
      // same), unless the widget chose its own left padding.
      if (!p.rows && p.padding?.left == null) {
        const needed = Math.ceil(valueLabelWidth(context, frame, tokens, (p.formatValue ?? formatAxisValue))) + VALUE_LABEL_GAP + LABEL_MARGIN;
        if (needed > padding.left) {
          frame = plotFrame(layoutWidth, layoutHeight, { ...padding, left: needed }, x, y, targetY.log, strip, !!p.rows);
          frame.valueTicks = p.valueTicks;
        }
      }

      const until = shownUntil(p.playhead, p.playback, p.afterPlayhead === true);
      const preparedMarks = p.markSummary !== undefined || p.markSummaries !== undefined;
      const summaries = [...(p.markSummary ? [p.markSummary] : []), ...(p.markSummaries ?? [])].filter(summary => markSummaryMatches(summary, frame!, until));
      const baseKey = [size.width, size.height, frame.plotLeft, frame.plotTop, frame.plotWidth, frame.plotHeight, frame.x[0], frame.x[1], frame.y[0], frame.y[1],
        frame.log, tokens, p.layers, p.revision, until, p.strip, p.playhead, p.playback, p.badges, preparedMarks, p.overlay, p.rows, p.formatValue, p.valueTicks, ...summaries];
      const kept = base;
      pickFrame.current = { ...picks.drawn({ frame, until }, p.onPick), props: p };
      if (kept && kept.key.length === baseKey.length && kept.key.every((value, index) => Object.is(value, baseKey[index]))) {
        context.setTransform(1, 0, 0, 1, 0, 0);
        context.clearRect(0, 0, size.width, size.height);
        context.drawImage(kept.canvas, 0, 0);
        context.setTransform(size.scaleX, 0, 0, size.scaleY, 0, 0);
      } else {
        context.setTransform(size.scaleX, 0, 0, size.scaleY, 0, 0);
        context.clearRect(0, 0, layoutWidth, layoutHeight);
        drawAxes(context, frame, tokens, (p.formatValue ?? formatAxisValue), p.rows?.labels);
        context.save();
        clipToPlot(context, frame);
        for (const layer of p.layers) layer.draw(context, frame, tokens, { playhead: until });
        context.restore();
        if (p.strip && frame.strip) p.strip.draw(context, frame, tokens, { playhead: until });
        if (p.playhead != null && p.playback === 'cursor') drawPlayhead(context, frame, tokens, p.playhead);
        for (const summary of summaries) drawMarkSummary(context, frame, tokens, summary);
        if (!preparedMarks && p.badges) drawBadges(context, frame, tokens, p.badges, until);
        p.overlay?.(context, frame, tokens);
        if (pointer) {
          const canvas = kept?.canvas ?? document.createElement('canvas');
          if (canvas.width !== size.width) canvas.width = size.width;
          if (canvas.height !== size.height) canvas.height = size.height;
          const copy = canvas.getContext('2d');
          if (copy) { copy.clearRect(0, 0, size.width, size.height); copy.drawImage(element, 0, 0); base = { canvas, key: baseKey }; }
        } else base = null;
      }
      shown.current = { x, until };
      if (p.onViewChange && (p.onViewChange !== reported?.handler || x[0] !== reported.x[0] || x[1] !== reported.x[1])) {
        reported = { handler: p.onViewChange, x };
        p.onViewChange(x);
      }
      // Whole hours, and while the view keeps moving (playback, a pan or zoom) at most
      // one request per interval, the newest view sent when it ends: not one per
      // animation frame. The last summary draws in place until the next arrives
      // (`summaryFits`); a settled view always gets its own.
      const summaryRequest = { callback: p.onSummaryView, from: Math.floor(frame.x[0]),
        to: Math.floor(Math.min(frame.x[1], until ?? frame.x[1])), width: Math.ceil(frame.plotWidth) };
      const sameView = (view: SummaryView | null) => !!view && view.callback === summaryRequest.callback && view.from === summaryRequest.from
        && view.to === summaryRequest.to && view.width === summaryRequest.width;
      if (!sameView(summaryLatest)) {
        summaryLatest = summaryRequest;
        if (sameView(summaryView)) { if (summaryTimer) clearTimeout(summaryTimer); summaryTimer = null; }
        else if (!summaryView || summaryView.callback !== summaryRequest.callback || summaryView.width !== summaryRequest.width
          || performance.now() - summaryAt >= SUMMARY_INTERVAL_MS) { if (summaryTimer) clearTimeout(summaryTimer); sendSummary(); }
        else summaryTimer ??= setTimeout(sendSummary, SUMMARY_INTERVAL_MS - (performance.now() - summaryAt));
      }
      const drawn = !preparedMarks && p.badges ? visibleBadges(p.badges, frame.x, until) : [];
      if (!preparedMarks && (spokeSummary || drawn.length !== spokenFor.length || drawn.some((badge, index) => badge !== spokenFor[index]))) {
        spokeSummary = false;
        spokenFor = drawn;
        setSpoken(drawn.map(badge => `${dayLabel(badge.hour)}, ${badge.label}`).join('; '));
      }
      if (preparedMarks) {
        spokeSummary = true;
        const text = summaries.flatMap(summary => summary.clusters.map(cluster => cluster.badge ? `${dayLabel(cluster.badge.hour)}, ${cluster.badge.label}`
          : `${dayLabel(cluster.firstHour) === dayLabel(cluster.lastHour) ? dayLabel(cluster.firstHour) : `${dayLabel(cluster.firstHour)} to ${dayLabel(cluster.lastHour)}`}, ${eventCount(cluster.count)}`)).join('; ');
        setSpoken(previous => previous === text ? previous : text);
      }
      if (drag?.mode === 'box') drawSelection(context, tokens, drag);

      // Live hover: re-resolved every frame, so a still pointer stays current during playback.
      // The pointer drives it until a navigation key hands it to the keyboard, whose hour
      // shows while it is in view and not after the playhead.
      const zone = pointer ? hitZone(pointer, frame) : null;
      const plot = frame;
      const hit = hoverAt(hover.current, frame.x, until, point => !drag || drag.mode !== 'box' ? pointedAt(point, plot, p) : null);
      const pointed = hit?.by === 'pointer' ? hit as Pointed : null;
      // Hover callbacks hear only changes: widgets pass new ones each render, and repeating the
      // same news would render them again, which wakes this chart again, every frame.
      if ((pointed?.cluster ?? null) !== markHovered) { markHovered = pointed?.cluster ?? null; p.onMarkHover?.(markHovered); }
      // The crosshair sits at `hour`; the tooltip and the keys are about `describes`. A
      // keyboard stop at a strip bar the view cuts off is about the bar's read.
      const stop = keyedStopRef.current;
      const keyed = hit?.by === 'keyboard' ? (stop && stop.at === hit.hour && stop.describes <= (until ?? Infinity) ? stop.describes : hit.hour) : null;
      const eventHour = keyed != null && keyed >= frame.x[0] && keyed <= Math.min(frame.x[1], until ?? frame.x[1]) ? keyed : null;
      if (eventHour != null && (!keyedBadges || keyedBadges.source !== p.badges || keyedBadges.hour !== eventHour)) {
        const source = p.badges;
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
        if (moved) previous!.callback(null);
        const told = !previous || moved ? undefined : previous.hour, hour = shownHour.current;
        let tell = hour !== told;
        // Back at the hour already heard: a later move rests afresh.
        if (!tell && pendingHover) { clearTimeout(pendingHover.timer); pendingHover = null; }
        // A pointer sweeping the plot is heard where it rests: widgets read exact values for
        // each hour they hear, which would otherwise rebuild and repaint them every move.
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
        if (tell) p.onHover(hour);
      }
      if (at) {
        if (p.rows && hit?.by === 'pointer' && pointer) rowRef.current = Math.max(0, Math.min(p.rows.labels.length - 1,
          Math.floor((pointer.y - frame.plotTop) / frame.plotHeight * p.rows.labels.length)));
        const row = Math.max(0, Math.min((p.rows?.labels.length ?? 1) - 1, rowRef.current));
        drawCrosshair(context, frame, tokens, at.hour);
        const hour = Math.floor(at.describes);
        if (!hovered || hovered.hour !== hour || hovered.row !== row || hovered.rows !== p.rows || hovered.tooltip !== p.tooltip || hovered.revision !== p.revision || hovered.cluster !== pointed?.cluster || hovered.markTooltip !== p.markTooltip || !sameBadges(hovered.badges, at.badges)) {
          hovered = { hour, row, badges: at.badges, cluster: pointed?.cluster, markTooltip: p.markTooltip, rows: p.rows, tooltip: p.tooltip, revision: p.revision,
            content: pointed?.cluster ? p.markTooltip?.(pointed.cluster) ?? null : p.rows ? p.rows.tooltip(row, hour) : p.tooltip?.(hour, frame, p.badgesAtHour?.(hour) ?? at.badges) ?? null,
            key: `${hour}:${at.badges.map(badge => `${badge.hour}|${badge.label}`).join(',')}` };
        }
        // The keyboard hover's tooltip sits at the top of its crosshair; layout pixels
        // become screen pixels through the widget's Size zoom.
        let point: { clientX: number; clientY: number } | null = pointed ? pointer : null;
        if (!point) {
          const rect = element.getBoundingClientRect();
          point = { clientX: rect.left + frame.toX(at.hour) * rect.width / layoutWidth, clientY: rect.top + (frame.plotTop + 8) * rect.height / layoutHeight };
        }
        if (hovered.content) showTooltip(owner, hovered.content, point.clientX, point.clientY, hovered.key); else hideTooltip(owner);
        const text = hit?.by === 'keyboard' && hovered.content ? tooltipText(hovered.content) : keyText.current;
        if (text !== keyText.current) { keyText.current = text; setKeySpoken(text); }
      } else { hovered = null; hideTooltip(owner); }
      element.style.cursor = drag?.mode === 'pan' ? 'grabbing' : pointed?.badges.length ? 'pointer' : zone === 'x-axis' ? 'ew-resize'
        : zone === 'y-axis' ? 'ns-resize' : zone === 'plot' || zone === 'strip' ? 'crosshair' : 'default';

      const settled = Math.abs(x[0] - targetX[0]) + Math.abs(x[1] - targetX[1]) < 1e-3
        && Math.abs(y[0] - targetY.range[0]) + Math.abs(y[1] - targetY.range[1]) < 1e-5 * ySpan;
      return !settled || drag != null;
    };
    /** What the pointer is over, as hover, right-click and pick all use it: an event
     * badge (at its hour), or an hour of the plot or the strip, kept in view and not
     * after the playhead, with where its crosshair sits (`hour`) and what it is about
     * (`describes`: a plot mark it snaps to, or the strip item's own hour). */
    const pointedAt = (point: { x: number; y: number }, plot: PlotFrame, p: TimeChartProps): Pointed | null => {
      const until = shownUntil(p.playhead, p.playback, p.afterPlayhead === true);
      const summaries = [...(p.markSummary ? [p.markSummary] : []), ...(p.markSummaries ?? [])];
      let cluster: MarkCluster | null = null;
      for (const summary of summaries) {
        if (!markSummaryMatches(summary, plot, until)) continue;
        const next = markClusterAt(point, plot, summary);
        if (next && (!cluster || Math.abs(plot.toX(clusterHour(next)) - point.x) < Math.abs(plot.toX(clusterHour(cluster)) - point.x))) cluster = next;
      }
      if (cluster) return { hour: clusterHour(cluster), describes: cluster.firstHour,
        badges: cluster.badge ? [cluster.badge] : [], cluster };
      const badges = !p.markSummary && !p.markSummaries && p.badges ? badgesAt(point, plot, p.badges, until) : [];
      if (badges.length) return { hour: badges[0].hour, describes: badges[0].hour, badges };
      const zone = hitZone(point, plot);
      if (zone !== 'plot' && zone !== 'strip') return null;
      const target = pointerTarget(zone, pointerHour(plot.hourAt(point.x), plot.x, until), plot.x, until, p.extent,
        { snap: p.snap && (hour => p.snap!(hour, plot)), describe: p.strip?.describe && (hour => p.strip!.describe!(point, plot, until) ?? hour) });
      return target ? { hour: target.at, describes: target.describes, badges: [] } : null;
    };
    const sameBadges = (a: readonly EventBadge[], b: readonly EventBadge[]) => a.length === b.length && a.every((badge, index) => badge === b[index]);
    const current: FrameTask = { frame: draw };
    task.current = current;
    const wake = () => scheduler.wake(current);
    const remove = scheduler.add(current);
    const unsubscribe = subscribeTokens(next => { tokens = next; wake(); });

    const local = (event: PointerEvent | WheelEvent | MouseEvent) => {
      const point = toLocal(event.clientX, event.clientY, element.getBoundingClientRect(), element.clientWidth, element.clientHeight);
      return { ...point, clientX: event.clientX, clientY: event.clientY };
    };
    let secondPress = false;
    const down = (event: PointerEvent) => {
      pendingNavigation.current = null;
      hover.current = reduceHover(hover.current, { type: 'pointer-down' });
      // A click picks at once; the second press of a double-click never picks again.
      secondPress = event.detail >= 2;
      if (!frame || (event.button !== 0 && event.button !== 1)) return;
      const point = local(event);
      const zone = hitZone(point, frame);
      // A click on an event badge picks its hour.
      const drawn = pickFrame.current;
      const badges = event.button === 0 && drawn?.props.badges ? badgesAt(point, drawn.context.frame, drawn.props.badges, drawn.context.until) : [];
      if (badges.length && drawn?.onPick) {
        if (!secondPress) pick(badges[0].hour);
        return;
      }
      if (!zone) return;
      if (event.button === 0 && zone === 'x-axis') drag = { mode: 'axis-x', grab: frame.hourAt(point.x) };
      else if (event.button === 0 && zone === 'y-axis' && !latest.current.rows) drag = { mode: 'axis-y', grab: frame.valueAt(point.y) };
      else if ((zone === 'plot' || zone === 'strip') && (event.button === 1 || event.shiftKey)) drag = { mode: 'pan', lastX: point.x };
      else if (zone === 'plot' || zone === 'strip') drag = { mode: 'box', x0: point.x, y0: point.y, x1: point.x, y1: point.y, timeOnly: zone === 'strip' || !!latest.current.rows };
      else return;
      event.preventDefault();
      // Preventing the default also stops the chart taking focus; take it so Escape resets.
      element.focus({ preventScroll: true });
      element.setPointerCapture(event.pointerId);
      wake();
    };
    const move = (event: PointerEvent) => {
      pointer = local(event);
      // The pointer takes over from the keyboard hover.
      hover.current = reduceHover(hover.current, { type: 'pointer', point: pointer });
      if (drag && frame) {
        if (drag.mode === 'axis-x') dispatch({ type: 'axis-x', grab: drag.grab, fraction: axisFraction(pointer, frame, 'x') });
        else if (drag.mode === 'axis-y') dispatch({ type: 'axis-y', grab: drag.grab, fraction: axisFraction(pointer, frame, 'y') });
        else if (drag.mode === 'pan') {
          dispatch({ type: 'pan', hours: (drag.lastX - pointer.x) / frame.plotWidth * (frame.x[1] - frame.x[0]) });
          drag.lastX = pointer.x;
        } else { drag.x1 = pointer.x; drag.y1 = pointer.y; }
      }
      wake();
    };
    const pick = (hour: number, point?: { x: number; y: number }) => {
      const drawn = pickFrame.current;
      if (drawn) picks.pick(point ? { ...drawn, context: { ...drawn.context, point } } : drawn, hour);
    };
    const up = () => {
      const finished = drag;
      drag = null;
      if (finished?.mode === 'box' && frame && Math.abs(finished.x1 - finished.x0) > BOX_THRESHOLD) {
        dispatch({ type: 'box', x: [frame.hourAt(finished.x0), frame.hourAt(finished.x1)],
          y: !finished.timeOnly && Math.abs(finished.y1 - finished.y0) > BOX_THRESHOLD ? [frame.valueAt(finished.y1), frame.valueAt(finished.y0)] : undefined });
      } else if (finished?.mode === 'box' && frame && Math.abs(finished.y1 - finished.y0) <= BOX_THRESHOLD && latest.current.onPick) {
        const drawn = pickFrame.current;
        const pointed = drawn && pointedAt({ x: finished.x0, y: finished.y0 }, drawn.context.frame, drawn.props);
        if (pointed && !secondPress) pick(pointed.describes, !finished.timeOnly ? { x: finished.x0, y: finished.y0 } : undefined);
      }
      wake();
    };
    const cancel = () => { drag = null; wake(); };
    const leave = () => {
      pendingNavigation.current = null; pointer = null; base = null;
      if (pendingHover) { clearTimeout(pendingHover.timer); pendingHover = null; }
      hover.current = reduceHover(hover.current, { type: 'pointer-leave' }); hideTooltip(owner); wake();
    };
    const wheel = (event: WheelEvent) => {
      if (!frame) return;
      const point = local(event);
      const steps = wheelSteps(event.deltaY, event.deltaMode);
      const zone = hitZone(point, frame);
      if ((zone !== 'plot' && zone !== 'strip') || !steps) return;
      event.preventDefault();
      dispatch({ type: 'wheel', at: frame.hourAt(point.x), steps });
    };
    const doubleClick = (event: MouseEvent) => {
      if (!frame) return;
      const zone = hitZone(local(event), frame);
      // Where a click picks, its first click already did: the axes and Reset zoom reset instead.
      if (latest.current.onPick && (zone === 'plot' || zone === 'strip')) return;
      dispatch({ type: zone === 'x-axis' ? 'reset-x' : zone === 'y-axis' ? 'reset-y' : 'reset' });
    };
    const menu = (event: MouseEvent) => {
      // The menu key already opened the menu at the hour shown; the browser's own event
      // after it (at a place of its choosing) must not open another.
      const fromPointer = pointerMenu(hover.current);
      hover.current = reduceHover(hover.current, { type: 'contextmenu' });
      if (!fromPointer) { event.preventDefault(); return; }
      const handler = latest.current.onContextMenu;
      if (!handler || !frame) return;
      const pointed = pointedAt(local(event), frame, latest.current);
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
      remove(); unsubscribe(); surface.disconnect(); hideTooltip(owner);
      element.removeEventListener('pointerdown', down);
      element.removeEventListener('pointermove', move);
      element.removeEventListener('pointerup', up);
      element.removeEventListener('pointercancel', cancel);
      element.removeEventListener('pointerleave', leave);
      element.removeEventListener('wheel', wheel);
      element.removeEventListener('dblclick', doubleClick);
      element.removeEventListener('contextmenu', menu);
      task.current = null;
      controls.current = null;
    };
  }, [owner, picks]);

  useEffect(() => {
    const { pickOwner, branchStart } = props;
    const [first, last] = props.extent;
    const clear = () => {
      const observed = hoverObserver.current;
      // A frame may already have handed facts to the new owner before passive cleanup.
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
    // An inactive chart no longer draws, so it must not leave its tooltip on screen.
    if (!props.active) hideTooltip(owner);
  }, [props.active, owner]);
  useEffect(() => { if (task.current) scheduler.wake(task.current); });
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
      hover.current = reduceHover(hover.current, { type: 'key', hour: props.keyboardHour });
      if (task.current) scheduler.wake(task.current);
    }
  }, [props.keyboardHour, props.keyboardRevision, props.pickOwner]);

  const wake = () => { if (task.current) scheduler.wake(task.current); };
  const send = (event: HoverEvent<Point>) => { hover.current = reduceHover(hover.current, event); wake(); };
  const clearKeyed = () => { keyedStopRef.current = null; send({ type: 'key-clear' }); keyText.current = ''; setKeySpoken(''); };
  const keyDown = (event: ReactKeyboardEvent) => {
    // The arrow keys, Home and End hover the chart as the pointer would, with its tooltip read out.
    if ((event.key === 'ArrowUp' || event.key === 'ArrowDown') && props.rows && shown.current) {
      event.preventDefault();
      rowRef.current = keyTarget(event.key, rowRef.current, props.rows.labels.length, 1) ?? 0;
      send({ type: 'key', hour: hover.current.keyed ?? shownHour.current ?? props.playhead ?? props.extent[1] });
      return;
    }
    if ((event.key === 'ArrowLeft' || event.key === 'ArrowRight' || event.key === 'Home' || event.key === 'End') && shown.current && (props.tooltip || props.rows)) {
      event.preventDefault();
      // The keyboard takes over from the pointer, stepping on from the hour shown.
      // The strip's bars in view are stops too, so the keys reach everything the pointer can.
      const { x, until } = shown.current;
      if (props.onNavigateHour) {
        pendingNavigation.current = { hover: hover.current, owner: props.pickOwner };
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
    // The hour the hover shows (the keyboard's, or the pointer's), else the playhead.
    const moment = actionHour(shownHour.current, hover.current, props.playhead);
    // Enter or Space picks the moment shown, as a click on the chart would.
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
  return <div className="time-chart">
    <canvas ref={canvas} tabIndex={0} role="img" aria-label={props.label} aria-describedby={spoken ? `${owner}-events` : undefined}
      aria-keyshortcuts={props.rows ? 'ArrowUp ArrowDown ArrowLeft ArrowRight Home End' : props.tooltip ? 'ArrowLeft ArrowRight Home End' : undefined} onKeyDown={keyDown} onBlur={clearKeyed} />
    <p id={`${owner}-events`} className="sr-only">{spoken ? `Events: ${spoken}.` : ''}</p>
    <p className="sr-only" aria-live="polite">{keySpoken}</p>
    {zoomed && <ZoomReset onReset={() => controls.current?.dispatch({ type: 'reset' })} />}
  </div>;
}
