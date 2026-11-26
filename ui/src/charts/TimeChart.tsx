import { useEffect, useId, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent } from 'react';
import { ZoomReset } from './ZoomReset';
import { setStyle } from './domText.ts';
import WidgetState from '../components/WidgetState';
import { inFrame } from '../dashboard/frameUpdates';
import { drawCrosshair, drawSelection } from './draw';
import { approach, easeTau } from './easing';
import { useMotionNow } from './liveMotion.ts';
import { axisFraction, hitZone, keyedHour, keyedStopHour, keyStops, pointerHour, wheelSteps } from './frame';
import { actionHour, hoverAt, NO_HOVER, pointerMenu, reduceHover } from './hover';
import type { HoverEvent, HoverState } from './hover';
import type { PlotFrame } from './frame';
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
import { canvasMeasureText } from './textWidth';
import {
  TIME_CHART_DEFAULT_PADDING,
  TIME_CHART_MIN_SPAN,
  drawTimeChartScene,
  fitTimeChartFrame,
  resolveTimeChartTargetX,
  resolveTimeChartTargetY,
  type ChartLayer as PureChartLayer,
} from './renderTimeChart';
import './charts.css';

export type ChartLayer = PureChartLayer<CanvasRenderingContext2D>;

export type { ChartPick } from './pick';

export interface TimeChartProps {
  label: string;
  extent: Range;
  playhead: number | null;
  liveHead?: LiveHead;
  playback: PlaybackMode;
  afterPlayhead?: boolean;
  windowHours?: number;
  hoursPerSecond?: number;
  axes: AxisSettings;
  branchStart: number;
  layers: readonly ChartLayer[];
  formatValue?: (value: number, ticks: readonly number[]) => string;
  onSummaryView?: (view: { from: number; to: number; end: number; width: number }) => void;
  onNavigateHour?: (key: 'ArrowLeft' | 'ArrowRight' | 'Home' | 'End', from: number | null, x: Range, until: number | null) => void;
  keyboardHour?: number | null;
  keyboardRevision?: unknown;
  tooltip?: (hour: number, frame: PlotFrame) => TooltipContent | null;
  onHover?: (hour: number | null) => void;
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

const MIN_SPAN = TIME_CHART_MIN_SPAN;
const SUMMARY_INTERVAL_MS = 150;
const HOVER_REST_MS = 80;
const BOX_THRESHOLD = 8;

type Drag = { mode: 'axis-x'; grab: number } | { mode: 'axis-y'; grab: number } | { mode: 'pan'; lastX: number } | { mode: 'box'; x0: number; y0: number; x1: number; y1: number };

export type LiveHead = Live<number>;

function withLiveHead(props: TimeChartProps): TimeChartProps {
  const live = props.liveHead;
  if (!live || props.playhead == null) return props;
  const head = Math.max(props.extent[0], Math.min(props.extent[1], live.read()));
  return head === props.playhead ? props : { ...props, playhead: head };
}

export default function TimeChart(props: TimeChartProps) {
  const owner = useId();
  const canvas = useRef<HTMLCanvasElement>(null);
  const overlay = useRef<HTMLCanvasElement>(null);
  const latest = useRef(props);
  latest.current = props;
  const motionNow = useMotionNow();
  const [zoomed, setZoomed] = useState(false);
  type Point = { x: number; y: number; clientX: number; clientY: number };
  const hover = useRef<HoverState<Point>>(NO_HOVER);
  const pendingNavigation = useRef<{ hover: HoverState<Point>; owner: unknown; tooltip: number } | null>(null);
  const shownHour = useRef<number | null>(null);
  const hoverObserver = useRef<{ callback: NonNullable<TimeChartProps['onHover']>;
    pickOwner: unknown; branchStart: number; first: number; last: number; hour: number | null } | undefined>(undefined);
  const [keySpoken, setKeySpoken] = useState('');
  const keyText = useRef('');
  const keyIntent = useRef<boolean | { since: number }>(false);
  const shown = useRef<{ x: Range; until: number | null } | null>(null);
  const picks = useRef(new ChartPicks()).current;
  picks.update({ owner: props.pickOwner, extent: props.extent, branchStart: props.branchStart, active: props.active, onPick: props.onPick });
  const pickFrame = useRef<(ReturnType<ChartPicks['drawn']> & { props: TimeChartProps }) | null>(null);
  const task = useRef<FrameTask | null>(null);
  const controls = useRef<{ dispatch: (action: ViewAction) => void } | null>(null);

  useEffect(() => {
    const element = canvas.current!;
    const drawing = drawingSurface(element), context = drawing.context!;
    const measure = canvasMeasureText(context);
    const sheet = overlay.current ? drawingSurface(overlay.current) : null, overlayContext = sheet?.context ?? null;
    let tokens = readTokens();
    let view: ViewState = EMPTY_VIEW;
    let reported: { handler: (x: Range) => void; x: Range } | null = null;
    let telling: { handler: (x: Range) => void; x: Range } | null = null;
    let eased: { x: [number, number]; y: [number, number] } | null = null;
    let frame: PlotFrame | null = null;
    let painted: readonly unknown[] | null = null;
    let overlaid = false;
    const surface = watchCanvasSize(element, () => wake());
    let pointer: { x: number; y: number; clientX: number; clientY: number } | null = null;
    let drag: Drag | null = null;
    let zoomedFor = '';
    let hovered: { hour: number; tooltip: TimeChartProps['tooltip']; revision: unknown; content: TooltipContent | null; key: string } | null = null;
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

    const dispatch = (action: ViewAction) => {
      if (!frame) return;
      view = reduceView(view, action, { x: frame.x, y: frame.y, extent: latest.current.extent, minSpan: MIN_SPAN, log: frame.log });
      setZoomed(view.x != null || view.y != null);
      wake();
    };
    controls.current = { dispatch };

    const draw = (info: FrameInfo): boolean => {
      const p = withLiveHead(latest.current);
      const { layoutWidth, layoutHeight, backing: size } = surface.read();
      if (!layoutWidth || !layoutHeight) return false;
      sizeCanvas(drawing.canvas, size.width, size.height);
      if (sheet && sizeCanvas(sheet.canvas, size.width, size.height)) overlaid = false;

      const padding = TIME_CHART_DEFAULT_PADDING;
      const extentKey = `${p.extent[0]}:${p.extent[1]}:${p.branchStart}`;
      if (extentKey !== zoomedFor) {
        if (zoomedFor && (view.x || view.y)) { view = EMPTY_VIEW; setZoomed(false); }
        if (zoomedFor) eased = null;
        zoomedFor = extentKey;
      }
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
      const targetY = resolveTimeChartTargetY(p.layers, x, view, p.axes);
      const y: [number, number] = eased ? [glide(eased.y[0], targetY.range[0]), glide(eased.y[1], targetY.range[1])] : [targetY.range[0], targetY.range[1]];
      const ySpan = Math.abs(targetY.range[1] - targetY.range[0]) || Math.max(1, Math.abs(targetY.range[1]));
      if ((Math.abs(y[0] - targetY.range[0]) + Math.abs(y[1] - targetY.range[1])) * plotHeight < 0.5 * ySpan) {
        y[0] = targetY.range[0]; y[1] = targetY.range[1];
      }
      eased = { x, y };
      frame = fitTimeChartFrame({
        layoutWidth,
        layoutHeight,
        padding,
        x,
        y,
        log: targetY.log,
        tokens,
        formatValue: p.formatValue,
        measure,
      });

      const until = shownUntil(p.playhead, p.playback, p.afterPlayhead === true);
      const baseKey = [size.width, size.height, frame.plotLeft, frame.plotTop, frame.plotWidth, frame.plotHeight, frame.x[0], frame.x[1], frame.y[0], frame.y[1],
        frame.log, tokens, p.layers, p.revision, until, p.playhead, p.playback, p.overlay, p.formatValue,
        ...(p.data?.revision() ?? [])];
      pickFrame.current = { ...picks.drawn({ frame, until }, p.onPick), props: p };
      if (!painted || painted.length !== baseKey.length || !painted.every((value, index) => Object.is(value, baseKey[index]))) {
        drawTimeChartScene(
          context,
          frame,
          tokens,
          {
            layoutWidth,
            layoutHeight,
            scaleX: size.scaleX,
            scaleY: size.scaleY,
            layers: p.layers,
            playhead: p.playhead,
            playback: p.playback,
            afterPlayhead: p.afterPlayhead,
            until,
            formatValue: p.formatValue,
            overlay: p.overlay,
          },
          measure,
        );
        painted = baseKey;
      }
      if (overlayContext && overlaid) {
        overlayContext.setTransform(1, 0, 0, 1, 0, 0);
        overlayContext.clearRect(0, 0, size.width, size.height);
        overlaid = false;
      }
      overlayContext?.setTransform(size.scaleX, 0, 0, size.scaleY, 0, 0);
      shown.current = { x, until };
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
      if (drag?.mode === 'box' && overlayContext) { drawSelection(overlayContext, tokens, drag); overlaid = true; }

      const zone = pointer ? hitZone(pointer, frame) : null;
      const plot = frame;
      const hit = hoverAt(hover.current, frame.x, until, point => !drag || drag.mode !== 'box' ? pointedAt(point, plot, p) : null);
      shownHour.current = hit ? Math.floor(hit.hour) : null;
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
      if (hit) {
        if (overlayContext) { drawCrosshair(overlayContext, frame, tokens, hit.hour); overlaid = true; }
        const hour = Math.floor(hit.hour);
        if (!hovered || hovered.hour !== hour || hovered.tooltip !== p.tooltip || hovered.revision !== p.revision) {
          hovered = { hour, tooltip: p.tooltip, revision: p.revision,
            content: p.tooltip?.(hour, frame) ?? null,
            key: String(hour) };
        }
        let point: { clientX: number; clientY: number } | null = hit.by === 'pointer' ? pointer : null;
        if (!point) {
          const rect = element.getBoundingClientRect();
          point = { clientX: rect.left + frame.toX(hit.hour) * rect.width / layoutWidth, clientY: rect.top + (frame.plotTop + 8) * rect.height / layoutHeight };
        }
        const held = tooltipStore.get(), intent = keyIntent.current;
        const fresh = intent === true || (intent !== false && tooltipStore.revision() === intent.since);
        if (hit.by === 'pointer' || fresh || held == null || held.owner === owner) {
          if (hovered.content) showTooltip(owner, hovered.content, point.clientX, point.clientY, hovered.key); else hideTooltip(owner);
          const text = hit.by === 'keyboard' && hovered.content ? tooltipText(hovered.content) : keyText.current;
          if (text !== keyText.current) { keyText.current = text; setKeySpoken(text); }
        }
      } else { hovered = null; hideTooltip(owner); }
      keyIntent.current = false;
      setStyle(element, 'cursor', drag?.mode === 'pan' ? 'grabbing' : zone === 'x-axis' ? 'ew-resize'
        : zone === 'y-axis' ? 'ns-resize' : zone === 'plot' ? 'crosshair' : 'default');

      const settled = Math.abs(x[0] - targetX[0]) + Math.abs(x[1] - targetX[1]) < 1e-3
        && Math.abs(y[0] - targetY.range[0]) + Math.abs(y[1] - targetY.range[1]) < 1e-5 * ySpan;
      return !settled || drag != null;
    };
    const pointedAt = (point: { x: number; y: number }, plot: PlotFrame, p: TimeChartProps): { hour: number } | null => {
      if (hitZone(point, plot) !== 'plot') return null;
      const until = shownUntil(p.playhead, p.playback, p.afterPlayhead === true);
      const hour = pointerHour(plot.hourAt(point.x), plot.x, until);
      return hour == null ? null : { hour };
    };
    const current: FrameTask = { frame: draw };
    task.current = current;
    const wake = () => scheduler.wake(current);
    const remove = scheduler.add(current, element);
    const unsubscribe = subscribeTokens(next => { tokens = next; wake(); });

    const local = (event: PointerEvent | WheelEvent | MouseEvent) => {
      const point = toLocal(event.clientX, event.clientY, element.getBoundingClientRect(), element.clientWidth, element.clientHeight);
      return { ...point, clientX: event.clientX, clientY: event.clientY };
    };
    let secondPress = false;
    const down = (event: PointerEvent) => {
      pendingNavigation.current = null;
      hover.current = reduceHover(hover.current, { type: 'pointer-down' });
      secondPress = event.detail >= 2;
      if (!frame || (event.button !== 0 && event.button !== 1)) return;
      const point = local(event);
      const zone = hitZone(point, frame);
      if (!zone) return;
      if (event.button === 0 && zone === 'x-axis') drag = { mode: 'axis-x', grab: frame.hourAt(point.x) };
      else if (event.button === 0 && zone === 'y-axis') drag = { mode: 'axis-y', grab: frame.valueAt(point.y) };
      else if (zone === 'plot' && (event.button === 1 || event.shiftKey)) drag = { mode: 'pan', lastX: point.x };
      else if (zone === 'plot') drag = { mode: 'box', x0: point.x, y0: point.y, x1: point.x, y1: point.y };
      else return;
      event.preventDefault();
      element.focus({ preventScroll: true });
      element.setPointerCapture(event.pointerId);
      wake();
    };
    const move = (event: PointerEvent) => {
      pointer = local(event);
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
          y: Math.abs(finished.y1 - finished.y0) > BOX_THRESHOLD ? [frame.valueAt(finished.y1), frame.valueAt(finished.y0)] : undefined });
      } else if (finished?.mode === 'box' && frame && Math.abs(finished.y1 - finished.y0) <= BOX_THRESHOLD && latest.current.onPick) {
        const drawn = pickFrame.current;
        const pointed = drawn && pointedAt({ x: finished.x0, y: finished.y0 }, drawn.context.frame, drawn.props);
        if (pointed && !secondPress) pick(pointed.hour, { x: finished.x0, y: finished.y0 });
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
      if (!frame) return;
      const point = local(event);
      const steps = wheelSteps(event.deltaY, event.deltaMode);
      const zone = hitZone(point, frame);
      if (zone !== 'plot' || !steps) return;
      event.preventDefault();
      dispatch({ type: 'wheel', at: frame.hourAt(point.x), steps });
    };
    const doubleClick = (event: MouseEvent) => {
      if (!frame) return;
      const zone = hitZone(local(event), frame);
      if (latest.current.onPick && zone === 'plot') return;
      dispatch({ type: zone === 'x-axis' ? 'reset-x' : zone === 'y-axis' ? 'reset-y' : 'reset' });
    };
    const menu = (event: MouseEvent) => {
      const fromPointer = pointerMenu(hover.current);
      hover.current = reduceHover(hover.current, { type: 'contextmenu' });
      if (!fromPointer) { event.preventDefault(); return; }
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
  }, [props.active, owner]);
  useEffect(() => { if (task.current) scheduler.wake(task.current); });
  useEffect(() => props.liveHead?.subscribe(() => { if (task.current) scheduler.wake(task.current); }), [props.liveHead]);
  useEffect(() => props.data?.subscribe(() => { if (task.current) scheduler.wake(task.current); }), [props.data]);
  useEffect(() => {
    if (props.keyboardHour !== undefined) {
      const pending = pendingNavigation.current;
      if (props.onNavigateHour && (!pending || pending.hover !== hover.current || !Object.is(pending.owner, props.pickOwner))) return;
      pendingNavigation.current = null;
      keyIntent.current = pending ? { since: pending.tooltip } : true;
      hover.current = reduceHover(hover.current, { type: 'key', hour: props.keyboardHour });
      if (task.current) scheduler.wake(task.current);
    }
  }, [props.keyboardHour, props.keyboardRevision, props.pickOwner]);

  const wake = () => { if (task.current) scheduler.wake(task.current); };
  const send = (event: HoverEvent<Point>) => { hover.current = reduceHover(hover.current, event); wake(); };
  const clearKeyed = () => { send({ type: 'key-clear' }); keyText.current = ''; setKeySpoken(''); };
  const stopsCache = useRef<{ source: readonly number[]; first: number; last: number; shown: number[] } | null>(null);
  const keyDown = (event: ReactKeyboardEvent) => {
    if ((event.key === 'ArrowLeft' || event.key === 'ArrowRight' || event.key === 'Home' || event.key === 'End') && shown.current && props.tooltip) {
      event.preventDefault();
      keyIntent.current = true;
      const { x, until } = shown.current;
      if (props.onNavigateHour && Math.min(x[1], until ?? x[1]) < x[0]) {
        pendingNavigation.current = null;
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
      const from = hover.current.keyed ?? shownHour.current;
      if (props.keyHours) {
        const first = x[0], last = Math.min(x[1], until ?? x[1]);
        const cached = stopsCache.current;
        const visibleStops = cached && cached.source === props.keyHours && cached.first === first && cached.last === last
          ? cached.shown
          : (stopsCache.current = { source: props.keyHours, first, last, shown: last < first ? [] : keyStops(props.keyHours, first, last) }).shown;
        send({ type: 'key', hour: keyedStopHour(event.key, from, visibleStops) });
      } else {
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
  return <div className="time-chart">
    <canvas ref={canvas} tabIndex={0} role="img" aria-label={props.label}
      aria-keyshortcuts={props.tooltip ? 'ArrowLeft ArrowRight Home End' : undefined} onKeyDown={keyDown} onBlur={clearKeyed} />
    <p className="sr-only" aria-live="polite">{keySpoken}</p>
    <canvas ref={overlay} className="time-chart-overlay" aria-hidden="true" />
    {zoomed && <ZoomReset onReset={() => controls.current?.dispatch({ type: 'reset' })} />}
    {props.notRecorded && <div className="time-chart-state"><WidgetState kind="unavailable" message="Not recorded" detail={props.notRecorded} /></div>}
  </div>;
}
