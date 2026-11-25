import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import { useLayoutEffect } from 'react';
import type { FocusEvent as ReactFocusEvent, KeyboardEvent as ReactKeyboardEvent, MouseEvent as ReactMouseEvent, PointerEvent as ReactPointerEvent, ReactNode } from 'react';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { mix, parseColor, readableText, toHex } from '../../shell/appearance/color';
import type { Rgb } from '../../shell/appearance/color';
import { font } from '../draw';
import { approach, easeTau } from '../easing';
import { useMotionNow } from '../liveMotion.ts';
import { setStyle } from '../domText.ts';
import type { Live } from '../live';
import { scheduler } from '../scheduler';
import type { FrameInfo, FrameTask } from '../scheduler';
import { drawingSurface, sizeCanvas, toLocal, watchCanvasSize } from '../surface';
import { readTokens, subscribeTokens } from '../tokens';
import { useChartTokens } from '../useChartTokens';
import WindowedRows from '../WindowedRows';
import type { MovingRow } from '../rowMotion';
import { fitTooltip, hideTooltip, showTooltip, tooltipStore, tooltipText } from '../tooltip';
import type { TooltipContent } from '../tooltip';
import { amountText, approachColour, arcTargets, foldView, hasCorrections, keyRings, listed, keyedSlice, labelFits, notMeasured, pathTo, shareText, sliceAt, sliceColours,
  sliceTooltip, sunGeometry, sunburstSummary, sunburstTree, unknownTotal, walk } from './layout.ts';
import type { DrawnSlice, SunburstItem, SunburstNode, SunburstText, SunburstTotal, SunKey } from './layout';
import type { SunburstSettings } from './sunburstOptions';
import { pickSunburst } from './pick';
import type { SunburstPick } from './pick';
import '../charts.css';
import './sunburst.css';
import { textWidth } from '../textWidth.ts';

export interface SunburstProps {
  label: string;
  items: readonly SunburstItem[];
  root: { id: string; label: string };
  authoritativeTotal?: SunburstTotal;
  levels?: readonly boolean[];
  text: SunburstText;
  settings: SunburstSettings;
  summaryNotes?: boolean;
  rings?: 1 | 2;
  fold?: boolean;
  unavailableReason?: string | null;
  stretch?: number | string | null;
  tooltip?: (node: SunburstNode, focus: SunburstNode, text: SunburstText) => readonly TooltipContent[];
  onHover?: (node: SunburstNode) => void;
  onPick?: (node: SunburstNode) => boolean;
  canPick?: (node: SunburstNode) => boolean;
  pickOwner?: unknown;
  onContextMenu?: (node: SunburstNode | null, x: number, y: number) => void;
  hoursPerSecond?: number;
  active: boolean;
  footnote?: ReactNode;
  legendExtra?: ReactNode;
  live?: Live<SunburstValues>;
}

export type SunburstValues = Pick<SunburstProps, 'items' | 'authoritativeTotal' | 'unavailableReason'>;

const legendAmount = (part: SunburstNode, text: SunburstText) => part.valueUnknown ? 'Not known' : notMeasured(part) ? text.missingLabel ?? 'Not measured' : amountText(part, text);
const legendShare = (part: SunburstNode, focus: SunburstNode, text: SunburstText) => text.shares === false || part.valueUnknown || focus.valueUnknown || part.unmeasured
  || !(part.value > 0 || part.value === 0 && part.keepZero) || hasCorrections(focus) ? '' : shareText(part.value, focus.value);
const legendIds = (focus: SunburstNode) => focus.children.filter(listed).map(child => child.id).join('\u0000');

function paintLegend(host: HTMLElement | null, focus: SunburstNode, text: SunburstText) {
  if (!host) return;
  const byId = new Map(focus.children.map(child => [child.id, child]));
  for (const element of host.querySelectorAll<HTMLElement>('[data-sunburst-amount]')) {
    const part = byId.get(element.dataset.sunburstAmount!);
    const next = part ? legendAmount(part, text) : 'Not known';
    if (element.textContent !== next) element.textContent = next;
  }
  for (const element of host.querySelectorAll<HTMLElement>('[data-sunburst-share]')) {
    const part = byId.get(element.dataset.sunburstShare!);
    const next = part ? legendShare(part, focus, text) : '';
    if (element.textContent !== next) element.textContent = next;
  }
}

const GAP = 2;
const LIFT = 4;
const DIM = 0.45;

interface Motion { a0: number; a1: number; ring: number; alpha: number; rgb: Rgb; colour: string; stretch: SunburstNode['stretch'] }

const LINE = 1.35;
interface CentreLine { text: string; size: number; weight: number; colour: string; rank: number }

function centreLines(lines: readonly CentreLine[], height: number): CentreLine[] {
  const kept = new Set<CentreLine>();
  let used = 0;
  for (const line of [...lines].sort((a, b) => a.rank - b.rank)) {
    if (kept.size && used + line.size * LINE > height) continue;
    kept.add(line);
    used += line.size * LINE;
  }
  return lines.filter(line => kept.has(line));
}

function fitText(text: string, width: number, measure: (text: string) => number): string {
  if (measure(text) <= width) return text;
  let cut = text.length;
  while (cut > 1 && measure(`${text.slice(0, cut)}…`) > width) cut--;
  return `${text.slice(0, cut).trimEnd()}…`;
}

const free = (who: string) => { const current = tooltipStore.get(); return current == null || current.owner === who; };

export default function Sunburst(props: SunburstProps) {
  const owner = useId();
  const tokens = useChartTokens();
  const motionNow = useMotionNow();
  const canvas = useRef<HTMLCanvasElement>(null);
  const [focusPath, setFocusPath] = useState<readonly string[]>([]);
  const [adopted, setAdopted] = useState<{ live: Live<SunburstValues>; items: readonly SunburstItem[]; values: SunburstValues } | null>(null);
  const structure: SunburstValues = props.live && adopted?.live === props.live && adopted.items === props.items ? adopted.values : props;
  const { tree, focus } = useMemo(() => {
    const tree = sunburstTree(structure.items, props.root, props.levels, { fold: props.fold, unavailableReason: structure.unavailableReason, authoritativeTotal: structure.authoritativeTotal });
    return { tree, focus: foldView(tree, focusPath) };
  }, [structure.items, props.root, props.levels, props.fold, structure.unavailableReason, structure.authoritativeTotal, focusPath]);
  const shape = JSON.stringify([props.root, props.levels ?? null, props.fold ?? null, focusPath]);
  const liveTree = useRef<{ values: SunburstValues; shape: string; tree: SunburstNode; focus: SunburstNode } | null>(null);
  const live = props.live && liveTree.current?.shape === shape ? liveTree.current : null;
  const latest = useRef({ props, tree, focus, shape, focusPath, listed: legendIds(focus) });
  latest.current = { props, tree: live?.tree ?? tree, focus: live?.focus ?? focus, shape, focusPath, listed: legendIds(focus) };
  const legendHost = useRef<HTMLDivElement>(null);
  const legendShow = useRef<(focus: SunburstNode, fresh: boolean, speak: boolean) => void>(() => {});
  const [legendHover, setLegendHover] = useState<string | null>(null);
  const [legendTip, setLegendTip] = useState<{ id: string; x: number; y: number; row?: HTMLElement } | null>(null);
  const legendOwner = `${owner}-legend`;
  const legendHot = useRef<string | null>(null);
  legendHot.current = legendHover;
  const keyed = useRef<string | null>(null);
  const [spoken, setSpoken] = useState('');
  const quiet = useRef(false);
  const drawn = useRef<{ slices: DrawnSlice[]; geometry: ReturnType<typeof sunGeometry>; pick: SunburstPick } | null>(null);
  const task = useRef<FrameTask | null>(null);

  const zoomTo = (node: SunburstNode | null) => {
    if (!node) return;
    setFocusPath(pathTo(node).slice(1).map(step => step.id));
    setLegendHover(null);
  };
  const zoomable = (node: SunburstNode) => node.children.some(listed);

  useEffect(() => {
    const element = canvas.current!;
    const paper = drawingSurface(element), context = paper.context!;
    const sized = watchCanvasSize(element, () => { if (task.current) scheduler.wake(task.current); });
    let tokens = readTokens();
    let pointer: { x: number; y: number; clientX: number; clientY: number } | null = null;
    let moved = false;
    const motion = new Map<string, Motion>();
    let cached: { tree: SunburstNode; focus: SunburstNode; growth: string; rings: number; targets: ReturnType<typeof arcTargets>;
      colours: Map<string, string>; nodes: SunburstNode[]; byId: Map<string, SunburstNode> } | null = null;
    let previousStretch = latest.current.props.stretch;
    let wasUnavailable = false;
    const labelWidths = new Map<string, number>();
    let labelFont = '';
    const livePaint = () => {
      const held = latest.current, p = held.props;
      if (!p.live) return null;
      const values = p.live.read();
      const made = liveTree.current;
      if (made && made.values === values && made.shape === held.shape) return made;
      const tree = sunburstTree(values.items, p.root, p.levels, { fold: p.fold, unavailableReason: values.unavailableReason, authoritativeTotal: values.authoritativeTotal });
      const next = { values, shape: held.shape, tree, focus: foldView(tree, held.focusPath) };
      liveTree.current = next;
      held.tree = next.tree; held.focus = next.focus;
      paintLegend(legendHost.current, next.focus, p.text);
      legendShow.current(next.focus, false, false);
      if (legendIds(next.focus) !== held.listed) {
        setAdopted({ live: p.live, items: p.items, values });
      }
      return next;
    };

    const draw = (info: FrameInfo): boolean => {
      livePaint();
      const { props: p, tree: root, focus: zoomed } = latest.current;
      const { layoutWidth, layoutHeight, backing: size } = sized.read();
      if (!layoutWidth || !layoutHeight) return false;
      sizeCanvas(paper.canvas, size.width, size.height);
      context.setTransform(size.scaleX, 0, 0, size.scaleY, 0, 0);
      context.clearRect(0, 0, layoutWidth, layoutHeight);

      const rings = p.rings ?? 2;
      if (!cached || cached.tree !== root || cached.focus !== zoomed || cached.growth !== p.settings.growth || cached.rings !== rings) {
        const nodes = walk(root);
        cached = { tree: root, focus: zoomed, growth: p.settings.growth, rings, targets: arcTargets(root, zoomed, p.settings.growth, rings),
          colours: sliceColours(zoomed, tokens.categories, { light: tokens.text, dark: tokens.surface }), nodes, byId: new Map(nodes.map(node => [node.id, node])) };
      }
      const { targets, colours, nodes, byId } = cached;
      const g = sunGeometry(layoutWidth, layoutHeight, rings);
      const unavailable = (p.live ? liveTree.current?.values.unavailableReason : p.unavailableReason) != null;
      const snap = unavailable || wasUnavailable || p.stretch === null || previousStretch !== p.stretch;
      previousStretch = p.stretch;
      wasUnavailable = unavailable;
      if (unavailable) motion.clear();
      const tau = snap ? 0 : easeTau(tokens.motion, motionNow(p.hoursPerSecond), tokens.reducedMotion);
      const fade = tokens.reducedMotion ? 0 : tau * 1.2;
      const blend = tau > 0 ? 1 - Math.exp(-info.dt / tau) : 1;
      let moving = false;
      const angleWithin = 0.5 / Math.max(1, g.hole + rings * g.ring), ringWithin = 0.5 / Math.max(1, g.ring);
      const present = new Set<string>();
      for (const node of nodes) {
        present.add(node.id);
        const target = targets.get(node.id)!;
        const colour = colours.get(node.id);
        const alphaTarget = target.shown ? 1 : 0;
        let state = motion.get(node.id);
        const goal = parseColor(colour ?? tokens.muted) ?? { r: 0, g: 0, b: 0 };
        if (state && (node.stretch === null || state.stretch !== node.stretch)) {
          state.a0 = target.a0; state.a1 = target.a1; state.ring = target.ring; state.alpha = alphaTarget;
          state.rgb = goal; state.colour = toHex(goal); state.stretch = node.stretch;
        }
        if (!state) { state = { a0: target.a0, a1: target.a0, ring: target.ring, alpha: 0, rgb: goal, colour: toHex(goal), stretch: node.stretch }; motion.set(node.id, state); }
        state.a0 = approach(state.a0, target.a0, info.dt, tau, angleWithin);
        state.a1 = approach(state.a1, target.a1, info.dt, tau, angleWithin);
        state.ring = approach(state.ring, target.ring, info.dt, tau, ringWithin);
        state.alpha = approach(state.alpha, alphaTarget, info.dt, fade, 1 / 512);
        const recolouring = colour != null && (state.rgb.r !== goal.r || state.rgb.g !== goal.g || state.rgb.b !== goal.b);
        if (recolouring) { state.rgb = approachColour(state.rgb, goal, blend); state.colour = toHex(state.rgb); }
        if (state.a0 !== target.a0 || state.a1 !== target.a1 || state.ring !== target.ring || state.alpha !== alphaTarget || recolouring) moving = true;
      }
      for (const id of motion.keys()) if (!present.has(id)) motion.delete(id);

      if (p.settings.growth === 'fill' || !(zoomed.value > 0)) {
        context.beginPath();
        context.arc(g.cx, g.cy, g.hole + g.ring, 0, Math.PI * 2);
        context.arc(g.cx, g.cy, g.hole, 0, Math.PI * 2, true);
        context.fillStyle = mix(tokens.surface, tokens.border, 0.45);
        context.fill('evenodd');
      }

      const slices: DrawnSlice[] = [];
      const top = -Math.PI / 2;
      for (const node of nodes) {
        const state = motion.get(node.id)!;
        if (state.alpha < 0.01 || state.a1 - state.a0 < 5e-4 || state.ring <= 0.02) continue;
        const inner = Math.max(0, g.hole + (state.ring - 1) * g.ring);
        const outer = Math.max(inner, inner + g.ring);
        if (inner > g.radius + g.ring) continue;
        slices.push({ id: node.id, a0: state.a0, a1: state.a1, inner, outer, alpha: state.alpha });
      }
      const pointed = pointer && !quiet.current ? sliceAt(slices, g, pointer.x, pointer.y) : null;
      const hotId = keyed.current ?? legendHot.current ?? (pointed !== 'centre' ? pointed : null);
      const hot = hotId ? byId.get(hotId) ?? null : null;
      const related = (node: SunburstNode) => !hot || node === hot || pathTo(hot).includes(node) || pathTo(node).includes(hot);
      for (const slice of slices) {
        const node = byId.get(slice.id)!;
        const state = motion.get(slice.id)!;
        const lifted = node === hot ? LIFT : 0;
        context.globalAlpha = slice.alpha * (p.settings.dimming && !related(node) ? DIM : 1);
        context.beginPath();
        context.arc(g.cx, g.cy, slice.outer + lifted, top + slice.a0, top + slice.a1);
        context.arc(g.cx, g.cy, slice.inner, top + slice.a1, top + slice.a0, true);
        context.closePath();
        context.fillStyle = state.colour;
        context.fill();
        context.lineWidth = GAP;
        context.strokeStyle = tokens.surface;
        context.stroke();
        if (p.settings.labels && slice.alpha > 0.6) {
          const labelling = font(tokens, tokens.fontSize.small, 600);
          if (labelling !== labelFont) { labelFont = labelling; labelWidths.clear(); }
          context.font = labelling;
          let width = labelWidths.get(node.label);
          if (width === undefined) { width = textWidth(context, node.label); labelWidths.set(node.label, width); }
          if (labelFits(slice, width, tokens.fontSize.small)) {
            const middle = top + (slice.a0 + slice.a1) / 2;
            const radius = (slice.inner + slice.outer) / 2;
            context.save();
            context.translate(g.cx + Math.cos(middle) * radius, g.cy + Math.sin(middle) * radius);
            context.rotate(middle + Math.PI / 2 + (Math.sin(middle) > 0 ? Math.PI : 0));
            const background = parseColor(state.colour);
            context.fillStyle = background ? readableText(background) : tokens.text;
            context.textAlign = 'center'; context.textBaseline = 'middle';
            context.fillText(node.label, 0, 0);
            context.restore();
          }
        }
      }
      context.globalAlpha = 1;
      drawn.current = { slices, geometry: g, pick: { owner: p.pickOwner, nodes, onPick: p.onPick, canPick: p.canPick } };

      const zoomedIn = zoomed !== root;
      context.beginPath();
      context.arc(g.cx, g.cy, Math.max(0, g.hole - 6), 0, Math.PI * 2);
      context.fillStyle = pointed === 'centre' && zoomedIn ? mix(tokens.surface, tokens.text, 0.06) : tokens.surface;
      context.fill();
      const big = Math.max(tokens.fontSize.body, Math.min(tokens.fontSize.title * 2.2, g.hole * 0.36));
      const lines = centreLines([
        { text: zoomed.label, size: tokens.fontSize.small, weight: 500, colour: tokens.muted, rank: 1 },
        { text: zoomed.valueUnknown ? 'Not known' : notMeasured(zoomed) ? p.text.missingLabel ?? 'Not measured' : amountText(zoomed, p.text), size: zoomed.valueUnknown || notMeasured(zoomed) ? tokens.fontSize.body : big, weight: 600, colour: tokens.text, rank: 0 },
        { text: p.text.shares !== false && p.settings.growth === 'fill' && !zoomed.valueUnknown && !zoomed.finalUnknown && !hasCorrections(zoomed) && zoomed.final > 0 ? `${p.text.unit} · ${shareText(zoomed.value, zoomed.final)} of the whole` : p.text.unit,
          size: tokens.fontSize.small, weight: 400, colour: tokens.muted, rank: 3 },
        ...(p.settings.growth === 'fill' && zoomed.finalUnknown && !zoomed.valueUnknown ? [{ text: 'Whole total not known', size: tokens.fontSize.small, weight: 400, colour: tokens.muted, rank: 4 }] : []),
        ...(unknownTotal(zoomed) && !zoomed.valueUnknown && !notMeasured(zoomed) ? [{ text: 'partial', size: tokens.fontSize.small, weight: 400, colour: tokens.muted, rank: 4 }] : []),
        ...(zoomedIn ? [{ text: '↑ back', size: tokens.fontSize.small, weight: 500, colour: pointed === 'centre' ? tokens.text : tokens.muted, rank: 2 }] : []),
      ], Math.max(0, g.hole - 8) * 1.7);
      let y = g.cy - lines.reduce((sum, line) => sum + line.size * LINE, 0) / 2;
      context.textAlign = 'center'; context.textBaseline = 'middle';
      const room = Math.max(0, g.hole - 8) * 1.8;
      for (const line of lines) {
        context.font = font(tokens, line.size, line.weight);
        context.fillStyle = line.colour;
        if (line.rank === 0) {
          const width = textWidth(context, line.text);
          if (width > room) context.font = font(tokens, Math.max(tokens.fontSize.small * 0.8, line.size * room / width), line.weight);
          context.fillText(line.text, g.cx, y + line.size * LINE / 2);
        } else context.fillText(fitText(line.text, room, text => textWidth(context, text)), g.cx, y + line.size * LINE / 2);
        y += line.size * LINE;
      }

      if (!keyed.current) {
        const node = pointer && pointed && pointed !== 'centre' ? byId.get(pointed) : null;
        if (node && pointer) {
          if (moved || free(owner)) {
            p.onHover?.(node);
            const content = fitTooltip(p.tooltip?.(node, zoomed, p.text) ?? sliceTooltip(node, zoomed, p.text), globalThis.innerHeight);
            showTooltip(owner, content, pointer.clientX, pointer.clientY, `${node.id}:${node.value}:${zoomed.id}`);
          }
        } else hideTooltip(owner);
        moved = false;
      }
      setStyle(element, 'cursor', pointed === 'centre' ? (zoomedIn ? 'pointer' : 'default')
        : pointed && zoomable(byId.get(pointed)!) ? 'zoom-in' : 'default');
      return moving;
    };

    const current: FrameTask = { frame: draw };
    task.current = current;
    const wake = () => scheduler.wake(current);
    const remove = scheduler.add(current, element);
    const unsubscribe = subscribeTokens(next => { tokens = next; cached = null; wake(); });
    const local = (event: PointerEvent | MouseEvent) => ({
      ...toLocal(event.clientX, event.clientY, element.getBoundingClientRect(), element.clientWidth, element.clientHeight),
      clientX: event.clientX, clientY: event.clientY,
    });
    const move = (event: PointerEvent) => {
      if (keyed.current) { keyed.current = null; setSpoken(''); }
      quiet.current = false;
      pointer = local(event);
      moved = true;
      wake();
    };
    const leave = () => { pointer = null; hideTooltip(owner); wake(); };
    const click = (event: MouseEvent) => {
      const at = drawn.current;
      if (!at) return;
      const point = local(event);
      const hit = sliceAt(at.slices, at.geometry, point.x, point.y);
      if (hit && hit !== 'centre' && pickSunburst(at.pick, latest.current.props.pickOwner, latest.current.props.active, hit)) return;
      const { tree: root, focus: zoomed } = latest.current;
      if (hit === 'centre') zoomTo(zoomed.parent ?? root);
      else if (hit) {
        const node = walk(root).find(item => item.id === hit);
        if (node && zoomable(node)) zoomTo(node);
      }
    };
    const contextMenu = (event: MouseEvent) => {
      const { props: p, tree: root } = latest.current;
      if (!p.onContextMenu) return;
      event.preventDefault();
      const at = drawn.current;
      const point = local(event);
      const hit = at && sliceAt(at.slices, at.geometry, point.x, point.y);
      const node = hit && hit !== 'centre' ? walk(root).find(item => item.id === hit) ?? null : null;
      p.onContextMenu(node, event.clientX, event.clientY);
    };
    element.addEventListener('pointermove', move);
    element.addEventListener('pointerleave', leave);
    element.addEventListener('click', click);
    element.addEventListener('contextmenu', contextMenu);
    return () => {
      remove(); unsubscribe(); sized.disconnect(); hideTooltip(owner);
      element.removeEventListener('pointermove', move);
      element.removeEventListener('pointerleave', leave);
      element.removeEventListener('click', click);
      element.removeEventListener('contextmenu', contextMenu);
      task.current = null;
    };
  }, [owner]);

  useEffect(() => {
    if (task.current) scheduler.setVisible(task.current, props.active);
    if (!props.active) hideTooltip(owner);
  }, [props.active, owner]);
  useEffect(() => { if (task.current) scheduler.wake(task.current); });
  useEffect(() => {
    const live = props.live;
    if (!live) return undefined;
    let seen: SunburstValues | undefined;
    return live.subscribe(() => {
      const values = live.read();
      if (values === seen) return;
      seen = values;
      if (task.current) scheduler.wake(task.current);
    });
  }, [props.live]);
  useLayoutEffect(() => { if (live) paintLegend(legendHost.current, live.focus, props.text); });

  const showKeyed = () => {
    const { tree: root, focus: zoomed, props: p } = latest.current;
    const node = keyed.current ? walk(root).find(item => item.id === keyed.current) : null;
    const element = canvas.current;
    const slice = node && drawn.current?.slices.find(item => item.id === node.id);
    if (!node || !element || !slice || !drawn.current) { keyed.current = null; hideTooltip(owner); setSpoken(''); return; }
    const g = drawn.current.geometry;
    const bounds = element.getBoundingClientRect();
    const scale = element.offsetWidth ? bounds.width / element.offsetWidth : 1;
    const middle = -Math.PI / 2 + (slice.a0 + slice.a1) / 2;
    const radius = slice.outer;
    p.onHover?.(node);
    const forms = p.tooltip?.(node, zoomed, p.text) ?? sliceTooltip(node, zoomed, p.text);
    showTooltip(owner, fitTooltip(forms, globalThis.innerHeight), bounds.left + (g.cx + Math.cos(middle) * radius) * scale,
      bounds.top + (g.cy + Math.sin(middle) * radius) * scale, `key:${node.id}:${node.value}:${zoomed.id}`);
    setSpoken(`${tooltipText(forms[0])}${p.onPick && p.canPick?.(node) !== false ? '; Enter shows breakdown' : zoomable(node) ? '; Enter zooms in' : ''}`);
  };
  useEffect(() => {
    if (!keyed.current) return;
    if (!props.active) { keyed.current = null; setSpoken(''); return; }
    if (free(owner)) showKeyed();
  });

  const keyDown = (event: ReactKeyboardEvent) => {
    const { tree: root, focus: zoomed, props: p } = latest.current;
    const steps: readonly string[] = ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'];
    if (steps.includes(event.key)) {
      event.preventDefault();
      const next = keyedSlice(event.key as SunKey, keyed.current, keyRings(root, arcTargets(root, zoomed, p.settings.growth, p.rings ?? 2)));
      keyed.current = next?.id ?? null;
      quiet.current = false;
      requestAnimationFrame(() => showKeyed());
      return;
    }
    const node = keyed.current ? walk(root).find(item => item.id === keyed.current) : null;
    if ((event.key === 'Enter' || event.key === ' ') && keyed.current && drawn.current?.slices.some(slice => slice.id === keyed.current)
      && pickSunburst(drawn.current.pick, p.pickOwner, p.active, keyed.current)) {
      event.preventDefault(); return;
    }
    if ((event.key === 'ContextMenu' || event.shiftKey && event.key === 'F10'
      || event.key === 'Enter' && !zoomable(node ?? root)) && p.onContextMenu) {
      event.preventDefault();
      const bounds = event.currentTarget.getBoundingClientRect();
      p.onContextMenu(node ?? null, bounds.left + bounds.width / 2, bounds.top + bounds.height / 2);
      return;
    }
    if ((event.key === 'Enter' || event.key === ' ') && node && zoomable(node)) {
      event.preventDefault();
      zoomTo(node);
      keyed.current = null; hideTooltip(owner); setSpoken(`Zoomed into ${node.label}`);
      return;
    }
    if (event.key === 'Escape' && keyed.current) { event.preventDefault(); keyed.current = null; quiet.current = true; hideTooltip(owner); setSpoken(''); return; }
    if ((event.key === 'Backspace' || event.key === 'Escape') && zoomed !== root) {
      event.preventDefault();
      zoomTo(zoomed.parent ?? root);
      keyed.current = null; hideTooltip(owner); setSpoken(`Zoomed out to ${(zoomed.parent ?? root).label}`);
    }
  };

  const shownTip = useRef(legendTip);
  legendShow.current = (shownFocus, fresh, speak) => {
    const part = legendTip && props.active && props.settings.legend ? shownFocus.children.find(child => child.id === legendTip.id) : null;
    if (part && legendTip) {
      if (speak) props.onHover?.(part);
      if (fresh || free(legendOwner)) {
        const content = props.tooltip?.(part, shownFocus, props.text) ?? sliceTooltip(part, shownFocus, props.text);
        if (legendTip.row && speak) setSpoken(tooltipText(content[0]));
        showTooltip(legendOwner, fitTooltip(content, globalThis.innerHeight), legendTip.x, legendTip.y,
          `legend:${part.id}:${part.value}:${JSON.stringify(part.unknown)}:${shownFocus.id}`);
      }
    } else if (speak) hideTooltip(legendOwner);
  };
  useEffect(() => {
    const fresh = shownTip.current !== legendTip;
    shownTip.current = legendTip;
    legendShow.current(latest.current.focus, fresh, true);
  }, [legendTip, focus, props.text, props.tooltip, props.active, props.settings.legend, legendOwner]);
  useEffect(() => () => hideTooltip(legendOwner), [legendOwner]);
  useEffect(() => {
    if (!legendTip) return;
    const resize = () => setLegendTip(tip => {
      if (!tip?.row) return tip && { ...tip };
      const box = tip.row.getBoundingClientRect();
      return { ...tip, x: box.left, y: box.bottom };
    });
    globalThis.addEventListener('resize', resize);
    return () => globalThis.removeEventListener('resize', resize);
  }, [legendTip == null]);

  const { settings, text } = props;
  const parts = useMemo(() => focus.children.filter(listed), [focus]);
  const legendRows = useMemo(() => parts.map(item => ({ key: item.id, item })), [parts]);
  const visibleLegend = useCallback((rows: readonly MovingRow<SunburstNode>[]) => {
    const ids = new Set(rows.filter(row => row.phase !== 'leaving').map(row => row.key));
    setLegendHover(id => id !== null && ids.has(id) ? id : null);
    setLegendTip(tip => tip !== null && ids.has(tip.id) ? tip : null);
  }, []);
  const artwork = parts.some(part => part.icon);
  const icons = artwork || parts.some(part => part.countryFlag !== undefined);
  const colours = useMemo(() => sliceColours(focus, tokens.categories, { light: tokens.text, dark: tokens.surface }), [focus, tokens]);
  const path = pathTo(focus);
  const showLegend = settings.legend || props.legendExtra != null;
  return <div className="sunburst-host"><div className={`sunburst${showLegend ? '' : ' no-legend'}`}>
    <div className="sunburst-plot">
      {settings.breadcrumb && path.length > 1 && <nav className="sunburst-crumbs" aria-label="Zoomed into">
        {path.map((step, index) => index === path.length - 1
          ? <b key={step.id} aria-current="location">{step.label}</b>
          : <span key={step.id}><button type="button" className="sunburst-crumb" onClick={() => zoomTo(step)}>{step.label}</button><i aria-hidden="true">›</i></span>)}
      </nav>}
      <canvas ref={canvas} tabIndex={0} role="img" aria-label={props.label} aria-describedby={`${owner}-summary`}
        aria-keyshortcuts="ArrowLeft ArrowRight ArrowUp ArrowDown Home End Enter Backspace Escape" onKeyDown={keyDown}
        onBlur={() => { keyed.current = null; hideTooltip(owner); setSpoken(''); }} />
      <p id={`${owner}-summary`} className="sr-only">{sunburstSummary(focus, text, props.summaryNotes)}</p>
      <p className="sr-only" aria-live="polite">{spoken}</p>
    </div>
    {showLegend && <div ref={legendHost} className="sunburst-legend">
      {settings.legend && <>
      <h3>{focus.label}</h3>
      {parts.length ? <WindowedRows key={focus.id} items={legendRows} rowHeight={tokens.fontSize.body * 2.5}
        label={`${focus.label} parts`} className={`sunburst-legend-rows${icons ? ' has-icons' : ''}`} onVisibleRows={visibleLegend}>
      {({ item: part }) => {
        const zoom = zoomable(part);
        const pick = props.onPick !== undefined && props.canPick?.(part) !== false;
        const body = <>
          <i className={`sunburst-swatch${colours.has(part.id) ? '' : ' is-blank'}`} aria-hidden="true" style={colours.has(part.id) ? { background: colours.get(part.id) } : undefined} />
          {icons && (part.countryFlag !== undefined
            ? <CountryFlagUrl className="sunburst-icon" url={part.countryFlag.flag} detail={part.countryFlag.flagDetail} reason={part.countryFlag.flagReason} pending={part.countryFlag.flagPending} />
            : part.icon ? <img className="sunburst-icon" src={part.icon} alt="" />
            : null)}
          <span className="sunburst-name" data-detail={part.countryFlag?.nameFailure?.message} data-tooltip={part.label.length > 18 ? part.label : undefined}>{part.label}</span>
          <b>{props.live ? <span data-sunburst-amount={part.id}>{legendAmount(part, text)}</span> : legendAmount(part, text)}{unknownTotal(part) ? <small className="sunburst-partial"> partial</small> : null}</b>
          <small data-sunburst-share={props.live ? part.id : undefined}>{legendShare(part, focus, text)}</small>
        </>;
        const events = {
          onPointerEnter: (event: ReactPointerEvent) => { setLegendHover(part.id); setLegendTip({ id: part.id, x: event.clientX, y: event.clientY }); },
          onPointerLeave: () => { setLegendHover(null); setLegendTip(null); },
          onFocus: (event: ReactFocusEvent<HTMLElement>) => {
            const box = event.currentTarget.getBoundingClientRect();
            setLegendHover(part.id); setLegendTip({ id: part.id, x: box.left, y: box.bottom, row: event.currentTarget });
          },
          onBlur: () => { setLegendHover(null); setLegendTip(null); setSpoken(''); },
          onContextMenu: (event: ReactMouseEvent) => {
            if (!props.onContextMenu) return;
            event.preventDefault(); props.onContextMenu(part, event.clientX, event.clientY);
          },
          onKeyDown: (event: ReactKeyboardEvent) => {
            if (!props.onContextMenu || !(event.key === 'ContextMenu' || event.shiftKey && event.key === 'F10' || !zoom && !pick && event.key === 'Enter')) return;
            event.preventDefault();
            const box = event.currentTarget.getBoundingClientRect(); props.onContextMenu(part, box.left, box.bottom);
          },
        };
        const said = `${part.label}. ${part.valueUnknown ? 'Not known' : notMeasured(part) ? text.missingLabel ?? 'Not measured' : `${amountText(part, text)} ${text.unit}${unknownTotal(part) ? ', partial' : ''}`}`;
        return <div className={part.valueUnknown || notMeasured(part) ? 'is-unmeasured' : undefined}>
          {zoom || pick ? <button type="button" tabIndex={0} {...events} onClick={() => {
            if (pick && props.active && props.onPick?.(part)) return;
            if (zoom) zoomTo(part);
          }} aria-label={`${said}. ${pick ? 'Show breakdown' : 'Zoom in'}`}>{body}</button>
            : <div {...events} tabIndex={0} role="group" aria-label={said}>{body}</div>}
        </div>;
      }}</WindowedRows> : <p className="sunburst-empty">{structure.unavailableReason ?? text.emptyLabel ?? 'Nothing yet.'}</p>}
      </>}
      {props.legendExtra != null && <div className="sunburst-legend-extra">{props.legendExtra}</div>}
      {props.footnote && <p className="sunburst-footnote">{props.footnote}</p>}
    </div>}
    {!showLegend && props.footnote && <p className="sunburst-footnote is-under">{props.footnote}</p>}
  </div></div>;
}
