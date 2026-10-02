import { useEffect, useId, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent } from 'react';
import { Icon } from '../../components/Icons';
import { mix, parseColor, readableText } from '../../shell/appearance/color';
import { font } from '../draw';
import { approach, easeTau } from '../easing';
import { fitLabelWidth } from '../labels';
import { scheduler } from '../scheduler';
import type { FrameInfo, FrameTask } from '../scheduler';
import { toLocal } from '../surface';
import { timeTicks } from '../timeAxis';
import { readTokens, subscribeTokens } from '../tokens';
import { hideTooltip, showTooltip, tooltipStore } from '../tooltip';
import type { TooltipContent } from '../tooltip';
import { EMPTY_VIEW, reduceView } from '../view';
import type { ViewAction, ViewState } from '../view';
import { AXIS_HEIGHT, INDENT, ROW_HEIGHT, bandHalf, bandHitAt, barExtent, barHitAt, barHours, inSpans, laneAt, laneBandLayouts, laneBands, lanesInView, laneStops, leftColumn, nextHolder, spanAt } from './layout';
import type { CachedBands, HolderState } from './layout';
import type { TimelineLane, TimelineSpan } from './layout';
import { coversViewport, laneWindow, sizeLaneWindow, timelineHeight, timelineViewStyle, timelineViewport, timelineY } from './viewport';
import { barContent } from './barLabel';
import '../charts.css';

export type { BandStep, LaneGeometry, TimelineLane, TimelineSpan } from './layout';

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
  /** Taller lanes for a painted plot; every layout and interaction uses this height. */
  rowHeight?: number;
  /** Keep the dates in view while lanes scroll; false lets them scroll with the lanes. */
  stickyAxis?: boolean;
  /** Fade name or picture changes under one holder id; painted lanes default to this. */
  fadePresentation?: boolean;
  /** End long labels in an ellipsis; false keeps fillText's squeezing. */
  truncateLabels?: boolean;
  /** Align holder art by its top edge or centre; pills default to top alignment. */
  holderIconAlign?: 'top' | 'center';
  hoursPerSecond?: number;
  /** The value a band (`TimelineSpan.band`) is drawn full thickness at: the largest
   * any band reaches, so every lane shares one scale. */
  bandMax?: number;
  /** Bar thickness in layout pixels; bands retain their shared value scale. */
  barHeight?: number;
  /** `full` asks for every row, for text read out rather than shown. */
  tooltip?: (lane: TimelineLane, span: TimelineSpan | null, hour: number, full?: boolean) => TooltipContent | null;
  /** What the timeline shows at the moment shown, as text for keyboard and screen-reader users. */
  description?: string;
  onContextMenu?: (hour: number, clientX: number, clientY: number, lane?: TimelineLane, span?: TimelineSpan | null) => void;
  /** Opt into context menus in the label column; plot menus are unchanged. */
  labelMenu?: boolean;
  /** The lanes in view changed (first and last index, inclusive; last below first
   * when none), so a widget can ask for what only those lanes show, such as names
   * and flags at the playhead. */
  onLanesInView?: (first: number, last: number) => void;
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
/** How much the left column may squeeze a text before it ends it in an ellipsis. */
const SQUEEZE = 1.12;

/** The nearest ancestor that scrolls vertically, if any. */
function scrollParent(element: HTMLElement): HTMLElement | null {
  let parent = element.parentElement;
  while (parent && !/auto|scroll/.test(getComputedStyle(parent).overflowY)) parent = parent.parentElement;
  return parent;
}

/** The height a scrolling view shows inside its padding, in its own pixels. */
function viewHeight(parent: HTMLElement, rowHeight: number): number {
  const style = getComputedStyle(parent);
  return Math.max(AXIS_HEIGHT + rowHeight, parent.clientHeight - (parseFloat(style.paddingTop) || 0) - (parseFloat(style.paddingBottom) || 0));
}

/** A lane timeline: one lane per slot with bars for who held it and when, and a left
 * column naming each slot's holder at the playhead (the approved style-4 look). Time
 * zooms like the time chart: wheel, Shift/middle-drag pan, drag the dates to stretch,
 * double-click or Escape to reset. A full-height spacer keeps lane positions; the
 * canvas covers only visible lanes and overscan in their original coordinates. */
export default function Timeline(props: TimelineProps) {
  const owner = useId();
  const canvas = useRef<HTMLCanvasElement>(null);
  const spacer = useRef<HTMLDivElement>(null);
  const viewportElement = useRef<HTMLDivElement>(null);
  const offset = useRef(0);
  const region = useRef(laneWindow(props.lanes.length, 0, window.innerHeight, props.rowHeight ?? ROW_HEIGHT));
  const latest = useRef(props);
  latest.current = props;
  const [zoomed, setZoomed] = useState(false);
  const rowHeight = props.rowHeight ?? ROW_HEIGHT;
  const height = timelineHeight(props.lanes.length, rowHeight);
  const [viewportHeight, setViewportHeight] = useState(window.innerHeight);
  const viewportState = () => {
    const p = latest.current;
    return timelineViewport(spacer.current!, viewportElement.current!, p.lanes.length, p.rowHeight ?? ROW_HEIGHT, p.stickyAxis !== false);
  };
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
    let measured: readonly TimelineLane[] | null = null;
    let labelEmpty: string | undefined;
    let labelTruncate: boolean | undefined;
    let inView: [number, number] = [0, -1];
    const laneUnder = (y: number) => {
      const p = latest.current;
      const axisTop = p.stickyAxis !== false ? offset.current : 0;
      return y < axisTop + AXIS_HEIGHT ? null : laneAt(y, p.lanes.length, p.rowHeight ?? ROW_HEIGHT);
    };
    let pointer: { x: number; y: number; clientX: number; clientY: number } | null = null;
    let drag: { mode: 'axis'; grab: number } | { mode: 'pan'; lastX: number } | null = null;
    let zoomedFor = '';
    const holders = new Map<string, HolderState>();
    const images = new Map<string, HTMLImageElement>();
    // Each lane's bands as last drawn: hover and Jump to describe what is drawn under the pointer.
    // Rebuilt from the lanes shown at each draw (a lane gone is dropped), reusing a lane's
    // layout while its lane and view are unchanged, so pointer movement lays nothing out.
    let layouts = new Map<string, CachedBands>();
    // The span and hour under a pointer in the plot: a band's drawn column describes the span
    // and step it draws (`bandHitAt`), even a short band moved off its own hours; elsewhere
    // the span holding the pointer's hour.
    const pointed = (lane: TimelineLane, px: number, hour: number): { span: TimelineSpan | null; hour: number } => {
      if (lane.paint) return { span: spanAt(lane.spans, Math.floor(hour)), hour: Math.floor(hour) };
      const bands = layouts.get(lane.id)?.bands;
      const hit = bands && bandHitAt(bands, px, hour);
      if (hit) return { span: lane.spans.find(span => span.id === hit.spanId) ?? null, hour: hit.hour };
      // A plain bar by the rectangle drawn, which a short bar widens past its hours.
      const bar = barHitAt(lane.spans, px, latest.current.playhead, toX, hourAt, left, left + plotWidth());
      if (bar) return bar;
      return { span: spanAt(lane.spans, Math.floor(hour)), hour: Math.floor(hour) };
    };

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
    // Left-column text that fits `width` in the current font: a little squeezing is left to
    // fillText's maxWidth, past that the text ends in an ellipsis rather than being crushed.
    // Remembered per font, width and text, so a frame measures only what changed.
    const fits = new Map<string, string>();
    const fitText = (text: string, width: number): string => {
      const key = `${context.font}|${Math.round(width)}|${text}`;
      const known = fits.get(key);
      if (known != null) return known;
      const room = width * SQUEEZE;
      let result = text;
      if (context.measureText(text).width > room) {
        let low = 0;
        let high = text.length;
        while (low < high) {
          const middle = Math.ceil((low + high) / 2);
          if (context.measureText(`${text.slice(0, middle).trimEnd()}…`).width <= room) low = middle; else high = middle - 1;
        }
        result = `${text.slice(0, low).trimEnd()}…`;
      }
      if (fits.size > 4096) fits.clear();
      fits.set(key, result);
      return result;
    };
    // An icon `size` wide, centred on `cy`, so a flag's shorter image sits level with a square icon's.
    const drawIcon = (source: string | undefined, px: number, cy: number, size: number, alignTop = false) => {
      const loaded = image(source);
      if (!loaded) return;
      const height = size * loaded.naturalHeight / Math.max(1, loaded.naturalWidth);
      context.drawImage(loaded, px, alignTop ? cy - size / 2 : cy - height / 2, size, height);
    };

    const draw = (info: FrameInfo): boolean => {
      const p = latest.current;
      const row = p.rowHeight ?? ROW_HEIGHT;
      const { visible, offset: scrolled } = viewportState();
      offset.current = scrolled;
      const axisTop = p.stickyAxis !== false ? scrolled : 0;
      const viewport = laneWindow(p.lanes.length, visible.top, visible.height, row);
      region.current = viewport;
      const size = sizeLaneWindow(element, viewport, scrolled, window.devicePixelRatio || 1);
      if (!size) return false;
      const shown = p.lanes.slice(viewport.first, viewport.end);
      const { layoutWidth, layoutHeight } = size;
      context.setTransform(size.scaleX, 0, 0, size.scaleY, 0, 0);
      context.clearRect(0, 0, layoutWidth, layoutHeight);
      context.translate(0, -viewport.top);
      if (pointer) pointer = local(pointer);

      // Fit the campaign's names only when their text or presentation changes.
      const key = p.lanes === measured && p.emptyLabel === labelEmpty && p.truncateLabels === labelTruncate && labelKey ? labelKey
        : JSON.stringify([p.emptyLabel, p.truncateLabels, p.lanes.map(lane => [lane.depth, lane.heading, !!lane.pill,
          lane.label, lane.identity?.label, lane.spans.map(span => span.label)])]);
      measured = p.lanes;
      labelEmpty = p.emptyLabel;
      labelTruncate = p.truncateLabels;
      if (key !== labelKey) {
        labelKey = key;
        const texts = p.lanes.flatMap(lane => [lane.label, p.emptyLabel ?? 'Empty', ...(lane.identity ? [lane.identity.label] : []), ...lane.spans.map(span => span.label)]
          .map(text => [text, (lane.depth ?? 0) * INDENT, p.truncateLabels === false ? lane.heading ? 600 : 500 : lane.pill ? 500 : 600] as const));
        fitted = fitLabelWidth(texts, ([text, indent, weight]) => {
          context.font = font(tokens, tokens.fontSize.body, weight);
          return context.measureText(text).width + indent;
        }, 120, 280, ICON + 30);
      }
      left = leftColumn(fitted, layoutWidth);
      // Report clipped lanes, excluding the sticky axis and drawing's overscan.
      const plotTop = Math.max(visible.top, axisTop + AXIS_HEIGHT);
      const plotBottom = visible.top + visible.height;
      const visibleLanes: [number, number] = plotBottom > plotTop
        ? lanesInView(plotTop - AXIS_HEIGHT, plotBottom - plotTop + AXIS_HEIGHT, p.lanes.length, row) : [0, -1];
      if (visibleLanes[0] !== inView[0] || visibleLanes[1] !== inView[1]) {
        inView = visibleLanes;
        queueMicrotask(() => latest.current.onLanesInView?.(visibleLanes[0], visibleLanes[1]));
      }
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
      if (keyed.current && (viewed.current?.[0] !== target[0] || viewed.current?.[1] !== target[1])) queueMicrotask(() => refreshKeyed.current());
      viewed.current = target;
      eased = x;
      const bottom = AXIS_HEIGHT + p.lanes.length * row;
      const right = left + plotWidth();

      // Dates along the top, from the recording's own calendar.
      context.font = font(tokens, tokens.fontSize.small);
      const ticks = timeTicks(x[0], x[1], plotWidth(), text => context.measureText(text).width);
      const grid = (from: number, to: number) => {
        context.strokeStyle = mix(tokens.surface, tokens.border, 0.8);
        context.lineWidth = 1;
        for (const hour of ticks.grid) {
          const px = Math.round(toX(hour)) + 0.5;
          context.beginPath(); context.moveTo(px, from); context.lineTo(px, to); context.stroke();
        }
      };
      grid(axisTop + AXIS_HEIGHT - 4, bottom);

      let animating = false;
      // Each lane's bands, placed together so gaps between its spans stay visible too.
      layouts = laneBandLayouts(shown, `${x[0]}:${x[1]}:${left}:${right}:${p.playhead}:${p.bandMax}`, layouts,
        lane => laneBands(lane.spans, p.playhead, toX, value => bandHalf(value, p.bandMax ?? 1, 1, 12), left, right));
      const holderHour = p.playhead ?? p.extent[1];
      const unknown = inSpans(p.unobserved, Math.floor(holderHour));
      const kept = new Set(shown.map(lane => lane.id));
      for (const id of holders.keys()) if (!kept.has(id)) holders.delete(id);
      shown.forEach((lane, offset) => {
        const index = viewport.first + offset;
        const y = AXIS_HEIGHT + index * row;
        const centre = y + row / 2;
        const indent = (lane.depth ?? 0) * INDENT;
        if (index) { context.fillStyle = mix(tokens.surface, tokens.border, lane.heading ? 0.9 : 0.5); context.fillRect(8, y, layoutWidth - 16, 1); }
        // The holder at the playhead (or the end, off Live), cross-fading when it changes.
        // A lane about one thing names it whatever the hour holds.
        const current = lane.identity ? { id: lane.id, firstHour: 0, lastHour: 0, ...lane.identity }
          : unknown ? null : spanAt(lane.spans, holderHour);
        const state = nextHolder(holders.get(lane.id), current, p.fadePresentation ?? !!lane.paint);
        state.t = tokens.reducedMotion ? 1 : Math.min(1, state.t + info.dt / FADE_MS);
        if (state.t < 1) animating = true;
        holders.set(lane.id, state);
        // An ended lane's name and icon are faded.
        const fade = lane.muted ? 0.55 : 1;
        const blueprint = lane.identity?.tile === 'blueprint';
        // A heading with no icon of its own starts its text where an icon would be.
        const textAt = indent + (blueprint ? 62 : lane.heading && !lane.identity?.icon ? 10 : ICON + 18);
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
                context.fillStyle = tokens.blueprintText; context.fillText('?', 27, centre);
              }
            } else drawIcon(span.icon, 10 + indent, centre, ICON, p.holderIconAlign ? p.holderIconAlign === 'top' : !!lane.pill);
            context.fillStyle = tokens.text; context.font = font(tokens, tokens.fontSize.body, lane.heading ? 600 : 500);
            context.fillText(lane.pill ? span.label : labelText(span.label), textAt, centre - 7, textWidth);
          } else {
            context.setLineDash([3, 3]); context.strokeStyle = mix(tokens.surface, tokens.text, 0.3);
            context.beginPath(); context.roundRect(11.5 + indent, centre - 15.5, 30, 30, 5); context.stroke(); context.setLineDash([]);
            context.fillStyle = tokens.muted; context.font = font(tokens, tokens.fontSize.body);
            context.fillText(labelText(unknown ? 'Not recorded' : p.emptyLabel ?? 'Empty'), textAt, centre - 7, textWidth);
          }
          context.globalAlpha = 1;
        };
        if (state.t < 1) holder(state.previous, 1 - state.t);
        holder(state.current, state.t);
        context.globalAlpha = fade;
        context.font = font(tokens, tokens.fontSize.small, lane.pill ? 600 : 400);
        context.textAlign = 'left'; context.textBaseline = 'middle';
        if (lane.pill) {
          const width = Math.min(textWidth, context.measureText(lane.pill.text).width + 12);
          const pillTop = centre + 1;
          const pillHeight = tokens.fontSize.small + 5;
          context.fillStyle = lane.pill.color;
          context.beginPath(); context.roundRect(textAt, pillTop, width, pillHeight, pillHeight / 2); context.fill();
          const background = parseColor(lane.pill.color);
          context.fillStyle = background ? readableText(background) : tokens.text;
          context.fillText(lane.pill.text, textAt + 6, pillTop + pillHeight / 2 + 0.5, Math.max(1, width - 12));
        } else {
          context.fillStyle = tokens.muted;
          context.fillText(labelText(lane.label), textAt, centre + 8, textWidth);
        }
        context.globalAlpha = 1;

        if (lane.paint) {
          context.save();
          context.beginPath(); context.rect(left, y, plotWidth(), row); context.clip();
          lane.paint(context, { top: y, height: row, left, right, hours: x, toX, playhead: p.playhead }, tokens);
          context.restore();
          return;
        }
        context.save();
        context.beginPath(); context.rect(left, AXIS_HEIGHT, plotWidth() + 10, bottom - AXIS_HEIGHT); context.clip();
        const bands = layouts.get(lane.id)?.bands;
        for (const span of lane.spans) {
          const hours = barHours(span, p.playhead);
          if (!hours) continue;
          // A span's own colour (a faction, a subject level) comes first. A law keeps its
          // escalation shade even while held (signed off, mock-up politics-4); other held
          // spans are accented.
          const fill = span.color ?? (span.positive ? mix(tokens.surface, tokens.positive, 0.7) : span.shade != null ? mix(tokens.surface, tokens.accent, 0.3 + 0.6 * span.shade)
            : span.emphasis ? mix(tokens.surface, tokens.accent, 0.55) : mix(tokens.surface, tokens.text, 0.1));
          const background = parseColor(fill);
          const onFill = (span.color == null && span.shade == null && !span.emphasis) || !background ? tokens.text : readableText(background);
          context.fillStyle = fill;
          if (span.band) {
            // A band: its thickness follows the span's value, in whole pixel columns by the
            // kit's rule (`bandOutlines`), with no padding, so no recorded band vanishes.
            for (const group of bands?.outlines.get(span.id) ?? []) {
              context.beginPath();
              for (const edge of group) { context.lineTo(edge.x0, centre - edge.half); context.lineTo(edge.x1, centre - edge.half); }
              for (const edge of [...group].reverse()) { context.lineTo(edge.x1, centre + edge.half); context.lineTo(edge.x0, centre + edge.half); }
              context.closePath(); context.fill();
            }
            continue;
          }
          // Never narrower than a pixel column, however short the bar at this zoom.
          const extent = barExtent(toX(hours[0]), toX(hours[1]), left, right);
          if (!extent) continue;
          const [from, to] = extent;
          const barHeight = Math.min(rowHeight - 4, Math.max(2, p.barHeight ?? 24));
          context.beginPath(); context.roundRect(from, centre - barHeight / 2, to - from, barHeight, Math.min(5, barHeight / 2, (to - from) / 2)); context.fill();
          if (span.endLabel && to - from >= 16 && hours[1] >= span.lastHour + 1) {
            context.fillStyle = readableText(parseColor(fill) ?? parseColor(tokens.text)!);
            context.font = font(tokens, tokens.fontSize.small, 600);
            context.fillText(span.endLabel, to - 14, centre);
          }
          let textX = from + 7;
          const content = barContent(lane.icons ?? p.showIcons, lane.names ?? p.showNames, !!span.icon);
          if (content.icon && to - from > 28) { drawIcon(span.icon, from + 2, centre, BAR_ICON, p.holderIconAlign === 'top'); textX = from + BAR_ICON + 6; }
          context.font = font(tokens, tokens.fontSize.small, 600);
          if (span.glyph) {
            const width = context.measureText(span.glyph).width;
            if (width + (textX - from) + 4 < to - from) {
              context.fillStyle = onFill;
              context.fillText(span.glyph, textX - 2, centre);
              textX += width + 3;
            }
          }
          if (!content.name) continue;
          context.font = font(tokens, tokens.fontSize.small, 500);
          if (context.measureText(span.label).width + (textX - from) + 6 < to - from) {
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

      // Hover: the left column describes the lane's holder at the playhead; the plot, the span under the pointer.
      const index = pointer && !drag ? laneUnder(pointer.y) : null;
      // A tooltip opened from the keyboard wins over a pointer resting on the chart.
      if (pointer && index != null && p.tooltip && !keyed.current && !quiet.current) {
        const lane = p.lanes[index];
        const { span, hour } = pointer.x < left ? { span: spanAt(lane.spans, Math.floor(holderHour)), hour: Math.floor(holderHour) }
          : pointed(lane, pointer.x, Math.min(hourAt(pointer.x), p.playhead ?? Infinity));
        const content = p.tooltip(lane, span && span.firstHour <= (p.playhead ?? Infinity) ? span : null, hour);
        if (content) showTooltip(owner, content, pointer.clientX, pointer.clientY, `${lane.id}:${span?.id ?? ''}:${hour}`); else hideTooltip(owner);
      } else if (!keyed.current) hideTooltip(owner);
      const cursor = drag?.mode === 'pan' ? 'grabbing' : pointer && pointer.y < axisTop + AXIS_HEIGHT && pointer.x > left ? 'ew-resize' : 'default';
      if (element.style.cursor !== cursor) element.style.cursor = cursor;

      const settled = Math.abs(x[0] - target[0]) + Math.abs(x[1] - target[1]) < 1e-3;
      return !settled || animating || drag != null;
    };

    const current: FrameTask = { frame: draw };
    task.current = current;
    const wake = () => scheduler.wake(current);
    const remove = scheduler.add(current);
    const unsubscribe = subscribeTokens(next => { tokens = next; labelKey = ''; wake(); });
    const measureView = () => {
      const parent = scrollParent(spacer.current!);
      setViewportHeight(parent ? viewHeight(parent, latest.current.rowHeight ?? ROW_HEIGHT) : window.innerHeight);
    };
    const observer = new ResizeObserver(() => { measureView(); wake(); });
    observer.observe(element);
    observer.observe(viewportElement.current!);
    for (let parent: HTMLElement | null = spacer.current; parent; parent = parent.parentElement) observer.observe(parent);
    measureView();
    const scroll = () => {
      const p = latest.current;
      const { visible } = viewportState();
      if (p.active && (p.stickyAxis !== false || !coversViewport(region.current, p.lanes.length, visible, p.rowHeight ?? ROW_HEIGHT))) {
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
      if (point.x < left) return;
      if (event.button === 0 && point.y < (latest.current.stickyAxis !== false ? offset.current : 0) + AXIS_HEIGHT) drag = { mode: 'axis', grab: hourAt(point.x) };
      else if (event.button === 1 || (event.button === 0 && event.shiftKey)) drag = { mode: 'pan', lastX: point.x };
      else return;
      event.preventDefault();
      element.focus({ preventScroll: true });
      element.setPointerCapture(event.pointerId);
      wake();
    };
    const move = (event: PointerEvent) => {
      // Moving the pointer hands the tooltip back to it.
      keyed.current = null;
      quiet.current = false;
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
      const index = laneUnder(point.y);
      if (!handler || point.x < left && !latest.current.labelMenu || index == null) return;
      event.preventDefault();
      // Jump to what is drawn under the pointer: a band column's step, else the pointer's hour.
      const lane = latest.current.lanes[index];
      const target = point.x < left ? { hour: latest.current.playhead ?? latest.current.extent[1], span: null }
        : pointed(lane, point.x, Math.min(hourAt(point.x), latest.current.playhead ?? Infinity));
      handler(target.hour, event.clientX, event.clientY, lane, target.span);
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
      document.removeEventListener('scroll', scroll, true);
      window.removeEventListener('resize', scroll);
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
  useEffect(() => {
    const parent = scrollParent(spacer.current!);
    setViewportHeight(parent ? viewHeight(parent, rowHeight) : window.innerHeight);
  }, [rowHeight]);
  useEffect(() => { if (task.current) scheduler.wake(task.current); });

  // With the keyboard: Up and Down choose a lane, Left and Right step through its
  // bars (up to the playhead); the bar's tooltip shows beside the lane and is read out.
  // The lane is remembered by id, so lanes coming and going never move the selection.
  const keyed = useRef<{ lane: number; id: string; span: number; spanId?: string } | null>(null);
  const refreshKeyed = useRef<() => void>(() => {});
  // After Escape, no tooltip until the pointer moves again.
  const quiet = useRef(false);
  // The hours in view (zoom included), as last drawn.
  const viewed = useRef<readonly [number, number] | null>(null);
  // Where the keyboard can stop in a lane: each bar shown, and each step of a band, inside
  // the view and up to the playhead (`laneStops`, on the kit's `keyStops`).
  const barsOf = (lane: number) => {
    const p = latest.current;
    const [from, to] = viewed.current ?? p.extent;
    return laneStops(p.lanes[lane]?.spans ?? [], from, to, p.playhead);
  };
  const [spoken, setSpoken] = useState('');
  // Scrolls the nearest scrolling container so a keyboard-selected lane is in view.
  const revealLane = (lane: number) => {
    const element = spacer.current;
    if (!element) return;
    const bounds = element.getBoundingClientRect();
    const scale = element.offsetHeight ? bounds.height / element.offsetHeight : 1;
    const row = latest.current.rowHeight ?? ROW_HEIGHT;
    const top = bounds.top + (AXIS_HEIGHT + lane * row) * scale;
    const bottom = top + row * scale;
    const axis = latest.current.stickyAxis !== false ? AXIS_HEIGHT * scale : 0;
    let parent = element.parentElement;
    while (parent && !(parent.scrollHeight > parent.clientHeight && /auto|scroll/.test(getComputedStyle(parent).overflowY))) parent = parent.parentElement;
    if (!parent) {
      if (top < axis) window.scrollBy(0, top - axis);
      else if (bottom > window.innerHeight) window.scrollBy(0, bottom - window.innerHeight);
      return;
    }
    const box = parent.getBoundingClientRect();
    // Screen pixels to the container's own scroll units (a Size zoom scales them).
    const units = parent.clientHeight ? box.height / parent.clientHeight : 1;
    if (top < box.top + axis) parent.scrollTop -= (box.top + axis - top) / units;
    else if (bottom > box.bottom) parent.scrollTop += (bottom - box.bottom) / units;
  };
  const showKeyed = () => {
    const p = latest.current;
    const at = keyed.current;
    if (!at || !p.tooltip || !canvas.current) return;
    // A lane playback took away since is left: the tooltip closes.
    at.lane = p.lanes.findIndex(item => item.id === at.id);
    const lane = p.lanes[at.lane];
    if (!lane) { keyed.current = null; hideTooltip(owner); setSpoken(''); return; }
    const bars = barsOf(at.lane);
    // The selected stop is followed by id; one zoomed out of view gives way to the nearest in view.
    const same = at.spanId == null ? -1 : bars.findIndex(bar => bar.id === at.spanId);
    at.span = same >= 0 ? same : Math.max(0, Math.min(at.span, bars.length - 1));
    at.spanId = bars[at.span]?.id;
    const stop = bars[at.span] ?? null;
    const span = stop?.span ?? null;
    // With no bar in view, the lane is described at an hour in view, never one outside it.
    const [viewFrom, viewTo] = viewed.current ?? p.extent;
    // A view lying after the playback moment shows nothing yet: nothing is described.
    if (!span && p.playhead != null && viewFrom > p.playhead) { hideTooltip(owner); setSpoken(`${lane.label}: not reached yet`); return; }
    // A stop is described at its whole hour, as hover floors the pointer's, so a band's
    // stop gives the value of its step.
    const hour = stop ? Math.floor(stop.hour) : Math.floor(Math.max(viewFrom, Math.min(p.playhead ?? viewTo, viewTo)));
    // With no bar reachable, the lane's holder at that hour (as hover resolves it), not an empty slot.
    const holder = span ?? spanAt(lane.spans, hour);
    const shown = holder && holder.firstHour <= (p.playhead ?? Infinity) ? holder : null;
    const content = p.tooltip(lane, shown, hour);
    if (!content) { hideTooltip(owner); setSpoken(''); return; }
    const bounds = spacer.current!.getBoundingClientRect();
    // Layout pixels to screen pixels: the widget's Size zoom scales the canvas.
    const scale = spacer.current!.offsetHeight ? bounds.height / spacer.current!.offsetHeight : 1;
    showTooltip(owner, content, bounds.left + bounds.width / 2, bounds.top + (AXIS_HEIGHT + (at.lane + 1) * (p.rowHeight ?? ROW_HEIGHT)) * scale, `key:${lane.id}:${stop?.id ?? shown?.id ?? ''}`);
    // Read out in full: every row, not only those the window fits.
    const full = p.tooltip(lane, shown, hour, true) ?? content;
    setSpoken([full.title, full.subtitle, ...full.rows.map(row => row.value ? `${row.label} ${row.value}` : row.label), full.description, full.note]
      .filter(Boolean).join('; '));
  };
  // Only while no other chart holds the shared tooltip: one hovered since keeps it. A hidden
  // tooltip (such as one withheld until playback reaches the view) is free to take again.
  const ownsTooltip = () => { const current = tooltipStore.get(); return current == null || current.owner === owner; };
  refreshKeyed.current = () => { if (keyed.current && latest.current.active && ownsTooltip()) showKeyed(); };
  // As playback moves, a keyboard tooltip follows what its lane shows now; an
  // off-screen timeline drops it.
  useEffect(() => {
    if (!keyed.current) return;
    if (!props.active) { keyed.current = null; setSpoken(''); return; }
    if (ownsTooltip()) showKeyed();
  });
  const keyDown = (event: ReactKeyboardEvent) => {
    const lanes = latest.current.lanes;
    const bars = (lane: number) => barsOf(lane).length;
    if (['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight'].includes(event.key) && lanes.length) {
      event.preventDefault();
      // The remembered lane is found by id; one that has gone starts over at the first lane.
      const remembered = keyed.current && lanes.findIndex(lane => lane.id === keyed.current!.id);
      if (keyed.current && remembered != null && remembered >= 0) keyed.current.lane = remembered;
      else keyed.current = null;
      const at = keyed.current ?? { lane: 0, id: lanes[0].id, span: Math.max(0, bars(0) - 1) };
      if (keyed.current) {
        if (event.key === 'ArrowDown') { at.lane = Math.min(lanes.length - 1, at.lane + 1); at.span = Math.max(0, bars(at.lane) - 1); }
        if (event.key === 'ArrowUp') { at.lane = Math.max(0, at.lane - 1); at.span = Math.max(0, bars(at.lane) - 1); }
        if (event.key === 'ArrowLeft') at.span = Math.max(0, at.span - 1);
        if (event.key === 'ArrowRight') at.span = Math.min(Math.max(0, bars(at.lane) - 1), at.span + 1);
      }
      at.id = lanes[at.lane].id;
      // A key press chooses the bar by position; the id then follows it through zoom.
      at.spanId = undefined;
      keyed.current = at;
      revealLane(at.lane);
      showKeyed();
      return;
    }
    if (event.key === 'Escape' && keyed.current) { keyed.current = null; quiet.current = true; hideTooltip(owner); setSpoken(''); return; }
    if (event.key === 'Escape' && zoomed) { event.preventDefault(); controls.current?.dispatch({ type: 'reset' }); }
    // The keyboard's Jump to: the selected stop's hour, else the playhead.
    const selected = keyed.current ? barsOf(keyed.current.lane)[keyed.current.span] : undefined;
    const menuHour = selected ? selected.hour : props.playhead;
    if ((event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) && props.onContextMenu && menuHour != null) {
      event.preventDefault();
      const bounds = spacer.current!.getBoundingClientRect();
      const scale = spacer.current!.clientHeight ? bounds.height / spacer.current!.clientHeight : 1;
      const { visible } = viewportState();
      const lane = keyed.current ? lanes[keyed.current.lane] : undefined;
      const span = selected?.span ?? null;
      props.onContextMenu(Math.floor(menuHour), bounds.left + bounds.width / 2, bounds.top + (visible.top + visible.height / 2) * scale, lane, span);
    }
  };
  return <div ref={spacer} className="timeline-chart" style={{ height }}>
    <div ref={viewportElement} className="timeline-view" style={timelineViewStyle(height, viewportHeight, props.stickyAxis !== false)}>
      <canvas ref={canvas} tabIndex={0} role="img" aria-label={props.label} aria-describedby={props.description ? `${owner}-description` : undefined}
        aria-keyshortcuts="ArrowUp ArrowDown ArrowLeft ArrowRight" onKeyDown={keyDown} onBlur={() => { keyed.current = null; hideTooltip(owner); setSpoken(''); }}
        style={{ position: 'absolute', top: 0, height: region.current.height }} />
      {zoomed && <button type="button" className="button small time-chart-reset" onClick={() => controls.current?.dispatch({ type: 'reset' })}>
        <Icon name="refresh" />Reset zoom</button>}
    </div>
    <p className="sr-only" aria-live="polite">{spoken}</p>
    {props.description && <p id={`${owner}-description`} className="sr-only">{props.description}</p>}
  </div>;
}
