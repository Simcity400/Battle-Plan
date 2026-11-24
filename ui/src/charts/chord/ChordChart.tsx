import { useEffect, useId, useRef, useState } from 'react';
import { font } from '../draw';
import { easeTau } from '../easing';
import { useMotionNow } from '../liveMotion.ts';
import { NO_HOVER, pointerMenu, reduceHover } from '../hover';
import type { HoverState } from '../hover';
import { canvasImage } from '../images';
import { fitLabels, layoutEndLabels } from '../labels';
import { countryTooltipContent, drawCountryFlag } from '../countryFlag';
import type { Live } from '../live';
import { scheduler } from '../scheduler';
import type { FrameTask } from '../scheduler';
import { drawingSurface, sizeCanvas, toLocal, watchCanvasSize } from '../surface';
import { hideTooltip, scrollTooltip, showTooltip, tooltipStore, tooltipText } from '../tooltip';
import type { TooltipContent } from '../tooltip';
import { useChartTokens, useViewportHeight } from '../useChartTokens';
import { ARC_BAND, RIBBON_INSET, chordStops, chordSurface, hitChord, layoutChord, stepChord } from './layout';
import type { ChordArc, ChordData, ChordHit, ChordLayout, ChordSurface } from './layout';
import { ChordFrames } from './frame';
import type { ChordFrame } from './frame';
import '../charts.css';
import { textWidth } from '../textWidth.ts';

export type { ChordFrame } from './frame';
export interface ChordChartProps {
  label: string;
  caption: string;
  frame: ChordFrame;
  frameAt?: (now: number) => ChordFrame;
  live?: Live<ChordFrame>;
  hoursPerSecond?: number;
  active: boolean;
  tooltip: (hit: ChordHit, data: ChordData) => TooltipContent | null;
  onContextMenu?: (hit: ChordHit, data: ChordData, clientX: number, clientY: number) => void;
  onPick?: (hit: ChordHit, data: ChordData) => void;
  onError: (message: string | null) => void;
}

export default function ChordChart(props: ChordChartProps) {
  const owner = useId(), canvas = useRef<HTMLCanvasElement>(null);
  const tokens = useChartTokens(), viewportHeight = useViewportHeight();
  const motionNow = useMotionNow();
  const latest = useRef({ props, tokens, viewportHeight });
  latest.current = { props, tokens, viewportHeight };
  const task = useRef<FrameTask | null>(null);
  const [spoken, setSpoken] = useState('');

  useEffect(() => {
    const element = canvas.current!, paper = drawingSurface(element), context = paper.context;
    if (!context) return;
    const frames = new ChordFrames(), motion = frames.motion;
    let disposed = false, refusal: string | null | undefined;
    let layout: ChordLayout = { arcs: [], ribbons: [] }, surface: ChordSurface = chordSurface(0, 0);
    let placed: ChordData | null = null;
    let hover: HoverState<{ x: number; y: number; clientX: number; clientY: number }, ChordHit> = NO_HOVER;
    let hot: ChordHit | null = null, keyText = '';
    let descriptionKey = 0;
    let described: { data: ChordData; revision: unknown; hit: ChordHit; tooltip: ChordChartProps['tooltip']; height: number; content: TooltipContent | null; fitted: TooltipContent | null; spoken: string; key: string } | null = null;
    const wake = () => { described = null; if (!disposed && task.current) scheduler.wake(task.current); };
    const sized = watchCanvasSize(element, wake);
    let indexed: ChordData | null = null;
    let flowIds = new Set<string>(), nodeIds = new Set<string>();
    const current = (hit: ChordHit | null): ChordHit | null => {
      const data = frames.data;
      if (!hit || !data) return null;
      if (indexed !== data) {
        indexed = data;
        flowIds = new Set(data.flows.map(flow => flow.id));
        nodeIds = new Set(data.nodes.map(node => node.id));
      }
      return (hit.kind === 'flow' ? flowIds : nodeIds).has(hit.id) ? hit : null;
    };
    const keyboardPosition = () => {
      let angle: number;
      if (hot?.kind === 'node') {
        const arc = layout.arcs.find(arc => arc.node.id === hot!.id);
        if (!arc || arc.end <= arc.start) return null;
        angle = (arc.start + arc.end) / 2;
      } else {
        const ribbon = layout.ribbons.find(ribbon => ribbon.flow.id === hot?.id);
        if (!ribbon || ribbon.sourceEnd <= ribbon.sourceStart) return null;
        angle = (ribbon.sourceStart + ribbon.sourceEnd) / 2;
      }
      const rect = element.getBoundingClientRect();
      return { x: rect.left + (surface.x + Math.cos(angle) * surface.radius) * rect.width / Math.max(1, element.clientWidth),
        y: rect.top + (surface.y + Math.sin(angle) * surface.radius) * rect.height / Math.max(1, element.clientHeight) };
    };
    const publish = (keyIntent = false) => {
      const heldTooltip = tooltipStore.get();
      if (hover.keyed && !hover.pointer && !keyIntent && heldTooltip != null && heldTooltip.owner !== owner) return;
      const { props: p, viewportHeight: height } = latest.current;
      const data = frames.data;
      if (hot && data && (described?.data !== data || described.revision !== frames.revision || described.hit.kind !== hot.kind || described.hit.id !== hot.id || described.tooltip !== p.tooltip || described.height !== height)) {
        const flag = hot.kind === 'node' ? data.nodes.find(node => node.id === hot!.id)?.countryFlag : undefined;
        const content = p.tooltip(hot, data);
        let keep = 0;
        if (content) for (let i = content.rows.length - 1; i >= 0 && content.rows[i].strong; i--) keep++;
        const fitted = countryTooltipContent(content, flag, wake, height, keep);
        described = { data, revision: frames.revision, hit: hot, tooltip: p.tooltip, height, content, fitted, spoken: fitted ? tooltipText(fitted) : '', key: String(++descriptionKey) };
      }
      const content = hot && data ? described?.content : null;
      if (content) {
        const position = keyboardPosition();
        if (hover.pointer) showTooltip(owner, described!.fitted!, hover.pointer.clientX, hover.pointer.clientY, described!.key);
        else if (position) showTooltip(owner, described!.fitted!, position.x, position.y, described!.key);
        else hideTooltip(owner);
      } else hideTooltip(owner);
      const text = hover.keyed && content ? described!.spoken : '';
      if (text !== keyText) { keyText = text; setSpoken(text); }
    };
    const resolve = (now: number) => {
      const p = latest.current.props;
      let frame = p.frame;
      let error: string | null = null;
      try {
        frame = p.live ? p.live.read() : p.frameAt?.(now) ?? frame;
        frames.accept(frame);
      } catch (cause) {
        error = cause instanceof Error ? cause.message : String(cause);
        frames.clear();
      }
      if (error !== refusal) { refusal = error; p.onError(error); }
      return frame;
    };
    const drawing: FrameTask = { frame({ now, dt }) {
      const { props: p, tokens: t } = latest.current, frame = resolve(now);
      const { layoutWidth: width, layoutHeight: height, backing: size } = sized.read();
      sizeCanvas(paper.canvas, size.width, size.height);
      context.setTransform(size.scaleX, 0, 0, size.scaleY, 0, 0); context.clearRect(0, 0, width, height);
      surface = chordSurface(width, height);
      const eased = motion.advance(dt, easeTau(t.motion, motionNow(p.hoursPerSecond), t.reducedMotion));
      if (placed !== eased.data) { placed = eased.data; layout = layoutChord(eased.data, motion.presence, eased.ringPresence); }
      if (hover.pointer) {
        const local = toLocal(hover.pointer.clientX, hover.pointer.clientY, element.getBoundingClientRect(), width, height);
        hover.pointer.x = local.x; hover.pointer.y = local.y;
      }
      hot = current(hover.pointer ? hitChord(layout, surface, hover.pointer.x, hover.pointer.y) : hover.keyed);
      const { x, y, radius } = surface, inner = Math.max(0, radius - RIBBON_INSET);
      const colours = new Map(eased.data.nodes.map(node => [node.id, node.color]));
      if (inner > 0) for (const ribbon of layout.ribbons) {
        context.beginPath(); context.arc(x, y, inner, ribbon.sourceStart, ribbon.sourceEnd);
        context.quadraticCurveTo(x, y, x + Math.cos(ribbon.targetStart) * inner, y + Math.sin(ribbon.targetStart) * inner);
        context.arc(x, y, inner, ribbon.targetStart, ribbon.targetEnd);
        context.quadraticCurveTo(x, y, x + Math.cos(ribbon.sourceStart) * inner, y + Math.sin(ribbon.sourceStart) * inner);
        context.closePath(); context.fillStyle = colours.get(ribbon.flow.source)!;
        context.globalAlpha = hot?.kind === 'flow' && hot.id !== ribbon.flow.id ? 0.1 : 0.6; context.fill();
      }
      const labels: { arc: ChordArc; id: string; y: number; height: number; right: boolean }[] = [];
      if (radius > 0) for (const arc of layout.arcs) {
        if (arc.end <= arc.start) continue;
        context.globalAlpha = 1;
        context.beginPath(); context.arc(x, y, radius, arc.start, arc.end);
        context.arc(x, y, Math.max(0, radius - ARC_BAND), arc.end, arc.start, true); context.closePath();
        context.fillStyle = arc.node.color; context.fill();
        const angle = (arc.start + arc.end) / 2;
        labels.push({ arc, id: arc.node.id, y: y + Math.sin(angle) * (radius + 12), height: t.fontSize.small + 3, right: Math.cos(angle) >= 0 });
      }
      const measure = (text: string) => textWidth(context, text);
      for (const right of [true, false]) {
        const side = labels.filter(label => label.right === right);
        const placed = layoutEndLabels(side, 2, height - (right ? 26 : 2), 1);
        context.font = font(t, t.fontSize.small, 500);
        const rooms = new Map<string, number>();
        const laid = side.map(({ arc, id }) => {
          const ly = placed.get(id)!, ring = radius + 12, reach = Math.sqrt(Math.max(0, ring * ring - (ly - y) ** 2));
          const lx = right ? x + reach : x - reach;
          const room = right ? width - lx - 23 : lx - 23;
          const prev = rooms.get(arc.node.label);
          rooms.set(arc.node.label, prev === undefined ? room : Math.min(prev, room));
          return { arc, lx, ly };
        });
        const fitted = fitLabels(laid.map(item => item.arc.node.label), text => rooms.get(text) ?? 0, measure);
        for (const { arc, lx, ly } of laid) {
          const flagX = right ? lx : lx - 16;
          context.globalAlpha = arc.presence;
          if (arc.node.countryFlag) drawCountryFlag(context, arc.node.countryFlag, flagX, ly - 5, 15, 10, t, wake);
          else {
            const flag = arc.node.flag ? canvasImage(arc.node.flag, wake) : null;
            if (flag) context.drawImage(flag, flagX, ly - 5, 15, 10);
          }
          context.font = font(t, t.fontSize.small, 500); context.textBaseline = 'middle'; context.textAlign = right ? 'left' : 'right';
          context.fillStyle = t.text;
          context.fillText(fitted.get(arc.node.label) ?? '', right ? lx + 19 : lx - 19, ly);
        }
      }
      context.globalAlpha = 1; context.font = font(t, t.fontSize.small, 500); context.fillStyle = t.muted;
      context.textAlign = 'right'; context.textBaseline = 'middle'; if (labels.length) context.fillText(p.caption, width - 12, height - 14);
      publish();
      return eased.moving || Boolean(frame.playing);
    } };
    task.current = drawing;
    const remove = scheduler.add(drawing, element); scheduler.setVisible(drawing, latest.current.props.active);
    const move = (event: PointerEvent) => {
      const local = toLocal(event.clientX, event.clientY, element.getBoundingClientRect(), element.clientWidth, element.clientHeight);
      hover = reduceHover(hover, { type: 'pointer', point: { ...local, clientX: event.clientX, clientY: event.clientY } }); wake();
    };
    const leave = () => { hover = reduceHover(hover, { type: 'pointer-leave' }); hot = current(hover.keyed); publish(); wake(); };
    const press = (event: PointerEvent) => { hover = reduceHover(hover, { type: 'pointer-down' }); if (event.button === 2) move(event); };
    const pick = (event: MouseEvent) => {
      if (event.button !== 0 || !latest.current.props.onPick) return;
      resolve(performance.now());
      const point = toLocal(event.clientX, event.clientY, element.getBoundingClientRect(), element.clientWidth, element.clientHeight);
      const hit = current(hitChord(layout, surface, point.x, point.y));
      if (hit && frames.data) latest.current.props.onPick(hit, frames.data);
    };
    const menu = (clientX: number, clientY: number) => {
      resolve(performance.now()); hot = current(hot);
      if (hot && frames.data) latest.current.props.onContextMenu?.(hot, frames.data, clientX, clientY);
    };
    const contextMenu = (event: MouseEvent) => {
      const fromPointer = pointerMenu(hover);
      hover = reduceHover(hover, { type: 'contextmenu' });
      if (!fromPointer) { event.preventDefault(); return; }
      if (!latest.current.props.onContextMenu) return;
      resolve(performance.now());
      const point = toLocal(event.clientX, event.clientY, element.getBoundingClientRect(), element.clientWidth, element.clientHeight);
      hot = current(hitChord(layout, surface, point.x, point.y));
      if (!hot) return;
      event.preventDefault(); menu(event.clientX, event.clientY);
    };
    const key = (event: KeyboardEvent) => {
      if ((event.key === 'PageUp' || event.key === 'PageDown') && scrollTooltip(owner, event.key === 'PageUp' ? -1 : 1, true)) {
        event.preventDefault();
      } else if (['ArrowLeft', 'ArrowUp', 'ArrowRight', 'ArrowDown', 'Home', 'End'].includes(event.key)) {
        event.preventDefault(); resolve(performance.now());
        const stops = chordStops(layout).filter(stop => current(stop));
        const delta = event.key === 'ArrowLeft' || event.key === 'ArrowUp' ? -1 : 1;
        const keyed = event.key === 'Home' ? stops[0] ?? null : event.key === 'End' ? stops.at(-1) ?? null : stepChord(stops, hover.keyed ?? hot, delta);
        hover = reduceHover(hover, { type: 'key', hour: keyed }); hot = keyed; publish(true); wake();
      } else if (event.key === 'Escape') {
        event.preventDefault(); hover = reduceHover(reduceHover(hover, { type: 'key-clear' }), { type: 'pointer-leave' }); hot = null; publish(); wake();
      } else if ((event.key === 'Enter' || event.key === ' ') && latest.current.props.onPick) {
        resolve(performance.now()); hot = current(hot);
        if (hot && frames.data) { event.preventDefault(); latest.current.props.onPick(hot, frames.data); }
      } else if ((event.key === 'ContextMenu' || event.key === 'F10' && event.shiftKey) && latest.current.props.onContextMenu) {
        const position = keyboardPosition();
        if (position && hot) { event.preventDefault(); hover = reduceHover(hover, { type: 'menu-key' }); menu(position.x, position.y); }
      }
    };
    const blur = () => { hover = reduceHover(hover, { type: 'key-clear' }); if (!hover.pointer) { hot = null; publish(); wake(); } };
    const wheel = (event: WheelEvent) => {
      const delta = event.deltaY * (event.deltaMode === 1 ? latest.current.tokens.fontSize.body * 1.4 : 1);
      if (scrollTooltip(owner, delta, event.deltaMode === 2)) event.preventDefault();
    };
    element.addEventListener('pointermove', move); element.addEventListener('pointerleave', leave); element.addEventListener('pointerdown', press);
    element.addEventListener('contextmenu', contextMenu); element.addEventListener('keydown', key); element.addEventListener('blur', blur);
    element.addEventListener('click', pick);
    element.addEventListener('wheel', wheel, { passive: false });
    return () => {
      disposed = true; remove(); sized.disconnect(); hideTooltip(owner); task.current = null;
      element.removeEventListener('pointermove', move); element.removeEventListener('pointerleave', leave); element.removeEventListener('pointerdown', press);
      element.removeEventListener('contextmenu', contextMenu); element.removeEventListener('keydown', key); element.removeEventListener('blur', blur);
      element.removeEventListener('click', pick);
      element.removeEventListener('wheel', wheel);
    };
  }, [owner]);

  useEffect(() => {
    if (task.current) { scheduler.setVisible(task.current, props.active); scheduler.wake(task.current); }
    if (!props.active) hideTooltip(owner);
  }, [props, tokens, viewportHeight, owner]);
  useEffect(() => props.live?.subscribe(() => { if (task.current) scheduler.wake(task.current); }), [props.live]);

  return <div className="chord-chart">
    <canvas ref={canvas} role="img" aria-label={props.label} tabIndex={0} aria-describedby={`${owner}-keys`}
      aria-keyshortcuts="ArrowLeft ArrowUp ArrowRight ArrowDown Home End" />
    <p className="sr-only" id={`${owner}-keys`}>Arrow keys step through ribbons and arcs. Home and End go to either end. Enter picks the selected item. The menu key or Shift+F10 opens its menu. Escape leaves the selection.</p>
    <p className="sr-only" aria-live="polite">{spoken}</p>
  </div>;
}
