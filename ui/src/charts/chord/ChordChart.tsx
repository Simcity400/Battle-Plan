import { useEffect, useId, useRef, useState } from 'react';
import { font } from '../draw';
import { easeTau } from '../easing';
import { NO_HOVER, pointerMenu, reduceHover } from '../hover';
import type { HoverState } from '../hover';
import { canvasImage } from '../images';
import { countryTooltipContent, drawCountryFlag } from '../countryFlag';
import { scheduler } from '../scheduler';
import type { FrameTask } from '../scheduler';
import { backingSize, toLocal } from '../surface';
import { hideTooltip, scrollTooltip, showTooltip, tooltipStore, tooltipText } from '../tooltip';
import type { TooltipContent } from '../tooltip';
import { useChartTokens, useViewportHeight } from '../useChartTokens';
import { ARC_BAND, RIBBON_INSET, chordStops, chordSurface, hitChord, layoutChord, stepChord } from './layout';
import type { ChordData, ChordHit, ChordLayout, ChordSurface } from './layout';
import { ChordFrames } from './frame';
import type { ChordFrame } from './frame';
import '../charts.css';

export type { ChordFrame } from './frame';
export interface ChordChartProps {
  label: string;
  caption: string;
  frame: ChordFrame;
  /** Local playback lookup; reuse data identity until exact data changes. */
  frameAt?: (now: number) => ChordFrame;
  hoursPerSecond?: number;
  active: boolean;
  /** Always describes the resolved data, never the eased geometry. */
  tooltip: (hit: ChordHit, data: ChordData) => TooltipContent | null;
  onContextMenu?: (hit: ChordHit, data: ChordData, clientX: number, clientY: number) => void;
  onPick?: (hit: ChordHit, data: ChordData) => void;
  /** A refused frame is cleared; null reports recovery on the next valid frame. */
  onError: (message: string | null) => void;
}

/** Style-6's directed ribbons, on the dashboard's shared animation scheduler. */
export default function ChordChart(props: ChordChartProps) {
  const owner = useId(), canvas = useRef<HTMLCanvasElement>(null);
  const tokens = useChartTokens(), viewportHeight = useViewportHeight();
  const latest = useRef({ props, tokens, viewportHeight });
  latest.current = { props, tokens, viewportHeight };
  const task = useRef<FrameTask | null>(null);
  const [spoken, setSpoken] = useState('');

  useEffect(() => {
    const element = canvas.current!, context = element.getContext('2d');
    if (!context) return;
    const frames = new ChordFrames(), motion = frames.motion;
    let disposed = false, refusal: string | null | undefined;
    let layout: ChordLayout = { arcs: [], ribbons: [] }, surface: ChordSurface = chordSurface(0, 0);
    let hover: HoverState<{ x: number; y: number; clientX: number; clientY: number }, ChordHit> = NO_HOVER;
    let hot: ChordHit | null = null, keyText = '';
    let descriptionKey = 0;
    let described: { data: ChordData; hit: ChordHit; tooltip: ChordChartProps['tooltip']; height: number; content: TooltipContent | null; fitted: TooltipContent | null; spoken: string; key: string } | null = null;
    let device: { width: number; height: number } | undefined;
    const wake = () => { described = null; if (!disposed && task.current) scheduler.wake(task.current); };
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
      if (hot && data && (described?.data !== data || described.hit.kind !== hot.kind || described.hit.id !== hot.id || described.tooltip !== p.tooltip || described.height !== height)) {
        const flag = hot.kind === 'node' ? data.nodes.find(node => node.id === hot!.id)?.countryFlag : undefined;
        const content = p.tooltip(hot, data);
        let keep = 0;
        if (content) for (let i = content.rows.length - 1; i >= 0 && content.rows[i].strong; i--) keep++;
        const fitted = countryTooltipContent(content, flag, wake, height, keep);
        described = { data, hit: hot, tooltip: p.tooltip, height, content, fitted, spoken: fitted ? tooltipText(fitted) : '', key: String(++descriptionKey) };
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
        frame = p.frameAt?.(now) ?? frame;
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
      const rect = element.getBoundingClientRect(), width = element.clientWidth, height = element.clientHeight;
      const size = backingSize({ layoutWidth: width, layoutHeight: height, rectWidth: rect.width, rectHeight: rect.height,
        devicePixelRatio: window.devicePixelRatio || 1, devicePixelWidth: device?.width, devicePixelHeight: device?.height });
      if (element.width !== size.width || element.height !== size.height) { element.width = size.width; element.height = size.height; }
      context.setTransform(size.scaleX, 0, 0, size.scaleY, 0, 0); context.clearRect(0, 0, width, height);
      surface = chordSurface(width, height);
      const eased = motion.advance(dt, easeTau(t.motion, p.hoursPerSecond ?? 0, t.reducedMotion));
      layout = layoutChord(eased.data, motion.presence, eased.ringPresence);
      if (hover.pointer) {
        const local = toLocal(hover.pointer.clientX, hover.pointer.clientY, rect, width, height);
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
      if (radius > 0) for (const arc of layout.arcs) {
        if (arc.end <= arc.start) continue;
        context.globalAlpha = 1;
        context.beginPath(); context.arc(x, y, radius, arc.start, arc.end);
        context.arc(x, y, Math.max(0, radius - ARC_BAND), arc.end, arc.start, true); context.closePath();
        context.fillStyle = arc.node.color; context.fill();
        const angle = (arc.start + arc.end) / 2, lx = x + Math.cos(angle) * (radius + 12), ly = y + Math.sin(angle) * (radius + 12);
        const right = Math.cos(angle) >= 0, flagX = right ? lx : lx - 16;
        context.globalAlpha = arc.presence;
        if (arc.node.countryFlag) drawCountryFlag(context, arc.node.countryFlag, flagX, ly - 5, 15, 10, t, wake);
        else {
          const flag = arc.node.flag ? canvasImage(arc.node.flag, wake) : null;
          if (flag) context.drawImage(flag, flagX, ly - 5, 15, 10);
        }
        context.font = font(t, t.fontSize.small, 500); context.textBaseline = 'middle'; context.textAlign = right ? 'left' : 'right';
        context.fillStyle = t.text; context.fillText(arc.node.label, right ? lx + 19 : lx - 19, ly);
      }
      context.globalAlpha = 1; context.font = font(t, t.fontSize.small, 500); context.fillStyle = t.muted;
      context.textAlign = 'right'; context.textBaseline = 'middle'; context.fillText(p.caption, width - 12, height - 14);
      publish();
      return eased.moving || Boolean(frame.playing);
    } };
    task.current = drawing;
    const remove = scheduler.add(drawing); scheduler.setVisible(drawing, latest.current.props.active);
    const resize = (entries: ResizeObserverEntry[]) => {
      const box = entries[0]?.devicePixelContentBoxSize?.[0];
      device = box ? { width: box.inlineSize, height: box.blockSize } : undefined; wake();
    };
    let observer: ResizeObserver;
    try { observer = new ResizeObserver(resize); observer.observe(element, { box: 'device-pixel-content-box' }); }
    catch { observer = new ResizeObserver(resize); observer.observe(element); }
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
      disposed = true; remove(); observer.disconnect(); hideTooltip(owner); task.current = null;
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

  return <div className="chord-chart">
    <canvas ref={canvas} role="img" aria-label={props.label} tabIndex={0} aria-describedby={`${owner}-keys`}
      aria-keyshortcuts="ArrowLeft ArrowUp ArrowRight ArrowDown Home End" />
    <p className="sr-only" id={`${owner}-keys`}>Arrow keys step through ribbons and arcs. Home and End go to either end. Enter picks the selected item. The menu key or Shift+F10 opens its menu. Escape leaves the selection.</p>
    <p className="sr-only" aria-live="polite">{spoken}</p>
  </div>;
}
