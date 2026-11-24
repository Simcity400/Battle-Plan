import { useEffect, useId, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent } from 'react';
import { mix } from '../../shell/appearance/color';
import { font } from '../draw';
import { easeTau } from '../easing';
import { useMotionNow } from '../liveMotion.ts';
import { canvasImage } from '../images';
import { countryTooltipContent, drawCountryFlag } from '../countryFlag';
import { fitLabels, layoutEndLabels } from '../labels';
import type { Live } from '../live';
import { scheduler } from '../scheduler';
import type { FrameInfo, FrameTask } from '../scheduler';
import { drawingSurface, sizeCanvas, toLocal, watchCanvasSize } from '../surface';
import { readTokens, subscribeTokens } from '../tokens';
import { hideTooltip, scrollTooltip, showTooltip, tooltipStore, tooltipText } from '../tooltip';
import type { TooltipContent } from '../tooltip';
import { flowCapacity, flowPlaybackSnap, FlowMotion, flowStops, labelAt, linkAt, nodeAt } from './layout';
import type { FlowFrame, FlowLink, FlowNode, FlowStop, LabelBox, LinkShape, NodeBox } from './layout';
import { FlowTooltipCache } from './tooltipCache';
import '../charts.css';
import { textWidth } from '../textWidth.ts';
import { setStyle } from '../domText.ts';

export type { FlowLink, FlowNode } from './layout';

export type FlowHit = { kind: 'node'; node: FlowNode } | { kind: 'link'; link: FlowLink };

export interface FlowDiagramProps {
  label: string;
  nodes: readonly FlowNode[];
  links: readonly FlowLink[];
  stretch: unknown;
  scale?: number | null;
  slots?: number;
  hoursPerSecond?: number;
  playbackHour?: number;
  message?: string | null;
  tooltip?: (hit: FlowHit) => TooltipContent | null;
  hour?: number;
  onPick?: (hit: FlowHit, hour: number) => void;
  onLabelPick?: (node: FlowNode, hour: number) => void;
  onContextMenu?: (hit: FlowHit | null, clientX: number, clientY: number) => void;
  description?: string;
  active: boolean;
  onCapacity?: (perSide: number) => void;
  live?: Live<FlowValues>;
}

export type FlowValues = Pick<FlowDiagramProps, 'nodes' | 'links' | 'hour' | 'playbackHour' | 'message'>;

const withLive = (props: FlowDiagramProps): FlowDiagramProps => props.live ? { ...props, ...props.live.read() } : props;

const NODE_WIDTH = 12;
const PAD = 8;
const FLAG_WIDTH = 18;
const FLAG_HEIGHT = 12;
const LABEL_GAP = 6;
const RIBBON_ALPHA = 0.55;
const RIBBON_HOT = 0.8;
const RIBBON_DIM = 0.12;

export default function FlowDiagram(props: FlowDiagramProps) {
  const owner = useId();
  const canvas = useRef<HTMLCanvasElement>(null);
  const latest = useRef(props);
  latest.current = props;
  const motionNow = useMotionNow();
  const task = useRef<FrameTask | null>(null);
  const drawn = useRef<FlowFrame>({ nodes: [], links: [], settled: true });
  const drawnLabels = useRef<LabelBox[]>([]);
  const drawnHour = useRef<number | null>(null);
  const keyed = useRef<FlowStop | null>(null);
  const quiet = useRef(false);
  const [spoken, setSpoken] = useState('');
  const showKeyed = useRef<(explicit?: boolean) => void>(() => {});
  const flagLoaded = useRef(() => { if (task.current) scheduler.wake(task.current); showKeyed.current(); });

  useEffect(() => {
    const element = canvas.current!;
    const paper = drawingSurface(element), context = paper.context!;
    const sized = watchCanvasSize(element, () => { if (task.current) scheduler.wake(task.current); });
    let tokens = readTokens();
    const motion = new FlowMotion();
    let pointer: { x: number; y: number; clientX: number; clientY: number } | null = null;
    let stretch: unknown = undefined;
    let playbackHour: number | undefined;
    let labelWidth = 0;
    const tooltips = new FlowTooltipCache<FlowHit>();
    let capacity = 0;

    const labelHit = (x: number, y: number) => latest.current.onLabelPick ? labelAt(drawnLabels.current, x, y) : null;
    const hitAt = (x: number, y: number): FlowHit | null => {
      const frame = drawn.current;
      const box = nodeAt(frame.nodes, x, y);
      if (box) return { kind: 'node', node: box.node };
      const label = labelHit(x, y);
      if (label) return { kind: 'node', node: label.node };
      const shape = linkAt(frame.links, x, y);
      return shape ? { kind: 'link', link: shape.link } : null;
    };
    const hitId = (hit: FlowHit | null) => hit ? `${hit.kind}:${hit.kind === 'node' ? hit.node.id : hit.link.id}` : '';
    const describe = (hit: FlowHit, clientX: number, clientY: number) => {
      const p = latest.current;
      const described = tooltips.describe(hitId(hit), hit, p.tooltip);
      const flag = hit.kind === 'node' ? hit.node.countryFlag : undefined;
      const content = countryTooltipContent(described.content, flag, wake, globalThis.innerHeight);
      const key = described.key;
      if (content) showTooltip(owner, content, clientX, clientY, key); else hideTooltip(owner);
    };

    const draw = (info: FrameInfo): boolean => {
      const p = withLive(latest.current);
      const { layoutWidth: width, layoutHeight: height, backing: size } = sized.read();
      if (!width || !height) return false;
      sizeCanvas(paper.canvas, size.width, size.height);
      context.setTransform(size.scaleX, 0, 0, size.scaleY, 0, 0);
      context.clearRect(0, 0, width, height);

      context.font = font(tokens, tokens.fontSize.body, 500);
      for (const node of p.nodes) labelWidth = Math.max(labelWidth, textWidth(context, node.label) + FLAG_WIDTH + LABEL_GAP * 2);
      const column = Math.min(labelWidth, width / 3);
      const geometry = { left: PAD + column + NODE_WIDTH, right: width - PAD - column - NODE_WIDTH, top: PAD, bottom: height - PAD,
        nodeWidth: NODE_WIDTH, gap: Math.max(2, Math.min(10, height / 40)), slots: p.slots, scale: p.scale };
      const fits = flowCapacity(geometry.bottom - geometry.top, tokens.fontSize.body + 4);
      if (fits !== capacity) { capacity = fits; queueMicrotask(() => latest.current.onCapacity?.(fits)); }
      const snap = stretch !== p.stretch || tokens.reducedMotion || flowPlaybackSnap(playbackHour, p.playbackHour);
      stretch = p.stretch;
      playbackHour = p.playbackHour;
      const frame = motion.step({ nodes: p.nodes, links: p.links }, geometry, info.dt, easeTau(tokens.motion, motionNow(p.hoursPerSecond), tokens.reducedMotion), snap);
      drawn.current = frame;
      drawnHour.current = Number.isSafeInteger(p.hour) ? p.hour! : null;

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

      for (const box of frame.nodes) {
        if (box.height <= 0 || box.alpha <= 0) continue;
        context.globalAlpha = box.alpha;
        context.fillStyle = box.node.color;
        context.beginPath(); context.roundRect(box.x, box.y, box.width, Math.max(1, box.height), Math.min(3, box.height / 2)); context.fill();
      }
      context.globalAlpha = 1;
      const lineHeight = tokens.fontSize.body + 4;
      const labels: LabelBox[] = [];
      for (const side of ['source', 'target'] as const) {
        const boxes = frame.nodes.filter(box => box.node.side === side && box.alpha > 0.02 && box.height > 0);
        const placed = layoutEndLabels(boxes.map(box => ({ id: box.id, y: box.y + box.height / 2, height: lineHeight })), geometry.top, geometry.bottom);
        const room = Math.max(8, column - FLAG_WIDTH - LABEL_GAP * 2);
        context.font = font(tokens, tokens.fontSize.body, 500);
        const fittedLabels = fitLabels(boxes.map(box => box.node.label), room, text => textWidth(context, text));
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
          const flagX = side === 'source' ? edge - FLAG_WIDTH : edge;
          if (box.node.countryFlag) drawCountryFlag(context, box.node.countryFlag, flagX, at - FLAG_HEIGHT / 2, FLAG_WIDTH, FLAG_HEIGHT, tokens, wake);
          else {
            const flag = box.node.flag ? canvasImage(box.node.flag, wake) : null;
            if (flag) context.drawImage(flag, flagX, at - FLAG_HEIGHT / 2, FLAG_WIDTH, FLAG_HEIGHT);
            else if (box.node.flag !== undefined) {
              context.strokeStyle = mix(tokens.surface, tokens.text, 0.35); context.lineWidth = 1;
              context.setLineDash([2, 2]);
              context.strokeRect(flagX + 0.5, at - FLAG_HEIGHT / 2 + 0.5, FLAG_WIDTH - 1, FLAG_HEIGHT - 1);
              context.setLineDash([]);
            }
          }
          context.fillStyle = tokens.text; context.font = font(tokens, tokens.fontSize.body, 500); context.textBaseline = 'middle';
          context.textAlign = side === 'source' ? 'right' : 'left';
          const label = fittedLabels.get(box.node.label) ?? '';
          context.fillText(label, side === 'source' ? flagX - LABEL_GAP / 2 : flagX + FLAG_WIDTH + LABEL_GAP / 2, at);
          const text = textWidth(context, label) + LABEL_GAP / 2;
          labels.push({ id: box.id, node: box.node, x: side === 'source' ? flagX - text : flagX, y: at - lineHeight / 2,
            width: FLAG_WIDTH + text, height: lineHeight, alpha: box.alpha });
        }
      }
      drawnLabels.current = labels;
      context.globalAlpha = 1;
      if (p.message) {
        context.fillStyle = tokens.muted; context.font = font(tokens, tokens.fontSize.body); context.textAlign = 'center'; context.textBaseline = 'middle';
        context.fillText(p.message, width / 2, height / 2, Math.max(40, width - PAD * 4));
      }

      if (!keyStop) {
        if (hit && pointer) describe(hit, pointer.clientX, pointer.clientY);
        else { tooltips.clear(); hideTooltip(owner); }
      } else if (!hit) {
        keyed.current = null; tooltips.clear(); hideTooltip(owner); setSpoken('');
      } else if (p.tooltip !== keyedTooltip) { keyedTooltip = p.tooltip; queueMicrotask(() => showKeyed.current()); }
      setStyle(element, 'cursor', hit ? 'pointer' : 'default');
      return !frame.settled;
    };
    let keyedTooltip: FlowDiagramProps['tooltip'];

    const current: FrameTask = { frame: draw };
    task.current = current;
    const wake = () => { scheduler.wake(current); if (keyed.current) queueMicrotask(() => showKeyed.current()); };
    const remove = scheduler.add(current, element);
    const unsubscribe = subscribeTokens(next => { tokens = next; labelWidth = 0; wake(); });
    const local = (event: PointerEvent | MouseEvent) => ({ ...toLocal(event.clientX, event.clientY, element.getBoundingClientRect(), element.clientWidth, element.clientHeight), clientX: event.clientX, clientY: event.clientY });
    const move = (event: PointerEvent) => {
      if (keyed.current) { keyed.current = null; setSpoken(''); }
      quiet.current = false;
      pointer = local(event);
      wake();
    };
    const leave = () => { pointer = null; tooltips.clear(); hideTooltip(owner); wake(); };
    const wheel = (event: WheelEvent) => {
      const delta = event.deltaY * (event.deltaMode === 1 ? tokens.fontSize.body * 1.4 : 1);
      if (scrollTooltip(owner, delta, event.deltaMode === 2)) event.preventDefault();
    };
    const menu = (event: MouseEvent) => {
      const handler = latest.current.onContextMenu;
      if (!handler) return;
      event.preventDefault();
      const point = local(event);
      handler(hitAt(point.x, point.y), event.clientX, event.clientY);
    };
    const pick = (event: MouseEvent) => {
      const p = latest.current;
      if (event.button !== 0 || !p.active || drawnHour.current == null) return;
      const point = local(event);
      const label = nodeAt(drawn.current.nodes, point.x, point.y) ? null : labelHit(point.x, point.y);
      if (label) { p.onLabelPick!(label.node, drawnHour.current); return; }
      if (!p.onPick) return;
      const hit = hitAt(point.x, point.y);
      if (hit) p.onPick(hit, drawnHour.current);
    };
    element.addEventListener('pointermove', move);
    element.addEventListener('pointerleave', leave);
    element.addEventListener('wheel', wheel, { passive: false });
    element.addEventListener('contextmenu', menu);
    element.addEventListener('click', pick);
    return () => {
      remove(); unsubscribe(); sized.disconnect(); hideTooltip(owner);
      element.removeEventListener('pointermove', move);
      element.removeEventListener('pointerleave', leave);
      element.removeEventListener('wheel', wheel);
      element.removeEventListener('contextmenu', menu);
      element.removeEventListener('click', pick);
      task.current = null;
    };
  }, [owner]);

  useEffect(() => {
    if (task.current) scheduler.setVisible(task.current, props.active);
    if (!props.active) { hideTooltip(owner); keyed.current = null; setSpoken(''); }
  }, [props.active, owner]);
  useEffect(() => { if (task.current) scheduler.wake(task.current); });
  useEffect(() => props.live?.subscribe(() => { if (task.current) scheduler.wake(task.current); }), [props.live]);

  const stopHit = (stop: FlowStop): { hit: FlowHit; x: number; y: number } | null => {
    const frame = drawn.current;
    if (stop.kind === 'node') {
      const box = frame.nodes.find(item => item.id === stop.id);
      return box ? { hit: { kind: 'node', node: box.node }, x: box.node.side === 'source' ? box.x : box.x + box.width, y: box.y + box.height / 2 } : null;
    }
    const shape = frame.links.find(item => item.id === stop.id);
    return shape ? { hit: { kind: 'link', link: shape.link }, x: (shape.x0 + shape.x1) / 2, y: (shape.y0 + shape.y1 + shape.width) / 2 } : null;
  };
  const ownsTooltip = () => { const held = tooltipStore.get(); return held == null || held.owner === owner; };
  showKeyed.current = (explicit = false) => {
    const stop = keyed.current;
    const element = canvas.current;
    if (!stop || !element || !latest.current.active || (!explicit && !ownsTooltip())) return;
    const found = stopHit(stop);
    if (!found) { keyed.current = null; hideTooltip(owner); setSpoken(''); return; }
    const described = latest.current.tooltip?.(found.hit) ?? null;
    const flag = found.hit.kind === 'node' ? found.hit.node.countryFlag : undefined;
    const content = countryTooltipContent(described, flag, flagLoaded.current, globalThis.innerHeight);
    if (!content) { hideTooltip(owner); setSpoken(''); return; }
    const bounds = element.getBoundingClientRect();
    const scale = element.offsetWidth ? bounds.width / element.offsetWidth : 1;
    showTooltip(owner, content, bounds.left + found.x * scale, bounds.top + found.y * scale, `key:${stop.kind}:${stop.id}`);
    setSpoken(tooltipText(content));
  };
  const keyDown = (event: ReactKeyboardEvent) => {
    if (!props.active) return;
    if ((event.key === 'PageUp' || event.key === 'PageDown') && scrollTooltip(owner, event.key === 'PageUp' ? -1 : 1, true)) {
      event.preventDefault();
      return;
    }
    const stops = flowStops(drawn.current);
    const index = keyed.current ? stops.findIndex(stop => stop.kind === keyed.current!.kind && stop.id === keyed.current!.id) : -1;
    const go = (next: number) => {
      event.preventDefault();
      if (!stops.length) return;
      keyed.current = stops[Math.max(0, Math.min(stops.length - 1, next))];
      quiet.current = false;
      showKeyed.current(true);
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
    if ((event.key === 'Enter' || event.key === ' ') && props.onPick) {
      event.preventDefault();
      const found = keyed.current ? stopHit(keyed.current) : null;
      if (found && drawnHour.current != null) props.onPick(found.hit, drawnHour.current);
      return;
    }
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
      aria-keyshortcuts="ArrowUp ArrowDown ArrowLeft ArrowRight Home End PageUp PageDown Enter Space Escape ContextMenu" onKeyDown={keyDown}
      onBlur={() => { keyed.current = null; hideTooltip(owner); setSpoken(''); }} />
    <p className="sr-only" aria-live="polite">{spoken}</p>
    {props.description && <p id={`${owner}-description`} className="sr-only">{props.description}</p>}
  </div>;
}

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
