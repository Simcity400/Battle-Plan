import { useEffect, useId, useRef, useState } from 'react';
import type { MenuItem } from '../../components/ContextMenu';
import { useContextMenu } from '../../components/ContextMenu';
import { canvasImage } from '../images';
import { countryFlagContent, drawCountryFlag } from '../countryFlag';
import type { CountryFlagLook } from '../../components/CountryFlag';
import { revealRow } from '../../components/countryList';
import { approach, approachAcrossGap, easeTau } from '../easing';
import { nextRows } from '../rowMotion';
import type { MovingRow } from '../rowMotion';
import { scheduler } from '../scheduler';
import { SECONDARY_DASH } from '../palette';
import type { FrameTask } from '../scheduler';
import { backingSize, toLocal } from '../surface';
import { fitTooltip, hideTooltip, showTooltip, tooltipStore, tooltipText } from '../tooltip';
import type { TooltipContent } from '../tooltip';
import { useChartTokens, useViewportHeight } from '../useChartTokens';
import { butterflyLabelWidth, butterflyLayout, butterflyMinimumHeight, butterflyRowViewport, butterflyScale, butterflySurface, butterflyTextWidths, ellipsizeLabel } from './layout';
import type { ButterflyValue } from './layout';
import '../charts.css';
import './butterfly.css';

export interface ButterflyItem extends ButterflyValue {
  label: string; flag?: string; countryFlag?: CountryFlagLook; color: string;
  /** Already formatted exact values; shape easing never changes these strings. */
  leftText: string; rightText: string; ratioText: string;
  /** Identifies an unbroken data stretch. Changing it prevents easing across a gap. */
  segment: string;
}
export interface ButterflyProps<T extends ButterflyItem> {
  label: string; items: readonly T[];
  /** All names shown by this request, keeping the label column still in playback. */
  labelCandidates?: readonly string[];
  leftLabel: string; rightLabel: string; ratioLabel: string;
  showValues?: boolean; showRatio?: boolean; showFlags?: boolean;
  active: boolean; hoursPerSecond?: number;
  tooltip?: (item: T) => TooltipContent | null;
  contextItems?: (item: T) => readonly MenuItem[];
  /** Enter opens the left value; Shift+Enter opens the right. */
  onSelect?: (item: T, side: 'left' | 'right') => void;
}

interface Shape { slot: number; left: number | null; right: number | null; segment: string }

/** Paired bars, faded on the left and full on the right, sharing one symmetric scale.
 * Callers own values, wording, ordering and dates; the kit owns motion and interaction. */
export default function ButterflyChart<T extends ButterflyItem>(props: ButterflyProps<T>) {
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const owner = useId();
  const root = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const task = useRef<FrameTask | null>(null);
  const latest = useRef({ props, tokens, viewport });
  latest.current = { props, tokens, viewport };
  const hits = useRef<{ item: T; y: number; slot: number }[]>([]);
  const headerHeight = useRef(0);
  const rightEdge = useRef(0);
  const keyed = useRef<string | null>(null);
  const keyIntent = useRef(false);
  const pointer = useRef<{ y: number; x: number; clientX: number; clientY: number } | null>(null);
  const [spoken, setSpoken] = useState('');
  const menu = useContextMenu(`${props.label} actions`);
  const menuKey = useRef(false);

  useEffect(() => {
    const element = canvas.current!;
    const ctx = element.getContext('2d');
    if (!ctx) return;
    let rows: MovingRow<T>[] = [];
    let items: readonly T[] | null = null;
    const shapes = new Map<string, Shape>();
    let scale: number | null = null;
    let measuredLabels: readonly string[] | readonly T[] | undefined;
    let measuredFont = '';
    let measuredText: { items: readonly T[]; font: string; values: boolean; ratio: boolean; label: string; valueWidth: number; ratioWidth: number } | undefined;
    let longestName = 0;
    const shortened = new Map<string, string>();
    let shortWidth = -1, shortFont = '';
    const loaded = () => { if (task.current) scheduler.wake(task.current); };
    const frameTask: FrameTask = { frame: ({ dt, now }) => {
      const { props: p, tokens: t, viewport: room } = latest.current;
      const tau = easeTau(t.motion, p.hoursPerSecond ?? 0, t.reducedMotion);
      if (items !== p.items) {
        rows = nextRows(rows, p.items.map(item => ({ key: item.id, item })), now, tau * 3, items == null);
        items = p.items;
      } else rows = rows.filter(row => row.phase !== 'leaving' || now - row.since < tau * 3);
      const ids = new Set(rows.map(row => row.key));
      for (const id of shapes.keys()) if (!ids.has(id)) shapes.delete(id);
      const container = root.current!;
      const surface = butterflySurface(p.items.length, t.fontSize.body, container.clientHeight);
      const canvasHeight = `${surface.canvasHeight}px`;
      if (element.style.height !== canvasHeight) element.style.height = canvasHeight;
      const width = element.clientWidth, height = element.clientHeight, scrollTop = container.scrollTop;
      const rect = element.getBoundingClientRect();
      const size = backingSize({ layoutWidth: width, layoutHeight: height, rectWidth: rect.width, rectHeight: rect.height, devicePixelRatio: window.devicePixelRatio || 1 });
      if (element.width !== size.width) element.width = size.width;
      if (element.height !== size.height) element.height = size.height;
      ctx.setTransform(size.scaleX, 0, 0, size.scaleY, 0, 0);
      ctx.clearRect(0, 0, width, height);
      ctx.font = `${t.fontSize.small}px ${t.font}`;
      const values = p.showValues !== false, ratio = p.showRatio !== false, flags = p.showFlags !== false;
      if (!measuredText || measuredText.items !== p.items || measuredText.font !== ctx.font
        || measuredText.values !== values || measuredText.ratio !== ratio || measuredText.label !== p.ratioLabel) {
        measuredText = { items: p.items, font: ctx.font, values, ratio, label: p.ratioLabel,
          ...butterflyTextWidths(p.items, text => ctx.measureText(text).width, p.ratioLabel, values, ratio) };
      }
      const { valueWidth, ratioWidth } = measuredText;
      const candidates = p.labelCandidates ?? p.items;
      const labelFont = `${t.fontSize.body}px ${t.font}`;
      if (measuredLabels !== candidates || measuredFont !== labelFont) {
        ctx.font = labelFont;
        longestName = 0;
        for (const candidate of candidates) longestName = Math.max(longestName, ctx.measureText(typeof candidate === 'string' ? candidate : candidate.label).width);
        measuredLabels = candidates; measuredFont = labelFont;
      }
      const labelWidth = butterflyLabelWidth(width, t.fontSize.body, longestName, flags);
      const minimumBarWidth = t.fontSize.body * 3;
      const edge = t.fontSize.body * 2 / 3, gap = t.fontSize.body / 2;
      const ratioRoom = ratioWidth ? ratioWidth + gap * 2 : 0;
      const minimumWidth = labelWidth + 2 * (minimumBarWidth + valueWidth + edge + gap + ratioRoom);
      const minimum = `${Math.ceil(minimumWidth)}px`;
      if (element.style.minWidth !== minimum) element.style.minWidth = minimum;
      const layout = butterflyLayout(width, surface.contentHeight, p.items.length, t.fontSize.body, labelWidth, valueWidth, ratioWidth);
      headerHeight.current = layout.top;
      const visible = butterflyRowViewport(layout.top, height, scrollTop);
      const shortRoom = layout.labelWidth - t.fontSize.body * (flags ? 3 : 1);
      if (shortWidth !== shortRoom || shortFont !== labelFont) { shortened.clear(); shortWidth = shortRoom; shortFont = labelFont; }
      const targetScale = butterflyScale(p.items);
      scale = approach(scale ?? targetScale, targetScale, dt, tau);
      let moving = scale !== targetScale;
      ctx.textBaseline = 'middle';
      ctx.save();
      ctx.beginPath(); ctx.rect(0, layout.top, width, visible.height); ctx.clip();
      ctx.translate(0, -scrollTop);
      hits.current = [];
      for (const row of rows) {
        const item = row.item;
        const shape = shapes.get(row.key) ?? { slot: row.slot, left: item.left, right: item.right, segment: item.segment };
        const gap = shape.segment !== item.segment;
        shape.slot = approach(shape.slot, row.slot, dt, tau);
        shape.left = approachAcrossGap(shape.left, item.left, dt, tau, gap);
        shape.right = approachAcrossGap(shape.right, item.right, dt, tau, gap);
        shape.segment = item.segment; shapes.set(row.key, shape);
        moving ||= shape.slot !== row.slot || shape.left !== item.left || shape.right !== item.right;
        const opacity = row.phase === 'leaving' ? Math.max(0, 1 - (now - row.since) / (tau * 3)) : row.phase === 'entering' && tau > 0 ? Math.min(1, (now - row.since) / tau) : 1;
        moving ||= opacity < 1;
        const y = layout.top + (shape.slot + 0.5) * layout.slot;
        if (row.phase !== 'leaving') hits.current.push({ item, y, slot: layout.slot });
        if (y + layout.slot / 2 < visible.start || y - layout.slot / 2 > visible.end) continue;
        rightEdge.current = layout.right;
        const l = Math.min(layout.half, Math.abs(shape.left ?? 0) / scale * layout.half), r = Math.min(layout.half, Math.abs(shape.right ?? 0) / scale * layout.half);
        const selected = keyed.current === item.id || pointer.current && pointer.current.y >= layout.top && Math.abs(pointer.current.y + scrollTop - y) < layout.slot / 2;
        if (selected && row.phase !== 'leaving') { ctx.globalAlpha = opacity; ctx.fillStyle = t.raised; ctx.fillRect(0, y - layout.slot / 2, width, layout.slot); }
        ctx.fillStyle = item.color;
        const radius = t.fontSize.small / 3;
        const bar = (x: number, size: number, value: number | null, alpha: number, radii: number[]) => {
          if (value == null) return;
          ctx.globalAlpha = opacity * alpha; ctx.beginPath(); ctx.roundRect(x, y - layout.barHeight / 2, size, layout.barHeight, radii); ctx.fill();
          if (value < 0) { ctx.save(); ctx.setLineDash([...SECONDARY_DASH]); ctx.strokeStyle = t.text; ctx.lineWidth = t.lineWidth.series; ctx.stroke(); ctx.restore(); }
        };
        bar(layout.left - l, l, shape.left, 0.5, [radius, 0, 0, radius]);
        bar(layout.right, r, shape.right, 1, [0, radius, radius, 0]);
        ctx.globalAlpha = opacity; ctx.fillStyle = t.text; ctx.font = `${t.fontSize.body}px ${t.font}`; ctx.textAlign = 'left';
        const flagX = layout.left + t.fontSize.body / 2, flagWidth = t.fontSize.body * 1.5, flagHeight = t.fontSize.body;
        if (flags) {
          if (item.countryFlag) drawCountryFlag(ctx, item.countryFlag, flagX, y - flagHeight / 2, flagWidth, flagHeight, t, loaded);
          else {
            const flag = item.flag ? canvasImage(item.flag, loaded) : null;
            if (flag) ctx.drawImage(flag, flagX, y - flagHeight / 2, flagWidth, flagHeight);
            else { ctx.strokeStyle = t.border; ctx.lineWidth = t.lineWidth.grid; ctx.strokeRect(flagX, y - flagHeight / 2, flagWidth, flagHeight); }
          }
        }
        let short = shortened.get(item.label);
        if (short === undefined) { short = ellipsizeLabel(item.label, shortRoom, text => ctx.measureText(text).width); shortened.set(item.label, short); }
        ctx.fillText(short, flagX + (flags ? flagWidth + t.fontSize.body / 2 : 0), y);
        ctx.font = `${t.fontSize.small}px ${t.font}`; ctx.fillStyle = t.muted;
        if (p.showValues !== false || item.left == null) { ctx.textAlign = 'right'; ctx.fillText(item.leftText, layout.left - l - radius, y); }
        if (p.showValues !== false || item.right == null) { ctx.textAlign = 'left'; ctx.fillText(item.rightText, layout.right + r + radius, y); }
        if (p.showRatio !== false) { ctx.fillStyle = t.text; ctx.textAlign = 'right'; ctx.fillText(item.ratioText, layout.ratioX, y); }
      }
      ctx.restore();
      ctx.globalAlpha = 1;
      ctx.font = `${t.fontSize.small}px ${t.font}`; ctx.fillStyle = t.muted;
      ctx.textAlign = 'right'; ctx.fillText(p.leftLabel, layout.left - t.fontSize.small / 2, layout.top / 2);
      ctx.textAlign = 'left'; ctx.fillText(p.rightLabel, layout.right + t.fontSize.small / 2, layout.top / 2);
      if (ratio) { ctx.textAlign = 'right'; ctx.fillText(p.ratioLabel, layout.ratioX, layout.top / 2); }
      const hit = keyed.current ? hits.current.find(hit => hit.item.id === keyed.current)
        : pointer.current && pointer.current.y >= layout.top ? hits.current.find(hit => Math.abs(hit.y - pointer.current!.y - scrollTop) < hit.slot / 2) : undefined;
      const heldTooltip = tooltipStore.get();
      if (!keyed.current || keyIntent.current || heldTooltip == null || heldTooltip.owner === owner) {
        const content = hit && hit.y + hit.slot / 2 >= visible.start && hit.y - hit.slot / 2 <= visible.end
          ? p.tooltip?.(hit.item) ?? null : null;
        const flag = flags ? hit?.item.countryFlag : undefined;
        const raw = content && flag ? { ...content, countryFlag: flag } : content;
        const decoded = raw ? countryFlagContent(fitTooltip([raw], room, { keep: 1 }), undefined, loaded) : null;
        const fitted = decoded ? fitTooltip([decoded], room, { keep: 1 }) : null;
        if (fitted) {
          showTooltip(owner, fitted, pointer.current?.clientX ?? rect.left + rect.width / 2,
            pointer.current?.clientY ?? rect.top + (hit!.y - scrollTop) * rect.height / Math.max(1, height));
          if (keyed.current) setSpoken(tooltipText(fitted));
        } else { hideTooltip(owner); setSpoken(''); }
      }
      keyIntent.current = false;
      return moving;
    } };
    task.current = frameTask;
    const remove = scheduler.add(frameTask);
    scheduler.setVisible(frameTask, latest.current.props.active);
    const observer = new ResizeObserver(() => scheduler.wake(frameTask));
    observer.observe(element);
    observer.observe(root.current!);
    return () => { remove(); observer.disconnect(); task.current = null; hideTooltip(owner); };
  }, [owner]);
  useEffect(() => {
    if (task.current) { scheduler.setVisible(task.current, props.active); scheduler.wake(task.current); }
    if (!props.active) hideTooltip(owner);
  }, [props, tokens, viewport, owner]);
  const wake = () => { if (task.current) scheduler.wake(task.current); };
  const leave = () => { pointer.current = null; keyed.current = null; setSpoken(''); hideTooltip(owner); wake(); };
  const open = (item: T, x: number, y: number) => { const list = props.contextItems?.(item); if (list) menu.open(x, y, list); };
  return <div ref={root} className="butterfly-chart themed-scrollbar" onScroll={wake}>
    <div className="butterfly-content" style={{ minHeight: butterflyMinimumHeight(props.items.length, tokens.fontSize.body) }}>
      <canvas ref={canvas} tabIndex={0} role="img" aria-label={props.label}
        onPointerDown={() => { menuKey.current = false; }}
        onClick={event => {
          if (!props.onSelect) return;
          const element = canvas.current!, rect = element.getBoundingClientRect();
          const point = toLocal(event.clientX, event.clientY, rect, element.clientWidth, element.clientHeight);
          const hit = point.y >= headerHeight.current ? hits.current.find(hit => Math.abs(hit.y - point.y - root.current!.scrollTop) < hit.slot / 2) : undefined;
          if (hit) props.onSelect(hit.item, point.x >= rightEdge.current ? 'right' : 'left');
        }}
        onPointerMove={event => {
          const element = canvas.current!, rect = element.getBoundingClientRect();
          pointer.current = { ...toLocal(event.clientX, event.clientY, rect, element.clientWidth, element.clientHeight), clientX: event.clientX, clientY: event.clientY };
          keyed.current = null; setSpoken(''); wake();
        }} onPointerLeave={() => { pointer.current = null; if (!keyed.current) hideTooltip(owner); wake(); }} onBlur={leave}
        onContextMenu={event => {
          event.preventDefault();
          if (menuKey.current) { menuKey.current = false; return; }
          const element = canvas.current!, rect = element.getBoundingClientRect();
          const point = toLocal(event.clientX, event.clientY, rect, element.clientWidth, element.clientHeight);
          const hit = keyed.current ? hits.current.find(hit => hit.item.id === keyed.current)
            : point.y >= headerHeight.current ? hits.current.find(hit => Math.abs(hit.y - point.y - root.current!.scrollTop) < hit.slot / 2) : undefined;
          if (hit) open(hit.item, event.clientX, event.clientY);
        }} onKeyDown={event => {
          const items = props.items;
          if (event.key === 'Escape') { event.preventDefault(); leave(); return; }
          if (['ArrowDown', 'ArrowUp', 'ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) {
            event.preventDefault(); if (!items.length) return;
            const index = items.findIndex(item => item.id === keyed.current);
            const next = event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1 : index < 0 ? 0
              : Math.max(0, Math.min(items.length - 1, index + (event.key === 'ArrowUp' || event.key === 'ArrowLeft' ? -1 : 1)));
            keyed.current = items[next].id; pointer.current = null; keyIntent.current = true;
            const hit = hits.current.find(hit => hit.item.id === keyed.current);
            const container = root.current;
            if (hit && container) container.scrollTop = Math.max(0, revealRow((hit.y - headerHeight.current) / hit.slot - 0.5,
              container.scrollTop, butterflyRowViewport(headerHeight.current, container.clientHeight, container.scrollTop).height, hit.slot));
            wake();
          } else if (event.key === 'Enter' && props.onSelect) {
            const hit = hits.current.find(hit => hit.item.id === keyed.current);
            if (hit) { event.preventDefault(); props.onSelect(hit.item, event.shiftKey ? 'right' : 'left'); }
          } else if (event.key === 'ContextMenu' || event.key === 'F10' && event.shiftKey) {
            const hit = hits.current.find(hit => hit.item.id === keyed.current);
            if (hit) { event.preventDefault(); menuKey.current = true; const rect = canvas.current!.getBoundingClientRect(); open(hit.item, rect.left + rect.width / 2, rect.top + (hit.y - root.current!.scrollTop) * rect.height / Math.max(1, canvas.current!.clientHeight)); }
          }
        }} />
    </div>
    <p className="sr-only" aria-live="polite">{spoken}</p>
    {menu.menu}
  </div>;
}
