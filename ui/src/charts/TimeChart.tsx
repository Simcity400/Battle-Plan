import { useEffect, useId, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent } from 'react';
import { Icon } from '../components/Icons';
import { BADGE_STRIP, badgesAt, drawBadges, visibleBadges } from './badges';
import type { EventBadge } from './badges';
import { clipToPlot, dayLabel, drawAxes, drawCrosshair, drawPlayhead, drawSelection } from './draw';
import { approach, easeTau } from './easing';
import { axisFraction, hitZone, plotFrame, pointerHour, wheelSteps } from './frame';
import type { Padding, PlotFrame } from './frame';
import { scheduler } from './scheduler';
import type { FrameInfo, FrameTask } from './scheduler';
import { backingSize, toLocal } from './surface';
import { readTokens, subscribeTokens } from './tokens';
import type { ChartTokens } from './tokens';
import { hideTooltip, showTooltip } from './tooltip';
import type { TooltipContent } from './tooltip';
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

export interface TimeChartProps {
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
  padding?: Partial<Padding>;
  layers: readonly ChartLayer[];
  formatValue: (value: number) => string;
  /** Event glyphs along the top edge. Hovering one snaps to its hour. */
  badges?: readonly EventBadge[];
  /** Tooltip content for an hour under the pointer, with any event badges hovered
   * there; null shows none. */
  tooltip?: (hour: number, frame: PlotFrame, badges: readonly EventBadge[]) => TooltipContent | null;
  /** Moves a hovered hour onto something nearby, such as an event tick. */
  snap?: (hour: number, frame: PlotFrame) => number;
  /** Drawn after the plot, unclipped: end labels and markers. */
  overlay?: (ctx: CanvasRenderingContext2D, frame: PlotFrame, tokens: ChartTokens) => void;
  onContextMenu?: (hour: number, clientX: number, clientY: number) => void;
  /** A click (not a drag) on the plot picks that hour, e.g. to open or move a breakdown. */
  onPick?: (hour: number) => void;
  /** Off-screen charts stop drawing. */
  active: boolean;
  /** Changes whenever the data changes, so the chart redraws. */
  revision: unknown;
}

const DEFAULT_PADDING: Padding = { left: 44, right: 14, top: 12, bottom: 24 };
/** The narrowest time span a zoom may reach: two weeks. */
const MIN_SPAN = 14 * 24;
const BOX_THRESHOLD = 8;
/** How long a click waits to see if it is the first of a double-click (which
 * resets the zoom instead of picking): Windows' default double-click time. */
const DOUBLE_CLICK_MS = 500;

type Drag = { mode: 'axis-x'; grab: number } | { mode: 'axis-y'; grab: number } | { mode: 'pan'; lastX: number } | { mode: 'box'; x0: number; y0: number; x1: number; y1: number };

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
  const task = useRef<FrameTask | null>(null);
  const controls = useRef<{ dispatch: (action: ViewAction) => void } | null>(null);

  useEffect(() => {
    const element = canvas.current!;
    const context = element.getContext('2d')!;
    let tokens = readTokens();
    let view: ViewState = EMPTY_VIEW;
    // The badges last described, compared one by one (a count and range could repeat after a pan).
    let spokenFor: readonly EventBadge[] = [];
    let eased: { x: [number, number]; y: [number, number] } | null = null;
    let frame: PlotFrame | null = null;
    let device: { width: number; height: number } | null = null;
    let pointer: { x: number; y: number; clientX: number; clientY: number } | null = null;
    let drag: Drag | null = null;
    // The extent the zoom belongs to: another branch or range starts unzoomed.
    let zoomedFor = '';
    // Tooltip content for the hovered hour, rebuilt only when the hour, the hovered
    // badges or the data change.
    let hovered: { hour: number; badges: readonly EventBadge[]; props: TimeChartProps; content: TooltipContent | null } | null = null;

    const dispatch = (action: ViewAction) => {
      if (!frame) return;
      view = reduceView(view, action, { x: frame.x, y: frame.y, extent: latest.current.extent, minSpan: MIN_SPAN, log: frame.log });
      setZoomed(view.x != null || view.y != null);
      wake();
    };
    controls.current = { dispatch };

    const draw = (info: FrameInfo): boolean => {
      const p = latest.current;
      const layoutWidth = element.clientWidth;
      const layoutHeight = element.clientHeight;
      if (!layoutWidth || !layoutHeight) return false;
      const rect = element.getBoundingClientRect();
      const size = backingSize({ layoutWidth, layoutHeight, rectWidth: rect.width, rectHeight: rect.height, devicePixelRatio: window.devicePixelRatio || 1,
        devicePixelWidth: device?.width, devicePixelHeight: device?.height });
      if (element.width !== size.width) element.width = size.width;
      if (element.height !== size.height) element.height = size.height;

      // Badges sit above the plot, so a chart with them keeps room there.
      const padding = { ...DEFAULT_PADDING, ...(p.badges ? { top: BADGE_STRIP } : {}), ...p.padding };
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
      let data: [number, number] | null = null;
      for (const layer of p.layers) {
        const extent = layer.extent?.(x);
        if (extent) data = data ? [Math.min(data[0], extent[0]), Math.max(data[1], extent[1])] : [extent[0], extent[1]];
      }
      const fitted = p.fitValues?.(data) ?? null;
      const targetY = resolveY(view, fitted ? { ...p.axes, yMin: p.axes.yMin ?? fitted[0], yMax: p.axes.yMax ?? fitted[1] } : p.axes, data);
      const y: [number, number] = eased ? [glide(eased.y[0], targetY.range[0]), glide(eased.y[1], targetY.range[1])] : [targetY.range[0], targetY.range[1]];
      eased = { x, y };
      frame = plotFrame(layoutWidth, layoutHeight, padding, x, y, targetY.log);

      context.setTransform(size.scaleX, 0, 0, size.scaleY, 0, 0);
      context.clearRect(0, 0, layoutWidth, layoutHeight);
      drawAxes(context, frame, tokens, p.formatValue);
      context.save();
      clipToPlot(context, frame);
      const until = shownUntil(p.playhead, p.playback, p.afterPlayhead === true);
      for (const layer of p.layers) layer.draw(context, frame, tokens, { playhead: until });
      context.restore();
      if (p.playhead != null && p.playback === 'cursor') drawPlayhead(context, frame, tokens, p.playhead);
      if (p.badges) drawBadges(context, frame, tokens, p.badges, until);
      const drawn = p.badges ? visibleBadges(p.badges, frame.x, until) : [];
      if (drawn.length !== spokenFor.length || drawn.some((badge, index) => badge !== spokenFor[index])) {
        spokenFor = drawn;
        setSpoken(drawn.map(badge => `${dayLabel(badge.hour)}, ${badge.label}`).join('; '));
      }
      p.overlay?.(context, frame, tokens);
      if (drag?.mode === 'box') drawSelection(context, tokens, drag);

      // Live hover: re-resolved every frame, so a still pointer stays current during playback.
      const zone = pointer ? hitZone(pointer, frame) : null;
      const pointed = pointer && (!drag || drag.mode !== 'box') ? pointedAt(pointer, frame, p) : null;
      if (pointer && pointed) {
        drawCrosshair(context, frame, tokens, pointed.hour);
        const hour = Math.floor(pointed.hour);
        if (!hovered || hovered.hour !== hour || hovered.props !== p || !sameBadges(hovered.badges, pointed.badges)) {
          hovered = { hour, badges: pointed.badges, props: p, content: p.tooltip?.(hour, frame, pointed.badges) ?? null };
        }
        const key = `${hour}:${pointed.badges.map(badge => `${badge.hour}|${badge.label}`).join(',')}`;
        if (hovered.content) showTooltip(owner, hovered.content, pointer.clientX, pointer.clientY, key); else hideTooltip(owner);
      } else { hovered = null; hideTooltip(owner); }
      element.style.cursor = drag?.mode === 'pan' ? 'grabbing' : pointed?.badges.length ? 'pointer' : zone === 'x-axis' ? 'ew-resize'
        : zone === 'y-axis' ? 'ns-resize' : zone === 'plot' ? 'crosshair' : 'default';

      const ySpan = Math.abs(targetY.range[1] - targetY.range[0]) || Math.max(1, Math.abs(targetY.range[1]));
      const settled = Math.abs(x[0] - targetX[0]) + Math.abs(x[1] - targetX[1]) < 1e-3
        && Math.abs(y[0] - targetY.range[0]) + Math.abs(y[1] - targetY.range[1]) < 1e-5 * ySpan;
      return !settled || drag != null;
    };
    /** The hour under the pointer as hover and right-click both use it: clamped to the
     * playhead and snapped to nearby marks. */
    const pointedHour = (px: number, plot: PlotFrame, p: TimeChartProps): number | null => {
      const until = shownUntil(p.playhead, p.playback, p.afterPlayhead === true);
      const hour = pointerHour(plot.hourAt(px), plot.x, until);
      if (hour == null || !p.snap) return hour;
      // A snap may only land where the pointer itself could: in view and not after the playhead.
      const snapped = p.snap(hour, plot);
      return pointerHour(snapped, plot.x, until) === snapped ? snapped : hour;
    };
    /** What the pointer is over: an event badge (snapping to its hour) or an hour of the plot. */
    const pointedAt = (point: { x: number; y: number }, plot: PlotFrame, p: TimeChartProps): { hour: number; badges: readonly EventBadge[] } | null => {
      const badges = p.badges ? badgesAt(point, plot, p.badges, shownUntil(p.playhead, p.playback, p.afterPlayhead === true)) : [];
      if (badges.length) return { hour: badges[0].hour, badges };
      if (hitZone(point, plot) !== 'plot') return null;
      const hour = pointedHour(point.x, plot, p);
      return hour == null ? null : { hour, badges: [] };
    };
    const sameBadges = (a: readonly EventBadge[], b: readonly EventBadge[]) => a.length === b.length && a.every((badge, index) => badge === b[index]);
    const current: FrameTask = { frame: draw };
    task.current = current;
    const wake = () => scheduler.wake(current);
    const remove = scheduler.add(current);
    const unsubscribe = subscribeTokens(next => { tokens = next; wake(); });

    let observer: ResizeObserver | null = null;
    const resize = (entries: ResizeObserverEntry[]) => {
      const box = entries[0]?.devicePixelContentBoxSize?.[0];
      device = box ? { width: box.inlineSize, height: box.blockSize } : null;
      wake();
    };
    try { observer = new ResizeObserver(resize); observer.observe(element, { box: 'device-pixel-content-box' }); }
    catch { observer = new ResizeObserver(resize); observer.observe(element); }

    const local = (event: PointerEvent | WheelEvent | MouseEvent) => {
      const point = toLocal(event.clientX, event.clientY, element.getBoundingClientRect(), element.clientWidth, element.clientHeight);
      return { ...point, clientX: event.clientX, clientY: event.clientY };
    };
    const down = (event: PointerEvent) => {
      // The second press of a double-click never picks, however slow the double-click.
      if (event.detail >= 2 && pendingPick) { clearTimeout(pendingPick); pendingPick = null; }
      if (!frame || (event.button !== 0 && event.button !== 1)) return;
      const point = local(event);
      const zone = hitZone(point, frame);
      // A click on an event badge picks its hour.
      const shown = latest.current;
      const badges = event.button === 0 && shown.badges ? badgesAt(point, frame, shown.badges, shownUntil(shown.playhead, shown.playback, shown.afterPlayhead === true)) : [];
      if (badges.length && latest.current.onPick) {
        schedulePick(badges[0].hour);
        return;
      }
      if (!zone) return;
      if (event.button === 0 && zone === 'x-axis') drag = { mode: 'axis-x', grab: frame.hourAt(point.x) };
      else if (event.button === 0 && zone === 'y-axis') drag = { mode: 'axis-y', grab: frame.valueAt(point.y) };
      else if (zone === 'plot' && (event.button === 1 || event.shiftKey)) drag = { mode: 'pan', lastX: point.x };
      else if (zone === 'plot') drag = { mode: 'box', x0: point.x, y0: point.y, x1: point.x, y1: point.y };
      else return;
      event.preventDefault();
      // Preventing the default also stops the chart taking focus; take it so Escape resets.
      element.focus({ preventScroll: true });
      element.setPointerCapture(event.pointerId);
      wake();
    };
    const move = (event: PointerEvent) => {
      pointer = local(event);
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
    let pendingPick: ReturnType<typeof setTimeout> | null = null;
    // Wait out a possible second click: a double-click resets the zoom instead.
    const schedulePick = (hour: number) => {
      if (pendingPick) clearTimeout(pendingPick);
      pendingPick = setTimeout(() => { pendingPick = null; latest.current.onPick?.(Math.floor(hour)); }, DOUBLE_CLICK_MS);
    };
    const up = () => {
      const finished = drag;
      drag = null;
      if (finished?.mode === 'box' && frame && Math.abs(finished.x1 - finished.x0) > BOX_THRESHOLD) {
        dispatch({ type: 'box', x: [frame.hourAt(finished.x0), frame.hourAt(finished.x1)],
          y: Math.abs(finished.y1 - finished.y0) > BOX_THRESHOLD ? [frame.valueAt(finished.y1), frame.valueAt(finished.y0)] : undefined });
      } else if (finished?.mode === 'box' && frame && Math.abs(finished.y1 - finished.y0) <= BOX_THRESHOLD && latest.current.onPick) {
        const hour = pointedHour(finished.x0, frame, latest.current);
        if (hour != null) schedulePick(hour);
      }
      wake();
    };
    const cancel = () => { drag = null; wake(); };
    const leave = () => { pointer = null; hideTooltip(owner); wake(); };
    const wheel = (event: WheelEvent) => {
      if (!frame) return;
      const point = local(event);
      const steps = wheelSteps(event.deltaY, event.deltaMode);
      if (hitZone(point, frame) !== 'plot' || !steps) return;
      event.preventDefault();
      dispatch({ type: 'wheel', at: frame.hourAt(point.x), steps });
    };
    const doubleClick = (event: MouseEvent) => {
      if (pendingPick) { clearTimeout(pendingPick); pendingPick = null; }
      if (!frame) return;
      const zone = hitZone(local(event), frame);
      dispatch({ type: zone === 'x-axis' ? 'reset-x' : zone === 'y-axis' ? 'reset-y' : 'reset' });
    };
    const menu = (event: MouseEvent) => {
      const handler = latest.current.onContextMenu;
      if (!handler || !frame) return;
      const pointed = pointedAt(local(event), frame, latest.current);
      if (!pointed) return;
      event.preventDefault();
      handler(Math.floor(pointed.hour), event.clientX, event.clientY);
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
      remove(); unsubscribe(); observer?.disconnect(); hideTooltip(owner);
      element.removeEventListener('pointerdown', down);
      element.removeEventListener('pointermove', move);
      element.removeEventListener('pointerup', up);
      element.removeEventListener('pointercancel', cancel);
      if (pendingPick) clearTimeout(pendingPick);
      element.removeEventListener('pointerleave', leave);
      element.removeEventListener('wheel', wheel);
      element.removeEventListener('dblclick', doubleClick);
      element.removeEventListener('contextmenu', menu);
      task.current = null;
      controls.current = null;
    };
  }, [owner]);

  useEffect(() => {
    if (task.current) scheduler.setVisible(task.current, props.active);
    // An inactive chart no longer draws, so it must not leave its tooltip on screen.
    if (!props.active) hideTooltip(owner);
  }, [props.active, owner]);
  useEffect(() => { if (task.current) scheduler.wake(task.current); });

  const keyDown = (event: ReactKeyboardEvent) => {
    if (event.key === 'Escape' && zoomed) { event.preventDefault(); controls.current?.dispatch({ type: 'reset' }); }
    // Enter or Space picks the moment shown, as a click on the chart would.
    if ((event.key === 'Enter' || event.key === ' ') && props.onPick) {
      event.preventDefault();
      props.onPick(Math.floor(props.playhead ?? props.extent[1]));
    }
    if ((event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) && props.onContextMenu && props.playhead != null) {
      event.preventDefault();
      const bounds = canvas.current!.getBoundingClientRect();
      props.onContextMenu(Math.floor(props.playhead), bounds.left + bounds.width / 2, bounds.top + bounds.height / 2);
    }
  };
  return <div className="time-chart">
    <canvas ref={canvas} tabIndex={0} role="img" aria-label={props.label} aria-describedby={spoken ? `${owner}-events` : undefined} onKeyDown={keyDown} />
    <p id={`${owner}-events`} className="sr-only">{spoken ? `Events: ${spoken}.` : ''}</p>
    {zoomed && <button type="button" className="button small time-chart-reset" onClick={() => controls.current?.dispatch({ type: 'reset' })}>
      <Icon name="refresh" />Reset zoom</button>}
  </div>;
}
