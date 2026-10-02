import { useEffect, useId, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent } from 'react';
import { mix } from '../../shell/appearance/color';
import { font } from '../draw';
import { easeTau } from '../easing';
import { canvasImage } from '../images';
import { layoutEndLabels } from '../labels';
import { scheduler } from '../scheduler';
import type { FrameInfo, FrameTask } from '../scheduler';
import { backingSize, toLocal } from '../surface';
import { readTokens, subscribeTokens } from '../tokens';
import { hideTooltip, showTooltip, tooltipStore, tooltipText } from '../tooltip';
import type { TooltipContent } from '../tooltip';
import { flowCapacity, FlowMotion, flowStops, linkAt, nodeAt } from './layout';
import type { FlowFrame, FlowLink, FlowNode, FlowStop, LinkShape, NodeBox } from './layout';
import { FlowTooltipCache } from './tooltipCache';
import '../charts.css';

export type { FlowLink, FlowNode } from './layout';

/** What the pointer or the keyboard is on. */
export type FlowHit = { kind: 'node'; node: FlowNode } | { kind: 'link'; link: FlowLink };

export interface FlowDiagramProps {
  /** Names the diagram for screen readers. */
  label: string;
  nodes: readonly FlowNode[];
  links: readonly FlowLink[];
  /** Changes whenever what is shown may not glide from what was drawn (across a gap
   * in the recording, say): the next frame jumps there. */
  stretch: unknown;
  /** The value that fills the height (one scale for a whole campaign); null fits
   * what is shown now. */
  scale?: number | null;
  /** Node slots per side the height is shared between, so the scale holds as nodes
   * come and go. */
  slots?: number;
  hoursPerSecond?: number;
  /** A line drawn over the diagram, such as why nothing shows. */
  message?: string | null;
  /** The tooltip for a node or ribbon; its `spoken` form, when trimmed, is read out.
   * Memoize with every input it reads: a changed callback refreshes a still hover. */
  tooltip?: (hit: FlowHit) => TooltipContent | null;
  onContextMenu?: (hit: FlowHit | null, clientX: number, clientY: number) => void;
  /** What the diagram shows, as text for screen readers. */
  description?: string;
  active: boolean;
  /** Told how many nodes a side can show at the diagram's size (`flowCapacity`) when
   * that changes: the widget folds the rest into one node. */
  onCapacity?: (perSide: number) => void;
}

const NODE_WIDTH = 12;
const PAD = 8;
const FLAG_WIDTH = 18;
const FLAG_HEIGHT = 12;
const LABEL_GAP = 6;
/** Ribbon opacity at rest, highlighted and dimmed (another ribbon hovered). */
const RIBBON_ALPHA = 0.55;
const RIBBON_HOT = 0.8;
const RIBBON_DIM = 0.12;

/** A Sankey of sources (left) to targets (right) on Canvas 2D (mock-up style-6):
 * ribbons sized by value that grow, shrink, appear and vanish as playback moves,
 * nodes that glide to their new places, and inside each node a fixed ribbon order,
 * so nothing jumps. Hovering a ribbon or a node dims the rest and shows its tooltip,
 * live while playback runs; right-click (or the context-menu key) asks the widget
 * for its menu. With the keyboard, the arrow keys step through each source node and
 * its ribbons, then the target nodes (Home and End go to either end, Escape leaves),
 * showing each tooltip and reading it out; Enter or the context-menu key opens the
 * menu for the stop. */
export default function FlowDiagram(props: FlowDiagramProps) {
  const owner = useId();
  const canvas = useRef<HTMLCanvasElement>(null);
  const latest = useRef(props);
  latest.current = props;
  const task = useRef<FrameTask | null>(null);
  const drawn = useRef<FlowFrame>({ nodes: [], links: [], settled: true });
  const keyed = useRef<FlowStop | null>(null);
  // After Escape, no tooltip until the pointer moves again.
  const quiet = useRef(false);
  const [spoken, setSpoken] = useState('');
  const showKeyed = useRef<() => void>(() => {});

  useEffect(() => {
    const element = canvas.current!;
    const context = element.getContext('2d')!;
    let tokens = readTokens();
    const motion = new FlowMotion();
    let pointer: { x: number; y: number; clientX: number; clientY: number } | null = null;
    let stretch: unknown = undefined;
    let labelWidth = 0;
    const tooltips = new FlowTooltipCache<FlowHit>();
    let capacity = 0;

    const hitAt = (x: number, y: number): FlowHit | null => {
      const frame = drawn.current;
      const box = nodeAt(frame.nodes, x, y);
      if (box) return { kind: 'node', node: box.node };
      const shape = linkAt(frame.links, x, y);
      return shape ? { kind: 'link', link: shape.link } : null;
    };
    const hitId = (hit: FlowHit | null) => hit ? `${hit.kind}:${hit.kind === 'node' ? hit.node.id : hit.link.id}` : '';
    const describe = (hit: FlowHit, clientX: number, clientY: number) => {
      const p = latest.current;
      const { content, key } = tooltips.describe(hitId(hit), hit, p.tooltip);
      if (content) showTooltip(owner, content, clientX, clientY, key); else hideTooltip(owner);
    };

    const draw = (info: FrameInfo): boolean => {
      const p = latest.current;
      const width = element.clientWidth;
      const height = element.clientHeight;
      if (!width || !height) return false;
      const rect = element.getBoundingClientRect();
      const size = backingSize({ layoutWidth: width, layoutHeight: height, rectWidth: rect.width, rectHeight: rect.height, devicePixelRatio: window.devicePixelRatio || 1 });
      if (element.width !== size.width) element.width = size.width;
      if (element.height !== size.height) element.height = size.height;
      context.setTransform(size.scaleX, 0, 0, size.scaleY, 0, 0);
      context.clearRect(0, 0, width, height);

      // Label columns as wide as the longest name shown so far, so they never jump
      // back and forth during playback; at most a third of the width each side.
      context.font = font(tokens, tokens.fontSize.body, 500);
      for (const node of p.nodes) labelWidth = Math.max(labelWidth, context.measureText(node.label).width + FLAG_WIDTH + LABEL_GAP * 2);
      const column = Math.min(labelWidth, width / 3);
      const geometry = { left: PAD + column + NODE_WIDTH, right: width - PAD - column - NODE_WIDTH, top: PAD, bottom: height - PAD,
        nodeWidth: NODE_WIDTH, gap: Math.max(2, Math.min(10, height / 40)), slots: p.slots, scale: p.scale };
      // How many nodes a side can show with a label each; the widget folds the rest.
      const fits = flowCapacity(geometry.bottom - geometry.top, tokens.fontSize.body + 4);
      if (fits !== capacity) { capacity = fits; queueMicrotask(() => latest.current.onCapacity?.(fits)); }
      const snap = stretch !== p.stretch || tokens.reducedMotion;
      stretch = p.stretch;
      const frame = motion.step({ nodes: p.nodes, links: p.links }, geometry, info.dt, easeTau(tokens.motion, p.hoursPerSecond ?? 24, tokens.reducedMotion), snap);
      drawn.current = frame;

      // What is hot: the keyboard's stop, else what is under the pointer.
      const keyStop = keyed.current;
      const hit = keyStop ? (keyStop.kind === 'node' ? frame.nodes.find(box => box.id === keyStop.id)?.node && { kind: 'node' as const, node: frame.nodes.find(box => box.id === keyStop.id)!.node }
        : frame.links.find(shape => shape.id === keyStop.id) && { kind: 'link' as const, link: frame.links.find(shape => shape.id === keyStop.id)!.link }) ?? null
        : pointer && !quiet.current ? hitAt(pointer.x, pointer.y) : null;
      const hot = (shape: LinkShape) => !hit ? null : hit.kind === 'link' ? shape.id === hit.link.id : shape.link.source === hit.node.id || shape.link.target === hit.node.id;
      const ribbons = [...frame.links].sort((a, b) => Number(hot(a) === true) - Number(hot(b) === true));
      for (const shape of ribbons) {
        if (shape.width <= 0) continue;
        const state = hot(shape);
        context.globalAlpha = (state == null ? RIBBON_ALPHA : state ? RIBBON_HOT : RIBBON_DIM) * shape.alpha;
        context.fillStyle = shape.link.color;
        traceRibbon(context, shape);
        context.fill();
      }
      context.globalAlpha = 1;

      // Nodes, then their flags and names, placed apart with a leader line where moved.
      for (const box of frame.nodes) {
        if (box.height <= 0 || box.alpha <= 0) continue;
        context.globalAlpha = box.alpha;
        context.fillStyle = box.node.color;
        context.beginPath(); context.roundRect(box.x, box.y, box.width, Math.max(1, box.height), Math.min(3, box.height / 2)); context.fill();
      }
      context.globalAlpha = 1;
      const lineHeight = tokens.fontSize.body + 4;
      for (const side of ['source', 'target'] as const) {
        const boxes = frame.nodes.filter(box => box.node.side === side && box.alpha > 0.02 && box.height > 0);
        const placed = layoutEndLabels(boxes.map(box => ({ id: box.id, y: box.y + box.height / 2, height: lineHeight })), geometry.top, geometry.bottom);
        for (const box of boxes) {
          const centre = box.y + box.height / 2;
          const at = placed.get(box.id) ?? centre;
          const dim = hit && hit.kind === 'node' && hit.node.id !== box.id ? 0.55 : 1;
          context.globalAlpha = box.alpha * dim;
          const edge = side === 'source' ? box.x - LABEL_GAP : box.x + box.width + LABEL_GAP;
          if (Math.abs(at - centre) > 2) {
            context.strokeStyle = mix(tokens.surface, tokens.text, 0.35); context.lineWidth = 1;
            context.beginPath(); context.moveTo(side === 'source' ? box.x - 1 : box.x + box.width + 1, centre); context.lineTo(edge, at); context.stroke();
          }
          const flag = box.node.flag ? canvasImage(box.node.flag, wake) : null;
          const flagX = side === 'source' ? edge - FLAG_WIDTH : edge;
          if (flag) context.drawImage(flag, flagX, at - FLAG_HEIGHT / 2, FLAG_WIDTH, FLAG_HEIGHT);
          else if (box.node.flag !== undefined) {
            // A flag not (yet) known keeps its place, outlined, never silently blank.
            context.strokeStyle = mix(tokens.surface, tokens.text, 0.35); context.lineWidth = 1;
            context.setLineDash([2, 2]);
            context.strokeRect(flagX + 0.5, at - FLAG_HEIGHT / 2 + 0.5, FLAG_WIDTH - 1, FLAG_HEIGHT - 1);
            context.setLineDash([]);
          }
          context.fillStyle = tokens.text; context.font = font(tokens, tokens.fontSize.body, 500); context.textBaseline = 'middle';
          context.textAlign = side === 'source' ? 'right' : 'left';
          const room = Math.max(8, column - FLAG_WIDTH - LABEL_GAP * 2);
          context.fillText(box.node.label, side === 'source' ? flagX - LABEL_GAP / 2 : flagX + FLAG_WIDTH + LABEL_GAP / 2, at, room);
        }
      }
      context.globalAlpha = 1;
      if (p.message) {
        context.fillStyle = tokens.muted; context.font = font(tokens, tokens.fontSize.body); context.textAlign = 'center'; context.textBaseline = 'middle';
        context.fillText(p.message, width / 2, height / 2, Math.max(40, width - PAD * 4));
      }

      // Live hover: what is under a still pointer is described again every frame.
      if (!keyStop) {
        if (hit && pointer) describe(hit, pointer.clientX, pointer.clientY);
        else { tooltips.clear(); hideTooltip(owner); }
      } else if (p.tooltip !== keyedTooltip) { keyedTooltip = p.tooltip; queueMicrotask(() => showKeyed.current()); }
      element.style.cursor = hit ? 'pointer' : 'default';
      return !frame.settled;
    };
    let keyedTooltip: FlowDiagramProps['tooltip'];

    const current: FrameTask = { frame: draw };
    task.current = current;
    const wake = () => scheduler.wake(current);
    const remove = scheduler.add(current);
    const unsubscribe = subscribeTokens(next => { tokens = next; labelWidth = 0; wake(); });
    const observer = new ResizeObserver(() => wake());
    observer.observe(element);
    const local = (event: PointerEvent | MouseEvent) => ({ ...toLocal(event.clientX, event.clientY, element.getBoundingClientRect(), element.clientWidth, element.clientHeight), clientX: event.clientX, clientY: event.clientY });
    const move = (event: PointerEvent) => {
      // Moving the pointer hands the tooltip back to it.
      if (keyed.current) { keyed.current = null; setSpoken(''); }
      quiet.current = false;
      pointer = local(event);
      wake();
    };
    const leave = () => { pointer = null; tooltips.clear(); hideTooltip(owner); wake(); };
    const menu = (event: MouseEvent) => {
      const handler = latest.current.onContextMenu;
      if (!handler) return;
      event.preventDefault();
      const point = local(event);
      handler(hitAt(point.x, point.y), event.clientX, event.clientY);
    };
    element.addEventListener('pointermove', move);
    element.addEventListener('pointerleave', leave);
    element.addEventListener('contextmenu', menu);
    return () => {
      remove(); unsubscribe(); observer.disconnect(); hideTooltip(owner);
      element.removeEventListener('pointermove', move);
      element.removeEventListener('pointerleave', leave);
      element.removeEventListener('contextmenu', menu);
      task.current = null;
    };
  }, [owner]);

  useEffect(() => {
    if (task.current) scheduler.setVisible(task.current, props.active);
    if (!props.active) { hideTooltip(owner); keyed.current = null; setSpoken(''); }
  }, [props.active, owner]);
  useEffect(() => { if (task.current) scheduler.wake(task.current); });

  // The keyboard's stop: its tooltip beside it, read out in full.
  const stopHit = (stop: FlowStop): { hit: FlowHit; x: number; y: number } | null => {
    const frame = drawn.current;
    if (stop.kind === 'node') {
      const box = frame.nodes.find(item => item.id === stop.id);
      return box ? { hit: { kind: 'node', node: box.node }, x: box.node.side === 'source' ? box.x : box.x + box.width, y: box.y + box.height / 2 } : null;
    }
    const shape = frame.links.find(item => item.id === stop.id);
    return shape ? { hit: { kind: 'link', link: shape.link }, x: (shape.x0 + shape.x1) / 2, y: (shape.y0 + shape.y1 + shape.width) / 2 } : null;
  };
  // Only while no other chart holds the shared tooltip.
  const ownsTooltip = () => { const held = tooltipStore.get(); return held == null || held.owner === owner; };
  showKeyed.current = () => {
    const stop = keyed.current;
    const element = canvas.current;
    if (!stop || !element || !latest.current.active || !ownsTooltip()) return;
    const found = stopHit(stop);
    // A stop playback took away is left: the tooltip closes.
    if (!found) { keyed.current = null; hideTooltip(owner); setSpoken(''); return; }
    const content = latest.current.tooltip?.(found.hit) ?? null;
    if (!content) { hideTooltip(owner); setSpoken(''); return; }
    const bounds = element.getBoundingClientRect();
    const scale = element.offsetWidth ? bounds.width / element.offsetWidth : 1;
    showTooltip(owner, content, bounds.left + found.x * scale, bounds.top + found.y * scale, `key:${stop.kind}:${stop.id}`);
    setSpoken(tooltipText(content));
  };
  const keyDown = (event: ReactKeyboardEvent) => {
    const stops = flowStops(drawn.current);
    const index = keyed.current ? stops.findIndex(stop => stop.kind === keyed.current!.kind && stop.id === keyed.current!.id) : -1;
    const go = (next: number) => {
      event.preventDefault();
      if (!stops.length) return;
      keyed.current = stops[Math.max(0, Math.min(stops.length - 1, next))];
      quiet.current = false;
      showKeyed.current();
      if (task.current) scheduler.wake(task.current);
    };
    if (event.key === 'ArrowDown' || event.key === 'ArrowRight') return go(index < 0 ? 0 : index + 1);
    if (event.key === 'ArrowUp' || event.key === 'ArrowLeft') return go(index < 0 ? stops.length - 1 : index - 1);
    if (event.key === 'Home') return go(0);
    if (event.key === 'End') return go(stops.length - 1);
    if (event.key === 'Escape' && keyed.current) {
      keyed.current = null; quiet.current = true; hideTooltip(owner); setSpoken('');
      if (task.current) scheduler.wake(task.current);
      return;
    }
    // The context-menu key and Enter act on the stop, as on the time chart.
    if ((event.key === 'ContextMenu' || event.key === 'Enter' || (event.key === 'F10' && event.shiftKey)) && props.onContextMenu) {
      event.preventDefault();
      const found = keyed.current ? stopHit(keyed.current) : null;
      const bounds = canvas.current!.getBoundingClientRect();
      const scale = canvas.current!.offsetWidth ? bounds.width / canvas.current!.offsetWidth : 1;
      props.onContextMenu(found?.hit ?? null, found ? bounds.left + found.x * scale : bounds.left + bounds.width / 2, found ? bounds.top + found.y * scale : bounds.top + bounds.height / 2);
    }
  };
  return <div className="flow-diagram">
    <canvas ref={canvas} tabIndex={0} role="img" aria-label={props.label} aria-describedby={props.description ? `${owner}-description` : undefined}
      aria-keyshortcuts="ArrowUp ArrowDown ArrowLeft ArrowRight Home End Escape" onKeyDown={keyDown}
      onBlur={() => { keyed.current = null; hideTooltip(owner); setSpoken(''); }} />
    <p className="sr-only" aria-live="polite">{spoken}</p>
    {props.description && <p id={`${owner}-description`} className="sr-only">{props.description}</p>}
  </div>;
}

/** A ribbon's outline: its upper edge from source to target along the curve
 * `ribbonTop` measures, down its width, and back along the lower edge. */
export function traceRibbon(context: CanvasRenderingContext2D, shape: Pick<LinkShape, 'x0' | 'x1' | 'y0' | 'y1' | 'width'>): void {
  const middle = (shape.x0 + shape.x1) / 2;
  const width = Math.max(0.75, shape.width);
  context.beginPath();
  context.moveTo(shape.x0, shape.y0);
  context.bezierCurveTo(middle, shape.y0, middle, shape.y1, shape.x1, shape.y1);
  context.lineTo(shape.x1, shape.y1 + width);
  context.bezierCurveTo(middle, shape.y1 + width, middle, shape.y0 + width, shape.x0, shape.y0 + width);
  context.closePath();
}

export type { NodeBox, LinkShape };
