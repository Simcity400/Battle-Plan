import { useEffect, useId, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent } from 'react';
import { Icon } from '../../components/Icons';
import { mix, parseColor, readableText } from '../../shell/appearance/color';
import { font } from '../draw';
import { approach, easeTau } from '../easing';
import { fitLabelWidth } from '../labels';
import { scheduler } from '../scheduler';
import type { FrameInfo, FrameTask } from '../scheduler';
import { backingSize, toLocal } from '../surface';
import { timeTicks } from '../timeAxis';
import { readTokens, subscribeTokens } from '../tokens';
import { hideTooltip, showTooltip } from '../tooltip';
import type { TooltipContent } from '../tooltip';
import { EMPTY_VIEW, reduceView } from '../view';
import type { ViewAction, ViewState } from '../view';
import { AXIS_HEIGHT, ROW_HEIGHT, barHours, inSpans, laneAt, leftColumn, spanAt } from './layout';
import type { TimelineLane, TimelineSpan } from './layout';
import '../charts.css';

export type { TimelineLane, TimelineSpan } from './layout';

export interface TimelineProps {
  /** Names the chart for screen readers. */
  label: string;
  /** Every hour the timeline can show: the branch or the widget's range. */
  extent: readonly [number, number];
  /** The playback hour: bars stop and the left column shows holders there. */
  playhead: number | null;
  lanes: readonly TimelineLane[];
  /** Icons inside bars, and holder names in bars. */
  showIcons: boolean;
  showNames: boolean;
  /** Text in the left column when a lane has no holder. */
  emptyLabel?: string;
  /** Hours the data was not recorded: a lane there has an unknown holder, not an empty slot. */
  unobserved?: readonly { firstHour: number; lastHour: number }[];
  hoursPerSecond?: number;
  tooltip?: (lane: TimelineLane, span: TimelineSpan | null, hour: number) => TooltipContent | null;
  onContextMenu?: (hour: number, clientX: number, clientY: number) => void;
  active: boolean;
  revision: unknown;
}

/** The narrowest time span a zoom may reach: two weeks. */
const MIN_SPAN = 14 * 24;
const ICON = 32;
const BAR_ICON = 20;
const PAD_RIGHT = 14;
/** How long the left column cross-fades a new holder. */
const FADE_MS = 350;

/** A lane timeline: one lane per slot with bars for who held it and when, and a left
 * column naming each slot's holder at the playhead (the approved style-4 look). Time
 * zooms like the time chart: wheel, Shift/middle-drag pan, drag the dates to stretch,
 * double-click or Escape to reset. */
export default function Timeline(props: TimelineProps) {
  const owner = useId();
  const canvas = useRef<HTMLCanvasElement>(null);
  const latest = useRef(props);
  latest.current = props;
  const [zoomed, setZoomed] = useState(false);
  const [height, setHeight] = useState(AXIS_HEIGHT + ROW_HEIGHT + 8);
  const task = useRef<FrameTask | null>(null);
  const controls = useRef<{ dispatch: (action: ViewAction) => void } | null>(null);

  useEffect(() => {
    const element = canvas.current!;
    const context = element.getContext('2d')!;
    let tokens = readTokens();
    let view: ViewState = EMPTY_VIEW;
    let eased: [number, number] | null = null;
    let x: [number, number] = [0, 1];
    let left = 120;
    let fitted = 120;
    let labelKey = '';
    let pointer: { x: number; y: number; clientX: number; clientY: number } | null = null;
    let drag: { mode: 'axis'; grab: number } | { mode: 'pan'; lastX: number } | null = null;
    let zoomedFor = '';
    const holders = new Map<string, { id: string | null; previous: TimelineSpan | null; current: TimelineSpan | null; t: number }>();
    const images = new Map<string, HTMLImageElement>();

    const plotWidth = () => Math.max(8, element.clientWidth - left - PAD_RIGHT);
    const toX = (hour: number) => left + (hour - x[0]) / Math.max(1e-9, x[1] - x[0]) * plotWidth();
    const hourAt = (px: number) => x[0] + (px - left) / plotWidth() * (x[1] - x[0]);
    const dispatch = (action: ViewAction) => {
      const p = latest.current;
      view = reduceView(view, action, { x, extent: p.extent, y: [0, 1], minSpan: MIN_SPAN });
      setZoomed(view.x != null);
      wake();
    };
    controls.current = { dispatch };

    const image = (source: string | undefined) => {
      if (!source) return null;
      let found = images.get(source);
      if (!found) {
        found = new Image();
        found.onload = () => wake();
        found.src = source;
        if (images.size > 512) images.clear();
        images.set(source, found);
      }
      return found.complete && found.naturalWidth ? found : null;
    };
    const drawIcon = (source: string | undefined, px: number, py: number, size: number) => {
      const loaded = image(source);
      if (loaded) context.drawImage(loaded, px, py, size, size * loaded.naturalHeight / Math.max(1, loaded.naturalWidth));
    };

    const draw = (info: FrameInfo): boolean => {
      const p = latest.current;
      const layoutWidth = element.clientWidth;
      const layoutHeight = element.clientHeight;
      if (!layoutWidth || !layoutHeight) return false;
      const rect = element.getBoundingClientRect();
      const size = backingSize({ layoutWidth, layoutHeight, rectWidth: rect.width, rectHeight: rect.height, devicePixelRatio: window.devicePixelRatio || 1 });
      if (element.width !== size.width) element.width = size.width;
      if (element.height !== size.height) element.height = size.height;
      context.setTransform(size.scaleX, 0, 0, size.scaleY, 0, 0);
      context.clearRect(0, 0, layoutWidth, layoutHeight);

      // The left column fits its longest slot or holder name, measured over the whole campaign.
      const key = p.lanes.map(lane => `${lane.label}|${lane.spans.map(span => span.label).join('|')}`).join('\n');
      if (key !== labelKey) {
        labelKey = key;
        context.font = font(tokens, tokens.fontSize.body, 500);
        const texts = p.lanes.flatMap(lane => [lane.label, p.emptyLabel ?? 'Empty', ...lane.spans.map(span => span.label)]);
        fitted = fitLabelWidth(texts, text => context.measureText(text).width, 120, 280, ICON + 30);
      }
      left = leftColumn(fitted, layoutWidth);
      const extentKey = `${p.extent[0]}:${p.extent[1]}`;
      if (extentKey !== zoomedFor) {
        if (zoomedFor && view.x) { view = EMPTY_VIEW; setZoomed(false); }
        if (zoomedFor) eased = null;
        zoomedFor = extentKey;
      }
      const target: [number, number] = view.x ? [view.x[0], view.x[1]] : [p.extent[0], p.extent[1]];
      const tau = drag ? 0 : easeTau(tokens.motion, p.hoursPerSecond ?? 24, tokens.reducedMotion);
      const glide = (current: number, goal: number) => goal + approach(current - goal, 0, info.dt, tau);
      x = eased ? [glide(eased[0], target[0]), glide(eased[1], target[1])] : target;
      eased = x;
      const bottom = AXIS_HEIGHT + p.lanes.length * ROW_HEIGHT;
      const right = left + plotWidth();

      // Dates along the top, from the recording's own calendar.
      context.font = font(tokens, tokens.fontSize.small);
      const ticks = timeTicks(x[0], x[1], plotWidth(), text => context.measureText(text).width);
      context.strokeStyle = mix(tokens.surface, tokens.border, 0.8);
      context.lineWidth = 1;
      for (const hour of ticks.grid) {
        const px = Math.round(toX(hour)) + 0.5;
        context.beginPath(); context.moveTo(px, AXIS_HEIGHT - 4); context.lineTo(px, bottom); context.stroke();
      }
      context.fillStyle = tokens.muted;
      context.textAlign = 'center'; context.textBaseline = 'middle';
      for (const label of ticks.labels) context.fillText(label.text, toX(label.hour), AXIS_HEIGHT / 2);

      let animating = false;
      const holderHour = p.playhead ?? p.extent[1];
      const unknown = inSpans(p.unobserved, Math.floor(holderHour));
      p.lanes.forEach((lane, index) => {
        const y = AXIS_HEIGHT + index * ROW_HEIGHT;
        const centre = y + ROW_HEIGHT / 2;
        if (index) { context.fillStyle = mix(tokens.surface, tokens.border, 0.5); context.fillRect(8, y, layoutWidth - 16, 1); }
        // The holder at the playhead (or the end, off Live), cross-fading when it changes.
        const current = unknown ? null : spanAt(lane.spans, holderHour);
        const state = holders.get(lane.id) ?? { id: current?.id ?? null, previous: null, current, t: 1 };
        if (state.id !== (current?.id ?? null)) { state.previous = state.current; state.current = current; state.id = current?.id ?? null; state.t = 0; }
        state.t = tokens.reducedMotion ? 1 : Math.min(1, state.t + info.dt / FADE_MS);
        if (state.t < 1) animating = true;
        holders.set(lane.id, state);
        const holder = (span: TimelineSpan | null, alpha: number) => {
          context.globalAlpha = alpha;
          context.textAlign = 'left'; context.textBaseline = 'middle';
          if (span) {
            drawIcon(span.icon, 10, centre - ICON / 2, ICON);
            context.fillStyle = tokens.text; context.font = font(tokens, tokens.fontSize.body, 500);
            context.fillText(span.label, ICON + 18, centre - 7, left - ICON - 28);
          } else {
            context.setLineDash([3, 3]); context.strokeStyle = mix(tokens.surface, tokens.text, 0.3);
            context.beginPath(); context.roundRect(11.5, centre - 15.5, 30, 30, 5); context.stroke(); context.setLineDash([]);
            context.fillStyle = tokens.muted; context.font = font(tokens, tokens.fontSize.body);
            context.fillText(unknown ? 'Not recorded' : p.emptyLabel ?? 'Empty', ICON + 18, centre - 7, left - ICON - 28);
          }
          context.globalAlpha = 1;
        };
        if (state.t < 1) holder(state.previous, 1 - state.t);
        holder(state.current, state.t);
        context.fillStyle = tokens.muted; context.font = font(tokens, tokens.fontSize.small);
        context.textAlign = 'left'; context.textBaseline = 'middle';
        context.fillText(lane.label, ICON + 18, centre + 8, left - ICON - 28);

        context.save();
        context.beginPath(); context.rect(left, AXIS_HEIGHT - 2, plotWidth() + 10, bottom); context.clip();
        for (const span of lane.spans) {
          const hours = barHours(span, p.playhead);
          if (!hours) continue;
          const from = Math.max(left, toX(hours[0]) + 1);
          const to = Math.min(right, toX(hours[1]) - 1);
          if (to - from < 1.5) continue;
          const fill = span.shade == null ? mix(tokens.surface, tokens.text, 0.1) : mix(tokens.surface, tokens.accent, 0.3 + 0.6 * span.shade);
          context.fillStyle = fill;
          context.beginPath(); context.roundRect(from, centre - 12, to - from, 24, 5); context.fill();
          let textX = from + 7;
          if ((lane.icons ?? p.showIcons) && span.icon && to - from > 28) { drawIcon(span.icon, from + 2, centre - BAR_ICON / 2, BAR_ICON); textX = from + BAR_ICON + 6; }
          if (!(lane.names ?? p.showNames)) continue;
          context.font = font(tokens, tokens.fontSize.small, 500);
          if (context.measureText(span.label).width + (textX - from) + 6 < to - from) {
            const background = parseColor(fill);
            context.fillStyle = span.shade == null || !background ? tokens.text : readableText(background);
            context.fillText(span.label, textX, centre);
          }
        }
        context.restore();
      });

      if (p.playhead != null && p.playhead >= x[0] && p.playhead <= x[1]) {
        const px = Math.round(toX(p.playhead)) + 0.5;
        context.strokeStyle = tokens.accent; context.lineWidth = 1.5;
        context.beginPath(); context.moveTo(px, AXIS_HEIGHT - 6); context.lineTo(px, bottom); context.stroke();
        context.fillStyle = tokens.accent;
        context.beginPath(); context.moveTo(px - 4, AXIS_HEIGHT - 8); context.lineTo(px + 4, AXIS_HEIGHT - 8); context.lineTo(px, AXIS_HEIGHT - 3); context.fill();
      }

      // Hover: the left column describes the lane's holder at the playhead; the plot, the span under the pointer.
      const index = pointer && !drag ? laneAt(pointer.y, p.lanes.length) : null;
      if (pointer && index != null && p.tooltip) {
        const lane = p.lanes[index];
        const hour = pointer.x < left ? holderHour : Math.min(hourAt(pointer.x), p.playhead ?? Infinity);
        const span = spanAt(lane.spans, Math.floor(hour));
        const content = p.tooltip(lane, span && span.firstHour <= (p.playhead ?? Infinity) ? span : null, Math.floor(hour));
        if (content) showTooltip(owner, content, pointer.clientX, pointer.clientY, `${lane.id}:${span?.id ?? ''}:${Math.floor(hour)}`); else hideTooltip(owner);
      } else hideTooltip(owner);
      element.style.cursor = drag?.mode === 'pan' ? 'grabbing' : pointer && pointer.y < AXIS_HEIGHT && pointer.x > left ? 'ew-resize' : 'default';

      const settled = Math.abs(x[0] - target[0]) + Math.abs(x[1] - target[1]) < 1e-3;
      return !settled || animating || drag != null;
    };

    const current: FrameTask = { frame: draw };
    task.current = current;
    const wake = () => scheduler.wake(current);
    const remove = scheduler.add(current);
    const unsubscribe = subscribeTokens(next => { tokens = next; labelKey = ''; wake(); });
    const observer = new ResizeObserver(() => wake());
    observer.observe(element);

    const local = (event: PointerEvent | WheelEvent | MouseEvent) => {
      const point = toLocal(event.clientX, event.clientY, element.getBoundingClientRect(), element.clientWidth, element.clientHeight);
      return { ...point, clientX: event.clientX, clientY: event.clientY };
    };
    const down = (event: PointerEvent) => {
      const point = local(event);
      if (point.x < left) return;
      if (event.button === 0 && point.y < AXIS_HEIGHT) drag = { mode: 'axis', grab: hourAt(point.x) };
      else if (event.button === 1 || (event.button === 0 && event.shiftKey)) drag = { mode: 'pan', lastX: point.x };
      else return;
      event.preventDefault();
      element.focus({ preventScroll: true });
      element.setPointerCapture(event.pointerId);
      wake();
    };
    const move = (event: PointerEvent) => {
      pointer = local(event);
      if (drag?.mode === 'axis') dispatch({ type: 'axis-x', grab: drag.grab, fraction: (pointer.x - left) / plotWidth() });
      else if (drag?.mode === 'pan') {
        dispatch({ type: 'pan', hours: (drag.lastX - pointer.x) / plotWidth() * (x[1] - x[0]) });
        drag.lastX = pointer.x;
      }
      wake();
    };
    const up = () => { drag = null; wake(); };
    const leave = () => { pointer = null; hideTooltip(owner); wake(); };
    const wheel = (event: WheelEvent) => {
      const point = local(event);
      if (point.x < left || !event.deltaY) return;
      event.preventDefault();
      dispatch({ type: 'wheel', at: hourAt(point.x), steps: Math.max(-3, Math.min(3, event.deltaY * (event.deltaMode === 1 ? 33 : 1) / 100)) });
    };
    const doubleClick = () => dispatch({ type: 'reset' });
    const menu = (event: MouseEvent) => {
      const handler = latest.current.onContextMenu;
      const point = local(event);
      if (!handler || point.x < left || laneAt(point.y, latest.current.lanes.length) == null) return;
      event.preventDefault();
      handler(Math.floor(Math.min(hourAt(point.x), latest.current.playhead ?? Infinity)), event.clientX, event.clientY);
    };
    element.addEventListener('pointerdown', down);
    element.addEventListener('pointermove', move);
    element.addEventListener('pointerup', up);
    element.addEventListener('pointercancel', up);
    element.addEventListener('pointerleave', leave);
    element.addEventListener('wheel', wheel, { passive: false });
    element.addEventListener('dblclick', doubleClick);
    element.addEventListener('contextmenu', menu);
    return () => {
      remove(); unsubscribe(); observer.disconnect(); hideTooltip(owner);
      element.removeEventListener('pointerdown', down);
      element.removeEventListener('pointermove', move);
      element.removeEventListener('pointerup', up);
      element.removeEventListener('pointercancel', up);
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
    if (!props.active) hideTooltip(owner);
  }, [props.active, owner]);
  useEffect(() => { setHeight(AXIS_HEIGHT + props.lanes.length * ROW_HEIGHT + 8); }, [props.lanes.length]);
  useEffect(() => { if (task.current) scheduler.wake(task.current); });

  const keyDown = (event: ReactKeyboardEvent) => {
    if (event.key === 'Escape' && zoomed) { event.preventDefault(); controls.current?.dispatch({ type: 'reset' }); }
    if ((event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) && props.onContextMenu && props.playhead != null) {
      event.preventDefault();
      const bounds = canvas.current!.getBoundingClientRect();
      props.onContextMenu(Math.floor(props.playhead), bounds.left + bounds.width / 2, bounds.top + bounds.height / 2);
    }
  };
  return <div className="timeline-chart">
    <canvas ref={canvas} tabIndex={0} role="img" aria-label={props.label} onKeyDown={keyDown} style={{ height }} />
    {zoomed && <button type="button" className="button small time-chart-reset" onClick={() => controls.current?.dispatch({ type: 'reset' })}>
      <Icon name="refresh" />Reset zoom</button>}
  </div>;
}

