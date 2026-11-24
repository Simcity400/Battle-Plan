import ArtFallback from '../../components/ArtFallback';
import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { clipColumns, hatchColumns } from '../gaps';
import type { LostSpan } from '../gaps';
import type { KeyboardEvent as ReactKeyboardEvent } from 'react';
import { ZoomReset } from '../ZoomReset';
import { mix, parseColor, readableText } from '../../shell/appearance/color';
import { font } from '../draw';
import { countryTooltipContent, drawCountryFlag } from '../countryFlag';
import { canvasImageRead } from '../images';
import type { CountryFlagLook } from '../../components/CountryFlag';
import { approach, easeTau } from '../easing';
import { useMotionNow } from '../liveMotion.ts';
import { fitLabelWidth } from '../labels';
import type { Live } from '../live';
import { scheduler } from '../scheduler';
import type { FrameInfo, FrameTask } from '../scheduler';
import { drawingSurface, toLocal } from '../surface';
import { timeTicks } from '../timeAxis';
import { readTokens, subscribeTokens } from '../tokens';
import { hideTooltip, scrollTooltip, showTooltip, tooltipStore, tooltipText } from '../tooltip';
import type { TooltipContent } from '../tooltip';
import { EMPTY_VIEW, reduceView } from '../view';
import type { ViewAction, ViewState } from '../view';
import { AXIS_HEIGHT, INDENT, ROW_HEIGHT, activateLane, barGlyphHours, barOpenEnd, laneLabelY, laneRowAt, laneRows, laneTop, spanAtRow, bandHalf, bandHitAt, barExtent, barHitAt, barHours, inSpans, joinedStep, markerAt, periodBarHeight, pointerBarSpans, escapeDismissesTooltip, spanY, stackedSpanAt, laneAt, laneBandLayouts, laneBands, lanesInView, laneStops, leftColumn, nextHolder, spanAt } from './layout';
import type { CachedBands, HolderState } from './layout';
import type { TimelineLane, TimelineSpan } from './layout';
import { coversViewport, laneWindow, sizeLaneWindow, timelineHeight, timelineViewStyle, timelineViewport, timelineY } from './viewport';
import { labelMoment, liveHeaded, viewForExtent } from './extent';
import { drawCross, drawDisclosure, acceptsLaneClick, acceptsLaneMenu } from './marks';
import { catalogueWidth } from './labelCatalogue';
import type { TimelineLabel } from './labelCatalogue';
import { barContent, barNameFits, END_MARK_ROOM } from './barLabel';
import '../charts.css';
import { textWidth as measuredWidth } from '../textWidth.ts';

export type { LaneGeometry, TimelineLane } from './layout';

export interface TimelineProps {
  label: string;
  gaps?: readonly LostSpan[];
  extent: readonly [number, number];
  playhead: number | null;
  liveHead?: Live<number>;
  growTo?: number;
  labelHour?: number;
  labelCatalogue?: readonly TimelineLabel[];
  preserveZoomOnExtentChange?: boolean;
  onLaneActivate?: (lane: TimelineLane) => void;
  lanes: readonly TimelineLane[];
  showIcons: boolean;
  showNames: boolean;
  emptyLabel?: string;
  unobserved?: readonly { firstHour: number; lastHour: number }[];
  rowHeight?: number;
  stickyAxis?: boolean;
  fadePresentation?: boolean;
  truncateLabels?: boolean;
  holderIconAlign?: 'top' | 'center';
  hoursPerSecond?: number;
  bandMax?: number;
  barHeight?: number;
  tooltip?: (lane: TimelineLane, span: TimelineSpan | null, hour: number, full?: boolean) => TooltipContent | null;
  description?: string;
  onHover?: (hour: number | null) => void;
  dismissPointerTooltipOnEscape?: boolean;
  onContextMenu?: (hour: number, clientX: number, clientY: number, lane?: TimelineLane, span?: TimelineSpan | null) => void;
  labelMenu?: boolean;
  onLanesInView?: (first: number, last: number) => void;
  onSummaryView?: (view: { from: number; to: number; width: number }, lanes: readonly TimelineLane[]) => void;
  onNavigateHour?: (lane: TimelineLane, from: number, forward: boolean) => Promise<number | null>;
  active: boolean;
  revision: unknown;
}

const MIN_SPAN = 14 * 24;
const SUMMARY_INTERVAL_MS = 150;
const ICON = 32;
const BAR_ICON = 20;
const PAD_RIGHT = 14;
const FADE_MS = 350;
const SQUEEZE = 1.12;

const sameLaneLabels = (a: readonly TimelineLane[], b: readonly TimelineLane[]) => a.length === b.length && a.every((lane, index) => {
  const other = b[index];
  return lane.compact === other.compact && lane.depth === other.depth && lane.heading === other.heading && !lane.pill === !other.pill
    && lane.label === other.label && lane.identity?.label === other.identity?.label
    && lane.spans.length === other.spans.length && lane.spans.every((span, at) => span.label === other.spans[at].label);
});
const sameCatalogue = (a: readonly TimelineLabel[], b: readonly TimelineLabel[]) => a.length === b.length && a.every((label, index) =>
  label.text === b[index].text && label.space === b[index].space && label.weight === b[index].weight && label.size === b[index].size);

function withLiveHead(props: TimelineProps): TimelineProps {
  const live = props.liveHead;
  if (!live || props.playhead == null) return props;
  const { extent, head } = liveHeaded(props.extent, live.read(), props.growTo);
  return head === props.playhead && extent === props.extent ? props : { ...props, playhead: head, extent };
}

function scrollParent(element: HTMLElement): HTMLElement | null {
  let parent = element.parentElement;
  while (parent && !/auto|scroll/.test(getComputedStyle(parent).overflowY)) parent = parent.parentElement;
  return parent;
}

function viewHeight(parent: HTMLElement, rowHeight: number): number {
  const style = getComputedStyle(parent);
  return Math.max(AXIS_HEIGHT + rowHeight, parent.clientHeight - (parseFloat(style.paddingTop) || 0) - (parseFloat(style.paddingBottom) || 0));
}

export default function Timeline(props: TimelineProps) {
  const owner = useId();
  const canvas = useRef<HTMLCanvasElement>(null);
  const spacer = useRef<HTMLDivElement>(null);
  const viewportElement = useRef<HTMLDivElement>(null);
  const offset = useRef(0);
  const rows = useMemo(() => laneRows(props.lanes, props.rowHeight), [props.lanes, props.rowHeight]);
  const geometry = useRef(rows);
  geometry.current = rows;
  const region = useRef(laneWindow(props.lanes.length, 0, window.innerHeight, props.rowHeight ?? ROW_HEIGHT, rows));
  const latest = useRef(props);
  latest.current = props;
  const headed = () => withLiveHead(latest.current);
  const motionNow = useMotionNow();
  const hoverHour = useRef<number | null>(null);
  const notifyHover = (hour: number | null) => {
    if (hoverHour.current === hour) return;
    hoverHour.current = hour;
    queueMicrotask(() => headed().onHover?.(hour));
  };
  const [zoomed, setZoomed] = useState(false);
  const [artFallbacks, setArtFallbacks] = useState<readonly { id: string; left: number; top: number; width: number; height: number; opacity: number; icon: 'flask' | 'sparkle'; clipPath: string }[]>([]);
  const fallbacksShown = useRef(artFallbacks);
  const rowHeight = props.rowHeight ?? ROW_HEIGHT;
  const height = timelineHeight(props.lanes.length, rowHeight, rows);
  const [viewportHeight, setShownHeight] = useState(window.innerHeight);
  const heightShown = useRef(viewportHeight);
  const setViewportHeight = useRef((next: number) => { if (heightShown.current !== next) { heightShown.current = next; setShownHeight(next); } }).current;
  const viewportState = (spacerElement = spacer.current!, scrolling = viewportElement.current!) => {
    const p = headed();
    return timelineViewport(spacerElement, scrolling, p.lanes.length, p.rowHeight ?? ROW_HEIGHT, p.stickyAxis !== false, geometry.current);
  };
  const task = useRef<FrameTask | null>(null);
  const controls = useRef<{ dispatch: (action: ViewAction) => void } | null>(null);
  const flagLoaded = useRef(() => {
    if (task.current) { scheduler.wake(task.current); refreshKeyed.current(); }
  }).current;

  useEffect(() => {
    const element = canvas.current!;
    const spacerElement = spacer.current!;
    const scrolling = viewportElement.current!;
    const paper = drawingSurface(element), context = paper.context!;
    let tokens = readTokens();
    let view: ViewState = EMPTY_VIEW;
    let eased: [number, number] | null = null;
    type SummaryView = { from: number; to: number; width: number; lanes: readonly TimelineLane[] };
    const sameLanes = (a: readonly TimelineLane[], b: readonly TimelineLane[]) => a.length === b.length && a.every((lane, index) => lane === b[index]);
    let summarySent: SummaryView | null = null, summaryLatest: SummaryView | null = null;
    let summaryAt = -Infinity, summaryTimer: ReturnType<typeof setTimeout> | null = null;
    const sendSummary = () => {
      summaryTimer = null;
      const sent = summaryLatest, props = headed();
      if (!sent) return;
      summarySent = sent; summaryAt = performance.now();
      props.onSummaryView?.({ from: sent.from, to: sent.to, width: sent.width }, sent.lanes);
    };
    let x: [number, number] = [0, 1];
    let left = 120;
    let fitted = 120;
    let labelsFitted = false;
    let measured: readonly TimelineLane[] = [];
    let labelEmpty: string | undefined;
    let labelTruncate: boolean | undefined;
    let catalogue: TimelineProps['labelCatalogue'];
    let inView: [number, number] = [0, -1];
    const laneUnder = (y: number) => {
      const p = headed();
      const axisTop = p.stickyAxis !== false ? offset.current : 0;
      return y < axisTop + AXIS_HEIGHT ? null : laneAt(y, p.lanes.length, geometry.current);
    };
    let pointer: { x: number; y: number; clientX: number; clientY: number } | null = null;
    let drag: { mode: 'axis'; grab: number } | { mode: 'pan'; lastX: number } | null = null;
    let suppressClick = false;
    let pressed: { x: number; y: number } | null = null;
    let zoomedFor = '';
    const holders = new Map<string, HolderState>();
    const images = new Map<string, HTMLImageElement>();
    let layouts = new Map<string, CachedBands>();
    const pointed = (lane: TimelineLane, index: number, px: number, py: number, hour: number): { span: TimelineSpan | null; hour: number } => {
      const subrow = laneRowAt(py, index, geometry.current);
      if (subrow == null) return { span: null, hour: Math.floor(hour) };
      if (lane.paint) return { span: spanAt(lane.spans, Math.floor(hour)), hour: Math.floor(hour) };
      const marker = markerAt(lane.spans, px, toX, x[0], Math.min(x[1], headed().playhead ?? Infinity), subrow);
      if (marker) return { span: marker, hour: marker.firstHour };
      const bands = layouts.get(lane.id)?.bands;
      const hit = bands && bandHitAt(bands, px, hour);
      if (hit && subrow === 0) return { span: lane.spans.find(span => span.id === hit.spanId) ?? null, hour: hit.hour };
      const rows = geometry.current;
      const centre = laneTop(index, rows) + (rows.offsets[index + 1] - rows.offsets[index]) / 2;
      const offset = py - centre;
      const bar = barHitAt(pointerBarSpans(lane.spans, offset), px, headed().playhead, toX, hourAt, left, left + plotWidth(), subrow);
      if (bar) return bar;
      return { span: stackedSpanAt(lane.spans, Math.floor(hour), offset) ?? spanAtRow(lane.spans, Math.floor(hour), subrow), hour: Math.floor(hour) };
    };

    let canvasWidth = 0;
    const plotWidth = () => Math.max(8, (canvasWidth || element.clientWidth) - left - PAD_RIGHT);
    const toX = (hour: number) => left + (hour - x[0]) / Math.max(1e-9, x[1] - x[0]) * plotWidth();
    const hourAt = (px: number) => x[0] + (px - left) / plotWidth() * (x[1] - x[0]);
    const dispatch = (action: ViewAction) => {
      const p = headed();
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
    const fits = new Map<string, string>();
    const fitText = (text: string, width: number): string => {
      const key = `${context.font}|${Math.round(width)}|${text}`;
      const known = fits.get(key);
      if (known != null) return known;
      const room = width * SQUEEZE;
      let result = text;
      if (measuredWidth(context, text) > room) {
        let low = 0;
        let high = text.length;
        while (low < high) {
          const middle = Math.ceil((low + high) / 2);
          if (measuredWidth(context, `${text.slice(0, middle).trimEnd()}…`) <= room) low = middle; else high = middle - 1;
        }
        result = `${text.slice(0, low).trimEnd()}…`;
      }
      if (fits.size > 4096) fits.clear();
      fits.set(key, result);
      return result;
    };
    const drawIcon = (source: string | undefined, px: number, cy: number, size: number, alignTop = false, flag?: CountryFlagLook) => {
      if (flag) {
        const decoded = flag.flag ? canvasImageRead(flag.flag, flagLoaded).image : null;
        const height = size * (decoded ? decoded.naturalHeight / Math.max(1, decoded.naturalWidth) : 2 / 3);
        drawCountryFlag(context, flag, px, alignTop ? cy - size / 2 : cy - height / 2, size, height, tokens, flagLoaded);
        return;
      }
      const loaded = image(source);
      if (!loaded) return;
      const height = size * loaded.naturalHeight / Math.max(1, loaded.naturalWidth);
      context.drawImage(loaded, px, alignTop ? cy - size / 2 : cy - height / 2, size, height);
    };

    let viewMeasure: { key: unknown[]; view: ReturnType<typeof viewportState>; window?: { top: number; height: number }; size?: ReturnType<typeof sizeLaneWindow> } | null = null;
    const draw = (info: FrameInfo): boolean => {
      const p = headed();
      const row = p.rowHeight ?? ROW_HEIGHT;
      const key = [p.lanes.length, row, p.stickyAxis, geometry.current];
      if (!viewMeasure || key.some((value, index) => !Object.is(value, viewMeasure!.key[index]))) viewMeasure = { key, view: viewportState(spacerElement, scrolling) };
      const { visible, offset: scrolled } = viewMeasure.view;
      offset.current = scrolled;
      const axisTop = p.stickyAxis !== false ? scrolled : 0;
      const viewport = laneWindow(p.lanes.length, visible.top, visible.height, row, geometry.current);
      region.current = viewport;
      if (!viewMeasure.size || viewMeasure.window?.top !== viewport.top || viewMeasure.window.height !== viewport.height) {
        viewMeasure.size = sizeLaneWindow(element, viewport, scrolled, window.devicePixelRatio || 1, paper.canvas);
        viewMeasure.window = { top: viewport.top, height: viewport.height };
      }
      const size = viewMeasure.size;
      if (!size) return false;
      const shown = p.lanes.slice(viewport.first, viewport.end);
      const fallbacks: typeof artFallbacks[number][] = [];
      const { layoutWidth, layoutHeight } = size;
      canvasWidth = layoutWidth;
      context.setTransform(size.scaleX, 0, 0, size.scaleY, 0, 0);
      context.clearRect(0, 0, layoutWidth, layoutHeight);
      context.translate(0, -viewport.top);
      if (pointer) pointer = local(pointer);

      const refit = !labelsFitted || p.emptyLabel !== labelEmpty || p.truncateLabels !== labelTruncate || (p.labelCatalogue
        ? p.labelCatalogue !== catalogue && !(catalogue && sameCatalogue(catalogue, p.labelCatalogue))
        : !!catalogue || p.lanes !== measured && !sameLaneLabels(measured, p.lanes));
      catalogue = p.labelCatalogue;
      measured = p.lanes;
      labelEmpty = p.emptyLabel;
      labelTruncate = p.truncateLabels;
      if (refit) {
        labelsFitted = true;
        let measuring = '';
        const measureIn = (shown: string, text: string) => {
          if (shown !== measuring) { context.font = shown; measuring = shown; }
          return measuredWidth(context, text);
        };
        if (p.labelCatalogue) fitted = catalogueWidth(p.labelCatalogue, item =>
          measureIn(font(tokens, item.size ?? tokens.fontSize.body, item.weight), item.text));
        else {
          const texts = p.lanes.flatMap(lane => (lane.compact ? [lane.label] : [lane.label, p.emptyLabel ?? 'Empty', ...(lane.identity ? [lane.identity.label] : []), ...lane.spans.map(span => span.label)])
            .map(text => [text, ((lane.depth ?? 0) + (lane.expanded == null ? 0 : 1)) * INDENT, p.truncateLabels === false ? lane.heading ? 600 : 500 : lane.pill ? 500 : 600] as const));
          fitted = fitLabelWidth(texts, ([text, indent, weight]) => measureIn(font(tokens, tokens.fontSize.body, weight), text) + indent,
            120, 280, ICON + 30);
        }
      }
      left = leftColumn(fitted, layoutWidth);
      const plotTop = Math.max(visible.top, axisTop + AXIS_HEIGHT);
      const plotBottom = visible.top + visible.height;
      const visibleLanes: [number, number] = plotBottom > plotTop
        ? lanesInView(plotTop - AXIS_HEIGHT, plotBottom - plotTop + AXIS_HEIGHT, p.lanes.length, geometry.current) : [0, -1];
      if (visibleLanes[0] !== inView[0] || visibleLanes[1] !== inView[1]) {
        inView = visibleLanes;
        queueMicrotask(() => headed().onLanesInView?.(visibleLanes[0], visibleLanes[1]));
      }
      const extentKey = `${p.extent[0]}:${p.extent[1]}`;
      if (extentKey !== zoomedFor) {
        const next = viewForExtent(view, zoomedFor, extentKey, p.preserveZoomOnExtentChange);
        if (next !== view) { view = next; setZoomed(false); }
        if (zoomedFor && (!p.preserveZoomOnExtentChange || !view.x)) eased = null;
        zoomedFor = extentKey;
      }
      const target: [number, number] = view.x ? [view.x[0], view.x[1]] : [p.extent[0], p.extent[1]];
      const tau = drag ? 0 : easeTau(tokens.motion, motionNow(p.hoursPerSecond), tokens.reducedMotion);
      const glide = (current: number, goal: number) => goal + approach(current - goal, 0, info.dt, tau);
      x = eased ? [glide(eased[0], target[0]), glide(eased[1], target[1])] : target;
      if (keyed.current && (viewed.current?.[0] !== target[0] || viewed.current?.[1] !== target[1])) queueMicrotask(() => refreshKeyed.current());
      viewed.current = target;
      eased = x;
      const summaryView = { from: Math.floor(x[0]), to: Math.floor(Math.min(x[1], p.playhead ?? x[1])), width: Math.ceil(plotWidth()),
        lanes: p.lanes.slice(Math.max(0, inView[0]), inView[1] + 1) };
      const same = (view: SummaryView | null) => !!view && view.from === summaryView.from && view.to === summaryView.to && view.width === summaryView.width
        && sameLanes(view.lanes, summaryView.lanes);
      if (summaryView.to >= summaryView.from && !same(summaryLatest)) {
        summaryLatest = summaryView;
        if (same(summarySent)) { if (summaryTimer) clearTimeout(summaryTimer); summaryTimer = null; }
        else if (!summarySent || summarySent.width !== summaryView.width || performance.now() - summaryAt >= SUMMARY_INTERVAL_MS) {
          if (summaryTimer) clearTimeout(summaryTimer); queueMicrotask(sendSummary);
        } else summaryTimer ??= setTimeout(sendSummary, SUMMARY_INTERVAL_MS - (performance.now() - summaryAt));
      }
      const bottom = AXIS_HEIGHT + geometry.current.height;
      const right = left + plotWidth();
      const fallback = (id: string, px: number, top: number, width: number, height: number, opacity: number, icon: 'flask' | 'sparkle', plot = false) => {
        const clipTop = Math.max(viewport.top, axisTop + AXIS_HEIGHT);
        const clipBottom = Math.min(bottom, viewport.top + viewport.height);
        fallbacks.push({ id, left: px, top: top - scrolled, width, height, opacity, icon,
          clipPath: `inset(${Math.max(0, clipTop - top)}px ${Math.max(0, px + width - (plot ? right : layoutWidth))}px ${Math.max(0, top + height - clipBottom)}px ${Math.max(0, (plot ? left : 0) - px)}px)` });
      };

      context.font = font(tokens, tokens.fontSize.small);
      const ticks = timeTicks(x[0], x[1], plotWidth(), text => measuredWidth(context, text));
      const grid = (from: number, to: number) => {
        context.strokeStyle = mix(tokens.surface, tokens.border, 0.8);
        context.lineWidth = 1;
        for (const hour of ticks.grid) {
          const px = Math.round(toX(hour)) + 0.5;
          context.beginPath(); context.moveTo(px, from); context.lineTo(px, to); context.stroke();
        }
      };
      grid(axisTop + AXIS_HEIGHT - 4, bottom);
      const axis = { toX, x, left, right };
      const gaps = p.gaps ?? [];
      hatchColumns(context, tokens, axis, gaps, AXIS_HEIGHT, bottom - AXIS_HEIGHT);

      let animating = false;
      layouts = laneBandLayouts(shown, `${x[0]}:${x[1]}:${left}:${right}:${p.playhead}:${p.bandMax}`, layouts,
        lane => laneBands(lane.spans, p.playhead, toX, value => bandHalf(value, p.bandMax ?? 1, 1, 12), left, right));
      const holderHour = labelMoment(p.playhead, p.extent[1], p.labelHour);
      const unknown = inSpans(p.unobserved, Math.floor(holderHour));
      const kept = new Set(shown.map(lane => lane.id));
      for (const id of holders.keys()) if (!kept.has(id)) holders.delete(id);
      shown.forEach((lane, offset) => {
        const index = viewport.first + offset;
        const y = laneTop(index, geometry.current);
        const row = geometry.current.rowHeights[index];
        const laneHeight = geometry.current.offsets[index + 1] - geometry.current.offsets[index];
        const centre = laneLabelY(index, geometry.current, axisTop, visible.top + visible.height - axisTop) + axisTop;
        const iconSize = Math.min(ICON, Math.max(0, row - 4));
        const disclosure = lane.expanded == null ? 0 : INDENT;
        const indent = (lane.depth ?? 0) * INDENT;
        if (index && lane.separator !== false) { context.fillStyle = mix(tokens.surface, tokens.border, lane.heading ? 0.9 : 0.5); context.fillRect(8, y, layoutWidth - 16, 1); }
        if (lane.separatorAfter) { context.fillStyle = mix(tokens.surface, tokens.border, 0.6); context.fillRect(0, y + laneHeight - 1, layoutWidth, 1); }
        if (lane.expanded != null) drawDisclosure(context, indent + 12, centre, lane.expanded, tokens);
        const current = lane.identity ? { id: lane.id, firstHour: 0, lastHour: 0, ...lane.identity }
          : unknown ? null : spanAt(lane.spans, holderHour);
        const state = nextHolder(holders.get(lane.id), current, p.fadePresentation ?? !!lane.paint);
        state.t = tokens.reducedMotion ? 1 : Math.min(1, state.t + info.dt / FADE_MS);
        if (state.t < 1) animating = true;
        holders.set(lane.id, state);
        const fade = lane.labelStyle === 'side' ? 1 : lane.muted ? 0.55 : 1;
        const blueprint = lane.identity?.tile === 'blueprint';
        const textAt = indent + disclosure + (lane.compact ? 10 : blueprint ? 62 : lane.heading && !lane.identity?.icon && !lane.identity?.countryFlag ? 10 : iconSize + 18);
        const textWidth = Math.max(8, left - textAt - 10);
        const labelText = (text: string) => p.truncateLabels === false ? text : fitText(text, textWidth);
        const holder = (span: TimelineSpan | null, alpha: number) => {
          context.globalAlpha = alpha * fade;
          context.textAlign = 'left'; context.textBaseline = 'middle';
          if (span) {
            if (blueprint) {
              context.fillStyle = tokens.blueprint; context.beginPath(); context.roundRect(8, centre - 14, 46, 28, 4); context.fill();
              context.strokeStyle = tokens.blueprintBorder; context.stroke();
              const loaded = image(span.icon);
              if (loaded) {
                const scale = Math.min(44 / loaded.naturalWidth, 26 / loaded.naturalHeight);
                const w = loaded.naturalWidth * scale, h = loaded.naturalHeight * scale;
                context.drawImage(loaded, 31 - w / 2, centre - h / 2, w, h);
              } else {
                fallback(`${lane.id}:${span.id}`, 8, centre - 14, 46, 28, alpha * fade, 'flask');
              }
            } else drawIcon(span.icon, 10 + indent + disclosure, centre, iconSize, p.holderIconAlign ? p.holderIconAlign === 'top' : !!lane.pill, span.countryFlag);
            context.fillStyle = tokens.text; context.font = font(tokens, tokens.fontSize.body, lane.heading ? 600 : 500);
            if (lane.labelStyle === 'side') { context.font = font(tokens, 10, 600); context.fillStyle = tokens.muted; }
            context.fillText(lane.pill ? span.label : labelText(lane.labelStyle === 'side' ? span.label.toUpperCase() : span.label), textAt, centre - (lane.singleLine ? 0 : 7), textWidth);
          } else {
            context.fillStyle = tokens.muted; context.font = font(tokens, tokens.fontSize.body);
            context.fillText(labelText(unknown ? 'Not recorded' : p.emptyLabel ?? 'Empty'), textAt, centre - (lane.singleLine ? 0 : 7), textWidth);
          }
          context.globalAlpha = 1;
        };
        if (!lane.compact && state.t < 1) holder(state.previous, 1 - state.t);
        if (!lane.compact) holder(state.current, state.t);
        context.globalAlpha = fade;
        context.font = font(tokens, tokens.fontSize.small, lane.pill ? 600 : 400);
        context.textAlign = 'left'; context.textBaseline = 'middle';
        if (lane.compact) {
          context.fillStyle = tokens.muted;
          context.fillText(labelText(lane.label), textAt, centre, textWidth);
        } else if (lane.pill) {
          const width = Math.min(textWidth, measuredWidth(context, lane.pill.text) + 12);
          const pillTop = centre + 1;
          const pillHeight = tokens.fontSize.small + 5;
          context.fillStyle = lane.pill.color;
          context.beginPath(); context.roundRect(textAt, pillTop, width, pillHeight, pillHeight / 2); context.fill();
          const background = parseColor(lane.pill.color);
          context.fillStyle = background ? readableText(background) : tokens.text;
          context.fillText(lane.pill.text, textAt + 6, pillTop + pillHeight / 2 + 0.5, Math.max(1, width - 12));
        } else if (!lane.singleLine) {
          context.fillStyle = tokens.muted;
          context.fillText(labelText(lane.label), textAt, centre + 8, textWidth);
        }
        context.globalAlpha = 1;

        if (lane.paint) {
          context.save();
          context.beginPath(); context.rect(left, y, plotWidth(), laneHeight); context.clip();
          clipColumns(context, axis, gaps);
          lane.paint(context, { top: y, height: laneHeight, left, right, hours: x, toX, playhead: p.playhead }, tokens);
          context.restore();
          return;
        }
        context.save();
        context.beginPath(); context.rect(left, AXIS_HEIGHT, plotWidth() + 10, bottom - AXIS_HEIGHT); context.clip();
        clipColumns(context, axis, gaps);
        const bands = layouts.get(lane.id)?.bands;
        let previous: TimelineSpan | undefined;
        for (const span of lane.spans) {
          const centre = spanY(span, index, geometry.current, lane.spans);
          const spanRow = span.stack == null ? row : 16;
          if (centre + spanRow / 2 < viewport.top || centre - spanRow / 2 > viewport.top + viewport.height) continue;
          const glyphs = () => {
            for (const mark of barGlyphHours(span, p.playhead)) {
              if (mark < x[0] || mark > x[1]) continue;
              if (span.glyphShape === 'cross') drawCross(context, toX(mark), centre, tokens.text);
              else { context.font = font(tokens, tokens.fontSize.small, 600); context.fillStyle = tokens.text; context.textAlign = 'center'; context.fillText(span.glyph!, toX(mark), centre); context.textAlign = 'left'; }
            }
          };
          const positioned = span.glyphHour != null || span.glyphHours != null;
          const hours = barHours(span, p.playhead);
          if (!hours) { if (positioned) glyphs(); continue; }
          if (lane.stepMax != null && span.value != null) {
            const stepY = (value: number) => centre + 12 - 24 * value / Math.max(1, lane.stepMax!);
            const startX = toX(hours[0]), endX = toX(hours[1]);
            context.strokeStyle = tokens.accent; context.lineWidth = 2;
            context.beginPath();
            context.moveTo(startX, joinedStep(previous, span) ? stepY(previous!.value!) : stepY(span.value));
            context.lineTo(startX, stepY(span.value)); context.lineTo(endX, stepY(span.value)); context.stroke();
            if (span.dot && span.firstHour >= x[0]) { context.fillStyle = tokens.accent; context.beginPath(); context.arc(startX, stepY(span.value), 3.5, 0, Math.PI * 2); context.fill(); }
            previous = span; continue;
          }
          if (span.marker) {
            if (span.firstHour < x[0] || span.firstHour >= x[1]) continue;
            const px = toX(span.firstHour), icons = span.icons ?? [span.icon];
            const width = icons.length * (BAR_ICON + 2);
            if (px + width / 2 < left || px - width / 2 > right) continue;
            icons.forEach((source, i) => {
              const iconX = px - width / 2 + i * (BAR_ICON + 2);
              if (image(source)) drawIcon(source, iconX, centre, BAR_ICON);
              else fallback(`${lane.id}:${span.id}:${i}`, iconX, centre - 10, BAR_ICON, BAR_ICON, fade, 'sparkle', true);
            });
            continue;
          }
          if (lane.barMax != null && span.value != null) {
            const extent = barExtent(toX(hours[0]), toX(hours[1]), left, right);
            if (!extent) continue;
            const [from, to] = extent;
            const outcome = span.positiveCount != null && span.negativeCount != null;
            const barHeight = periodBarHeight(outcome ? span.positiveCount! : span.value, lane.barMax, outcome ? 16 : 24);
            context.fillStyle = outcome ? tokens.positive : mix(tokens.surface, tokens.text, 0.7);
            if (to > from && barHeight != null) context.fillRect(from, (outcome ? centre : centre + 10) - barHeight, to - from, barHeight);
            if (outcome) {
              const lost = periodBarHeight(span.negativeCount!, lane.barMax, 16);
              context.fillStyle = tokens.error;
              if (to > from && lost != null) context.fillRect(from, centre, to - from, lost);
            }
            continue;
          }
          const fill = span.color ?? (span.positive ? mix(tokens.surface, tokens.positive, 0.7) : span.shade != null ? mix(tokens.surface, tokens.accent, 0.3 + 0.6 * span.shade)
            : span.emphasis ? mix(tokens.surface, tokens.accent, 0.55) : mix(tokens.surface, tokens.text, 0.1));
          const background = parseColor(fill);
          const onFill = (span.color == null && span.shade == null && !span.emphasis) || !background ? tokens.text : readableText(background);
          context.fillStyle = fill;
          if (span.band) {
            for (const group of bands?.outlines.get(span.id) ?? []) {
              context.beginPath();
              for (const edge of group) { context.lineTo(edge.x0, centre - edge.half); context.lineTo(edge.x1, centre - edge.half); }
              for (const edge of [...group].reverse()) { context.lineTo(edge.x1, centre + edge.half); context.lineTo(edge.x0, centre + edge.half); }
              context.closePath(); context.fill();
            }
            continue;
          }
          const extent = barExtent(toX(hours[0]), toX(hours[1]), left, right);
          if (!extent) { if (positioned) glyphs(); continue; }
          const [from, to] = extent;
          const barHeight = Math.min(spanRow - 4, Math.max(2, span.barHeight ?? lane.barHeight ?? p.barHeight ?? (span.stack == null ? 24 : 12)));
          const radius = Math.min(5, barHeight / 2, (to - from) / 2);
          const open = barOpenEnd(span, p.playhead) && toX(hours[1]) <= right;
          context.beginPath(); context.roundRect(from, centre - barHeight / 2, to - from, barHeight, open ? [radius, 0, 0, radius] : radius); context.fill();
          if (open) { context.globalAlpha = 0.5; context.fillStyle = tokens.surface; context.fillRect(to - 1, centre - barHeight / 2, 1, barHeight); context.globalAlpha = 1; }
          if (positioned) glyphs();
          const endMark = !!span.endLabel && to - from >= 16 && hours[1] >= span.lastHour + 1;
          if (endMark) {
            context.fillStyle = readableText(parseColor(fill) ?? parseColor(tokens.text)!);
            context.font = font(tokens, tokens.fontSize.small, 600);
            context.fillText(span.endLabel!, to - END_MARK_ROOM, centre);
          }
          let textX = from + 7;
          const content = barContent(lane.icons ?? p.showIcons, lane.names ?? p.showNames, !!span.icon || !!span.countryFlag);
          if (content.icon && to - from > 28) { drawIcon(span.icon, from + 2, centre, BAR_ICON, p.holderIconAlign === 'top', span.countryFlag); textX = from + BAR_ICON + 6; }
          context.font = font(tokens, tokens.fontSize.small, 600);
          if (span.glyph && !positioned) {
            const width = measuredWidth(context, span.glyph);
            if (width + (textX - from) + 4 < to - from) {
              context.fillStyle = onFill;
              context.fillText(span.glyph, textX - 2, centre);
              textX += width + 3;
            }
          }
          if (!content.name) continue;
          context.font = font(tokens, tokens.fontSize.small, 500);
          if (barNameFits(measuredWidth(context, span.label), textX - from, to - from, endMark)) {
            context.fillStyle = onFill;
            context.fillText(span.label, textX, centre);
          }
        }
        context.restore();
      });

      if (p.stickyAxis !== false) {
        context.fillStyle = tokens.surface;
        context.fillRect(0, axisTop, layoutWidth, AXIS_HEIGHT);
        grid(axisTop + AXIS_HEIGHT - 4, axisTop + AXIS_HEIGHT);
      }
      context.font = font(tokens, tokens.fontSize.small);
      context.fillStyle = tokens.muted;
      context.textAlign = 'center'; context.textBaseline = 'middle';
      for (const label of ticks.labels) context.fillText(label.text, toX(label.hour), axisTop + AXIS_HEIGHT / 2);

      if (p.playhead != null && p.playhead >= x[0] && p.playhead <= x[1]) {
        const px = Math.round(toX(p.playhead)) + 0.5;
        context.strokeStyle = tokens.accent; context.lineWidth = 1.5;
        context.beginPath(); context.moveTo(px, axisTop + AXIS_HEIGHT - 6); context.lineTo(px, bottom); context.stroke();
        context.fillStyle = tokens.accent;
        context.beginPath(); context.moveTo(px - 4, axisTop + AXIS_HEIGHT - 8); context.lineTo(px + 4, axisTop + AXIS_HEIGHT - 8); context.lineTo(px, axisTop + AXIS_HEIGHT - 3); context.fill();
      }

      const index = pointer && !drag ? laneUnder(pointer.y) : null;
      if (pointer && index != null && p.tooltip && !keyed.current && !quiet.current) {
        const lane = p.lanes[index];
        const { span, hour } = pointer.x < left ? { span: spanAt(lane.spans, Math.floor(holderHour)), hour: Math.floor(holderHour) }
          : pointed(lane, index, pointer.x, pointer.y, Math.min(hourAt(pointer.x), p.playhead ?? Infinity));
        notifyHover(hour);
        const content = countryTooltipContent(p.tooltip(lane, span && span.firstHour <= (p.playhead ?? Infinity) ? span : null, hour),
          undefined, flagLoaded, globalThis.innerHeight);
        if (content) showTooltip(owner, content, pointer.clientX, pointer.clientY, `${lane.id}:${span?.id ?? ''}:${hour}`); else hideTooltip(owner);
      } else if (!keyed.current) { hideTooltip(owner); notifyHover(null); }
      const cursor = drag?.mode === 'pan' ? 'grabbing' : pointer && pointer.y < axisTop + AXIS_HEIGHT && pointer.x > left ? 'ew-resize'
        : index != null && p.onLaneActivate && (p.lanes[index].opens || p.lanes[index].expanded != null) ? 'pointer' : 'default';
      if (element.style.cursor !== cursor) element.style.cursor = cursor;

      const settled = Math.abs(x[0] - target[0]) + Math.abs(x[1] - target[1]) < 1e-3;
      const before = fallbacksShown.current;
      if (before.length !== fallbacks.length || !before.every((item, index) =>
        item.id === fallbacks[index].id && item.left === fallbacks[index].left && item.top === fallbacks[index].top
        && item.width === fallbacks[index].width && item.height === fallbacks[index].height && item.icon === fallbacks[index].icon
        && item.clipPath === fallbacks[index].clipPath && item.opacity === fallbacks[index].opacity)) {
        fallbacksShown.current = fallbacks;
        setArtFallbacks(fallbacks);
      }
      return !settled || animating || drag != null;
    };

    const current: FrameTask = { frame: draw };
    task.current = current;
    const wake = () => scheduler.wake(current);
    const remove = scheduler.add(current, element);
    const unsubscribe = subscribeTokens(next => { tokens = next; labelsFitted = false; wake(); });
    const measureView = () => {
      const parent = scrollParent(spacerElement);
      setViewportHeight(parent ? viewHeight(parent, headed().rowHeight ?? ROW_HEIGHT) : window.innerHeight);
    };
    const observer = new ResizeObserver(() => { if (spacerElement.isConnected) { viewMeasure = null; measureView(); wake(); } });
    observer.observe(element);
    observer.observe(scrolling);
    for (let parent: HTMLElement | null = spacerElement; parent; parent = parent.parentElement) observer.observe(parent);
    measureView();
    const scroll = () => {
      viewMeasure = null;
      const p = headed();
      const { visible } = viewportState(spacerElement, scrolling);
      if (p.active && (p.stickyAxis !== false || !coversViewport(region.current, p.lanes.length, visible, p.rowHeight ?? ROW_HEIGHT, geometry.current))) {
        draw({ now: performance.now(), dt: 0 });
      }
      wake();
      if (keyed.current) refreshKeyed.current();
    };
    document.addEventListener('scroll', scroll, true);
    window.addEventListener('resize', scroll);

    const local = (event: { clientX: number; clientY: number }) => {
      const point = toLocal(event.clientX, event.clientY, element.getBoundingClientRect(), element.clientWidth, element.clientHeight);
      return { ...point, y: timelineY(point.y, region.current), clientX: event.clientX, clientY: event.clientY };
    };
    const down = (event: PointerEvent) => {
      const point = local(event);
      pressed = point; suppressClick = false;
      if (point.x < left) return;
      if (event.button === 0 && point.y < (headed().stickyAxis !== false ? offset.current : 0) + AXIS_HEIGHT) drag = { mode: 'axis', grab: hourAt(point.x) };
      else if (event.button === 1 || (event.button === 0 && event.shiftKey)) drag = { mode: 'pan', lastX: point.x };
      else return;
      suppressClick = true;
      event.preventDefault();
      element.focus({ preventScroll: true });
      element.setPointerCapture(event.pointerId);
      wake();
    };
    const move = (event: PointerEvent) => {
      keyed.current = null;
      quiet.current = false;
      pointer = local(event);
      if (pressed && Math.hypot(pointer.x - pressed.x, pointer.y - pressed.y) > 3) suppressClick = true;
      if (drag?.mode === 'axis') dispatch({ type: 'axis-x', grab: drag.grab, fraction: (pointer.x - left) / plotWidth() });
      else if (drag?.mode === 'pan') {
        dispatch({ type: 'pan', hours: (drag.lastX - pointer.x) / plotWidth() * (x[1] - x[0]) });
        drag.lastX = pointer.x;
      }
      wake();
    };
    const up = () => { drag = null; pressed = null; wake(); };
    const leave = () => { pointer = null; notifyHover(null); hideTooltip(owner); wake(); };
    const wheel = (event: WheelEvent) => {
      const delta = event.deltaY * (event.deltaMode === 1 ? tokens.fontSize.body * 1.4 : 1);
      if (scrollTooltip(owner, delta, event.deltaMode === 2)) { event.preventDefault(); return; }
      const point = local(event);
      if (point.x < left || !event.deltaY) return;
      event.preventDefault();
      dispatch({ type: 'wheel', at: hourAt(point.x), steps: Math.max(-3, Math.min(3, event.deltaY * (event.deltaMode === 1 ? 33 : 1) / 100)) });
    };
    const doubleClick = () => dispatch({ type: 'reset' });
    const click = (event: MouseEvent) => {
      if (!acceptsLaneClick(event.detail, suppressClick)) return;
      const point = local(event);
      const index = laneUnder(point.y);
      if (index != null && activateLane(headed().lanes[index], 'click', headed().onLaneActivate)) {
        keyed.current = { lane: index, id: headed().lanes[index].id, span: 0 };
        element.focus({ preventScroll: true }); wake();
      }
    };
    const menu = (event: MouseEvent) => {
      const handler = headed().onContextMenu;
      const point = local(event);
      const index = laneUnder(point.y);
      if (!handler || !acceptsLaneMenu(point.x, left, headed().labelMenu) || index == null) return;
      event.preventDefault();
      const lane = headed().lanes[index];
      const target = point.x < left ? { hour: labelMoment(headed().playhead, headed().extent[1], headed().labelHour), span: null }
        : pointed(lane, index, point.x, point.y, Math.min(hourAt(point.x), headed().playhead ?? Infinity));
      handler(target.hour, event.clientX, event.clientY, lane, target.span);
    };
    element.addEventListener('pointerdown', down);
    element.addEventListener('pointermove', move);
    element.addEventListener('pointerup', up);
    element.addEventListener('pointercancel', up);
    element.addEventListener('pointerleave', leave);
    element.addEventListener('wheel', wheel, { passive: false });
    element.addEventListener('dblclick', doubleClick);
    element.addEventListener('click', click);
    element.addEventListener('contextmenu', menu);
    return () => {
      if (summaryTimer) clearTimeout(summaryTimer);
      remove(); unsubscribe(); observer.disconnect(); hideTooltip(owner);
      document.removeEventListener('scroll', scroll, true);
      window.removeEventListener('resize', scroll);
      element.removeEventListener('pointerdown', down);
      element.removeEventListener('pointermove', move);
      element.removeEventListener('pointerup', up);
      element.removeEventListener('pointercancel', up);
      element.removeEventListener('pointerleave', leave);
      element.removeEventListener('wheel', wheel);
      element.removeEventListener('dblclick', doubleClick);
      element.removeEventListener('click', click);
      element.removeEventListener('contextmenu', menu);
      task.current = null;
      controls.current = null;
    };
  }, [owner]);

  useEffect(() => {
    if (task.current) scheduler.setVisible(task.current, props.active);
    if (!props.active) { notifyHover(null); hideTooltip(owner); }
  }, [props.active, owner]);
  useEffect(() => {
    const parent = scrollParent(spacer.current!);
    setViewportHeight(parent ? viewHeight(parent, rowHeight) : window.innerHeight);
  }, [rowHeight]);
  useEffect(() => { if (task.current) scheduler.wake(task.current); });
  useEffect(() => props.liveHead?.subscribe(() => { if (task.current) scheduler.wake(task.current); }), [props.liveHead]);

  const keyed = useRef<{ lane: number; id: string; span: number; spanId?: string; hour?: number; navigation?: Promise<void> } | null>(null);
  const refreshKeyed = useRef<() => void>(() => {});
  const quiet = useRef(false);
  const viewed = useRef<readonly [number, number] | null>(null);
  const barsOf = (lane: number) => {
    const p = headed();
    const [from, to] = viewed.current ?? p.extent;
    return laneStops(p.lanes[lane]?.spans ?? [], from, to, p.playhead);
  };
  const [spoken, setSpoken] = useState('');
  const revealLane = (lane: number, span?: TimelineSpan) => {
    const element = spacer.current;
    if (!element) return;
    const bounds = element.getBoundingClientRect();
    const scale = element.offsetHeight ? bounds.height / element.offsetHeight : 1;
    const rows = geometry.current;
    const row = span?.stack == null ? rows.rowHeights[lane] : 16;
    const centre = span ? spanY(span, lane, rows, headed().lanes[lane].spans) : laneTop(lane, rows) + row / 2;
    const top = bounds.top + (centre - row / 2) * scale;
    const bottom = top + row * scale;
    const axis = headed().stickyAxis !== false ? AXIS_HEIGHT * scale : 0;
    let parent = element.parentElement;
    while (parent && !(parent.scrollHeight > parent.clientHeight && /auto|scroll/.test(getComputedStyle(parent).overflowY))) parent = parent.parentElement;
    if (!parent) {
      if (top < axis) window.scrollBy(0, top - axis);
      else if (bottom > window.innerHeight) window.scrollBy(0, bottom - window.innerHeight);
      return;
    }
    const box = parent.getBoundingClientRect();
    const units = parent.clientHeight ? box.height / parent.clientHeight : 1;
    if (top < box.top + axis) parent.scrollTop -= (box.top + axis - top) / units;
    else if (bottom > box.bottom) parent.scrollTop += (bottom - box.bottom) / units;
  };
  const showKeyed = () => {
    const p = headed();
    const at = keyed.current;
    if (!at || !p.tooltip || !canvas.current) return;
    at.lane = p.lanes.findIndex(item => item.id === at.id);
    const lane = p.lanes[at.lane];
    if (!lane) { keyed.current = null; notifyHover(null); hideTooltip(owner); setSpoken(''); return; }
    const bars = barsOf(at.lane);
    const same = at.spanId == null ? -1 : bars.findIndex(bar => bar.id === at.spanId);
    at.span = same >= 0 ? same : Math.max(0, Math.min(at.span, bars.length - 1));
    at.spanId = bars[at.span]?.id;
    const stop = bars[at.span] ?? null;
    const span = stop?.span ?? null;
    const [viewFrom, viewTo] = viewed.current ?? p.extent;
    if (at.hour != null && (at.hour < viewFrom || at.hour > Math.min(viewTo, p.playhead ?? viewTo))) {
      at.hour = undefined; at.navigation = undefined;
    }
    if (!span && p.playhead != null && viewFrom > p.playhead) { notifyHover(null); hideTooltip(owner); setSpoken(`${lane.label}: not reached yet`); return; }
    const hour = at.hour ?? (stop ? Math.floor(stop.hour) : Math.floor(Math.max(viewFrom, Math.min(labelMoment(p.playhead, viewTo, p.labelHour), viewTo))));
    const holder = at.hour == null ? span ?? spanAt(lane.spans, hour) : spanAt(lane.spans, hour);
    const shown = holder && holder.firstHour <= (p.playhead ?? Infinity) ? holder : null;
    notifyHover(hour);
    const raw = p.tooltip(lane, shown, hour);
    if (!raw) { hideTooltip(owner); setSpoken(''); return; }
    const full = p.tooltip(lane, shown, hour, true) ?? raw;
    const content = countryTooltipContent({ ...raw, spoken: tooltipText(full) }, undefined, flagLoaded, globalThis.innerHeight)!;
    const bounds = spacer.current!.getBoundingClientRect();
    const scale = spacer.current!.offsetHeight ? bounds.height / spacer.current!.offsetHeight : 1;
    const rows = geometry.current;
    const tooltipY = span ? spanY(span, at.lane, rows, lane.spans) + (span.stack == null ? rows.rowHeights[at.lane] : 16) / 2
      : laneTop(at.lane, rows) + rows.rowHeights[at.lane];
    showTooltip(owner, content, bounds.left + bounds.width / 2, bounds.top + tooltipY * scale, `key:${lane.id}:${stop?.id ?? shown?.id ?? ''}`);
    setSpoken([tooltipText(content), lane.opens ? 'Enter or Space to open.'
      : lane.expanded == null ? null : `${lane.expanded ? 'Expanded' : 'Collapsed'}. Enter or Space to ${lane.expanded ? 'collapse' : 'expand'}.`]
      .filter(Boolean).join('; '));
  };
  const ownsTooltip = () => { const current = tooltipStore.get(); return current == null || current.owner === owner; };
  refreshKeyed.current = () => { if (keyed.current && headed().active && ownsTooltip()) showKeyed(); };
  useEffect(() => {
    if (!keyed.current) return;
    if (!props.active) { keyed.current = null; setSpoken(''); return; }
    if (ownsTooltip()) showKeyed();
  });
  const keyDown = (event: ReactKeyboardEvent) => {
    if ((event.key === 'PageUp' || event.key === 'PageDown') && scrollTooltip(owner, event.key === 'PageUp' ? -1 : 1, true)) {
      event.preventDefault(); return;
    }
    const lanes = headed().lanes;
    const bars = (lane: number) => barsOf(lane).length;
    if (['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight'].includes(event.key) && lanes.length) {
      event.preventDefault();
      const remembered = keyed.current && lanes.findIndex(lane => lane.id === keyed.current!.id);
      if (keyed.current && remembered != null && remembered >= 0) keyed.current.lane = remembered;
      else keyed.current = null;
      const hadKey = keyed.current !== null;
      const at = keyed.current ?? { lane: 0, id: lanes[0].id, span: Math.max(0, bars(0) - 1) };
      const shownHour = barsOf(at.lane)[at.span]?.hour;
      if (keyed.current) {
        if (event.key === 'ArrowDown') { at.lane = Math.min(lanes.length - 1, at.lane + 1); at.span = Math.max(0, bars(at.lane) - 1); }
        if (event.key === 'ArrowUp') { at.lane = Math.max(0, at.lane - 1); at.span = Math.max(0, bars(at.lane) - 1); }
        if (event.key === 'ArrowLeft') at.span = Math.max(0, at.span - 1);
        if (event.key === 'ArrowRight') at.span = Math.min(Math.max(0, bars(at.lane) - 1), at.span + 1);
      }
      at.id = lanes[at.lane].id;
      if (props.onNavigateHour && (event.key === 'ArrowLeft' || event.key === 'ArrowRight')) {
        const selected = at;
        const [from, to] = viewed.current ?? props.extent;
        const first = !hadKey;
        const forward = hadKey && event.key === 'ArrowRight';
        keyed.current = selected;
        const request = selected.navigation ?? Promise.resolve();
        selected.navigation = request.then(async () => {
          const p = headed(), view = viewed.current ?? p.extent;
          if (keyed.current !== selected || view[0] !== from || view[1] !== to || !p.active) return;
          const lane = p.lanes.find(lane => lane.id === selected.id);
          if (!lane || !p.onNavigateHour) return;
          const start = first ? Math.floor(labelMoment(p.playhead, to, p.labelHour)) + 1
            : selected.hour ?? shownHour ?? labelMoment(p.playhead, to, p.labelHour);
          const hour = await p.onNavigateHour(lane, start, forward);
          const currentView = viewed.current ?? headed().extent;
          if (keyed.current !== selected || currentView[0] !== from || currentView[1] !== to || !headed().active) return;
          if (hour !== null && hour >= from && hour <= Math.min(to, headed().playhead ?? to)) selected.hour = hour;
          showKeyed();
        }).catch(() => { if (keyed.current === selected) showKeyed(); });
        return;
      }
      const next = { ...at, hour: undefined, navigation: undefined };
      next.spanId = undefined;
      keyed.current = next;
      revealLane(next.lane, barsOf(next.lane)[next.span]?.span);
      showKeyed();
      return;
    }
    if (event.key === 'Escape' && keyed.current) { keyed.current = null; quiet.current = true; notifyHover(null); hideTooltip(owner); setSpoken(''); return; }
    if (event.key === 'Escape') {
      if (escapeDismissesTooltip(false, props.dismissPointerTooltipOnEscape)) {
        quiet.current = true; notifyHover(null); hideTooltip(owner); setSpoken('');
      }
      if (zoomed) { event.preventDefault(); controls.current?.dispatch({ type: 'reset' }); }
      return;
    }
    const activeIndex = keyed.current
      ? lanes[keyed.current.lane]?.id === keyed.current.id ? keyed.current.lane : lanes.findIndex(lane => lane.id === keyed.current!.id)
      : -1;
    const activeLane = lanes[activeIndex];
    if (activateLane(activeLane, event.key, headed().onLaneActivate)) { event.preventDefault(); return; }
    const stops = activeLane ? barsOf(activeIndex) : [];
    const same = keyed.current?.spanId == null ? -1 : stops.findIndex(stop => stop.id === keyed.current!.spanId);
    const selected = keyed.current ? stops[same >= 0 ? same : Math.max(0, Math.min(keyed.current.span, stops.length - 1))] : undefined;
    const menuHour = keyed.current?.hour ?? (selected ? selected.hour : labelMoment(headed().playhead, props.extent[1], props.labelHour));
    if ((event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) && props.onContextMenu && menuHour != null) {
      event.preventDefault();
      const bounds = spacer.current!.getBoundingClientRect();
      const scale = spacer.current!.clientHeight ? bounds.height / spacer.current!.clientHeight : 1;
      const { visible } = viewportState();
      const span = selected?.span ?? null;
      props.onContextMenu(Math.floor(menuHour), bounds.left + bounds.width / 2, bounds.top + (visible.top + visible.height / 2) * scale, activeLane, span);
    }
  };
  return <div ref={spacer} className="timeline-chart" style={{ height }}>
    <div ref={viewportElement} className="timeline-view" style={timelineViewStyle(height, viewportHeight, props.stickyAxis !== false)}>
      <canvas ref={canvas} tabIndex={0} role="img" aria-label={props.label} aria-describedby={props.description ? `${owner}-description` : undefined}
        aria-keyshortcuts={props.onLaneActivate ? 'ArrowUp ArrowDown ArrowLeft ArrowRight Enter Space' : 'ArrowUp ArrowDown ArrowLeft ArrowRight'} onKeyDown={keyDown} onBlur={() => { keyed.current = null; notifyHover(null); hideTooltip(owner); setSpoken(''); }}
        style={{ position: 'absolute', top: 0, height: region.current.height }} />
      {artFallbacks.map(tile => <span key={tile.id} style={{ position: 'absolute', left: tile.left, top: tile.top,
        width: tile.width, height: tile.height, opacity: tile.opacity, clipPath: tile.clipPath, pointerEvents: 'none' }}><ArtFallback icon={tile.icon} /></span>)}
      {zoomed && <ZoomReset onReset={() => controls.current?.dispatch({ type: 'reset' })} />}
    </div>
    <p className="sr-only" aria-live="polite">{spoken}</p>
    {props.description && <p id={`${owner}-description`} className="sr-only">{props.description}</p>}
  </div>;
}
