import { useEffect, useId, useRef, useState } from 'react';
import type { MenuItem } from '../../components/ContextMenu';
import { useContextMenu } from '../../components/ContextMenu';
import { canvasImage } from '../images';
import { countryFlagContent, drawCountryFlag } from '../countryFlag';
import type { CountryFlagLook } from '../../components/CountryFlag';
import { revealRow } from '../../components/countryList';
import { approach, approachAcrossGap, easeTau } from '../easing';
import { useMotionNow } from '../liveMotion.ts';
import { nextRows } from '../rowMotion';
import type { MovingRow } from '../rowMotion';
import type { Live } from '../live';
import { scheduler } from '../scheduler';
import { SECONDARY_DASH } from '../palette';
import type { FrameTask } from '../scheduler';
import { backingSize, drawingSurface, sizeCanvas, toLocal } from '../surface';
import { fitTooltip, hideTooltip, showTooltip, tooltipStore, tooltipText } from '../tooltip';
import type { TooltipContent } from '../tooltip';
import { useChartTokens, useViewportHeight } from '../useChartTokens';
import { font, LABEL_WEIGHT, VALUE_WEIGHT } from '../draw';
import { barPath, fillHatched, FLAG, NAME_PAD, VALUE_GAP } from '../race/draw';
import { formatAxisValue } from '../valueAxis';
import { butterflyLabelWidth, butterflyLayout, butterflyMinimumHeight, butterflyMinimumWidth, butterflyNameX, butterflyPartAt, butterflyPartSpans, butterflyRowViewport, butterflyScale, butterflySurface, butterflyTextWidths, butterflyTicks, ellipsizeLabel } from './layout';
import type { ButterflyPart, ButterflyPartHit, ButterflyValue } from './layout';
import '../charts.css';
import './butterfly.css';

export interface ButterflyItem extends ButterflyValue {
  label: string; flag?: string; countryFlag?: CountryFlagLook; color: string;
  leftText: string; rightText: string; ratioText: string;
  segment: string;
  leftParts?: readonly ButterflyPart[]; rightParts?: readonly ButterflyPart[];
}
export interface ButterflyProps<T extends ButterflyItem> {
  label: string; items: readonly T[];
  slots?: number;
  labelCandidates?: readonly string[];
  leftLabel: string; rightLabel: string; ratioLabel: string;
  formatTick?: (value: number, ticks: readonly number[]) => string;
  axis?: boolean;
  showValues?: boolean; showRatio?: boolean; showFlags?: boolean;
  active: boolean; hoursPerSecond?: number;
  tooltip?: (item: T, part?: ButterflyPartHit | null) => TooltipContent | null;
  contextItems?: (item: T) => readonly MenuItem[];
  onSelect?: (item: T, side: 'left' | 'right') => void;
  liveItems?: Live<readonly T[]>;
  empty?: string;
}

interface Shape { slot: number; left: number | null; right: number | null; segment: string }
interface Span { from: number; to: number }

export default function ButterflyChart<T extends ButterflyItem>(props: ButterflyProps<T>) {
  const tokens = useChartTokens();
  const motionNow = useMotionNow();
  const viewport = useViewportHeight();
  const owner = useId();
  const root = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const task = useRef<FrameTask | null>(null);
  const latest = useRef({ props, tokens, viewport });
  latest.current = { props, tokens, viewport };
  const hits = useRef<{ item: T; y: number; slot: number; left: readonly Span[]; right: readonly Span[] }[]>([]);
  const headerHeight = useRef(0);
  const rowsEnd = useRef(0);
  const axisX = useRef(0);
  const keyed = useRef<string | null>(null);
  const keyIntent = useRef(false);
  const pointer = useRef<{ y: number; x: number; clientX: number; clientY: number } | null>(null);
  const [spoken, setSpoken] = useState('');
  const menu = useContextMenu(`${props.label} actions`);
  const menuKey = useRef(false);

  useEffect(() => {
    const element = canvas.current!, container = root.current!;
    const drawing = drawingSurface(element), ctx = drawing.context;
    if (!ctx) return;
    let geometry: { room: number; width: number; height: number; rectWidth: number; rectHeight: number; scrollTop: number } | null = null;
    const measure = () => {
      const rect = element.getBoundingClientRect();
      return geometry = { room: container.clientHeight, width: element.clientWidth, height: element.clientHeight, rectWidth: rect.width, rectHeight: rect.height, scrollTop: container.scrollTop };
    };
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
      const { props: latestProps, tokens: t, viewport: room } = latest.current;
      const p = latestProps.liveItems ? { ...latestProps, items: latestProps.liveItems.read() } : latestProps;
      const tau = easeTau(t.motion, motionNow(p.hoursPerSecond), t.reducedMotion);
      if (items !== p.items) {
        rows = nextRows(rows, p.items.map(item => ({ key: item.id, item })), now, tau * 3, items == null);
        items = p.items;
      } else rows = rows.filter(row => row.phase !== 'leaving' || now - row.since < tau * 3);
      const ids = new Set(rows.map(row => row.key));
      for (const id of shapes.keys()) if (!ids.has(id)) shapes.delete(id);
      const laid = geometry ?? measure();
      const surface = butterflySurface(p.items.length, laid.room);
      const canvasHeight = `${surface.canvasHeight}px`;
      if (element.style.height !== canvasHeight) element.style.height = canvasHeight;
      const { width, height, scrollTop } = laid;
      const size = backingSize({ layoutWidth: width, layoutHeight: height, rectWidth: laid.rectWidth, rectHeight: laid.rectHeight, devicePixelRatio: window.devicePixelRatio || 1 });
      sizeCanvas(drawing.canvas, size.width, size.height);
      ctx.setTransform(size.scaleX, 0, 0, size.scaleY, 0, 0);
      ctx.clearRect(0, 0, width, height);
      const labelFont = font(t, t.fontSize.body, LABEL_WEIGHT), valueFont = font(t, t.fontSize.body, VALUE_WEIGHT), axisFont = font(t, t.fontSize.small);
      const values = p.showValues !== false, ratio = p.showRatio !== false, flags = p.showFlags !== false;
      if (!measuredText || measuredText.items !== p.items || measuredText.font !== valueFont
        || measuredText.values !== values || measuredText.ratio !== ratio || measuredText.label !== p.ratioLabel) {
        ctx.font = valueFont;
        measuredText = { items: p.items, font: valueFont, values, ratio, label: p.ratioLabel,
          ...butterflyTextWidths(p.items, text => ctx.measureText(text).width, p.ratioLabel, values, ratio) };
      }
      const { valueWidth, ratioWidth } = measuredText;
      const candidates = p.labelCandidates ?? p.items;
      if (measuredLabels !== candidates || measuredFont !== labelFont) {
        ctx.font = labelFont;
        longestName = 0;
        for (const candidate of candidates) longestName = Math.max(longestName, ctx.measureText(typeof candidate === 'string' ? candidate : candidate.label).width);
        measuredLabels = candidates; measuredFont = labelFont;
      }
      const labelWidth = butterflyLabelWidth(width, longestName, flags);
      const minimum = `${Math.ceil(butterflyMinimumWidth(labelWidth, valueWidth, ratioWidth))}px`;
      if (element.style.minWidth !== minimum) element.style.minWidth = minimum;
      const layout = butterflyLayout(width, surface.contentHeight, Math.max(p.slots ?? 0, p.items.length), labelWidth, valueWidth, ratioWidth);
      headerHeight.current = layout.top;
      axisX.current = layout.axis;
      const visible = butterflyRowViewport(layout.top, height, scrollTop);
      const nameX = butterflyNameX(flags);
      const shortRoom = layout.labelWidth - nameX - NAME_PAD;
      if (shortWidth !== shortRoom || shortFont !== labelFont) { shortened.clear(); shortWidth = shortRoom; shortFont = labelFont; }
      const targetScale = butterflyScale(p.items);
      scale = approach(scale ?? targetScale, targetScale, dt, tau);
      let moving = scale !== targetScale;
      const silent = !!p.empty && !rows.length;
      const ticks = p.axis === false || silent ? [] : butterflyTicks(scale, layout.half);
      const format = p.formatTick ?? formatAxisValue;
      const rowsBottom = layout.top + visible.height;
      rowsEnd.current = rowsBottom;
      ctx.lineWidth = t.lineWidth.grid; ctx.strokeStyle = t.grid; ctx.font = axisFont; ctx.fillStyle = t.muted;
      ctx.textAlign = 'center'; ctx.textBaseline = 'top';
      for (const tick of ticks) for (const side of tick ? [-1, 1] : [1]) {
        const x = Math.round(layout.axis + side * tick / scale * layout.half) + 0.5;
        ctx.beginPath(); ctx.moveTo(x, layout.top); ctx.lineTo(x, rowsBottom); ctx.stroke();
        ctx.fillText(format(tick, ticks), x, rowsBottom + 6);
      }
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
        const l = Math.min(layout.half, Math.abs(shape.left ?? 0) / scale * layout.half), r = Math.min(layout.half, Math.abs(shape.right ?? 0) / scale * layout.half);
        const leftSpans = butterflyPartSpans(item.leftParts, l), rightSpans = butterflyPartSpans(item.rightParts, r);
        if (row.phase !== 'leaving') hits.current.push({ item, y, slot: layout.slot, left: leftSpans, right: rightSpans });
        if (y + layout.slot / 2 < visible.start || y - layout.slot / 2 > visible.end) continue;
        const selected = keyed.current === item.id || pointer.current && pointer.current.y >= layout.top && pointer.current.y <= rowsBottom
          && Math.abs(pointer.current.y + scrollTop - y) < layout.slot / 2;
        if (selected && row.phase !== 'leaving') { ctx.globalAlpha = opacity * 0.05; ctx.fillStyle = t.text; ctx.fillRect(0, y - layout.slot / 2, width, layout.slot); }
        const barTop = y - layout.barHeight / 2;
        const bar = (size: number, value: number | null, side: -1 | 1, parts: readonly ButterflyPart[] | undefined, spans: readonly Span[]) => {
          if (value == null) return;
          ctx.globalAlpha = opacity; ctx.fillStyle = item.color;
          barPath(ctx, layout.axis, barTop, side * size, layout.barHeight);
          if (parts?.length) {
            ctx.save(); ctx.clip();
            spans.forEach((span, index) => {
              const part = parts[index], width = span.to - span.from;
              if (width <= 0) return;
              const x0 = side > 0 ? layout.axis + span.from : layout.axis - span.to;
              if (part.hatched) fillHatched(ctx, x0, barTop, width, layout.barHeight, part.color, 4, 1.5);
              else { ctx.fillStyle = part.color; ctx.fillRect(x0, barTop, width, layout.barHeight); }
              if (span.from > 0.5) { ctx.fillStyle = t.surface; ctx.fillRect(layout.axis + side * span.from - 0.5, barTop, 1, layout.barHeight); }
            });
            ctx.setLineDash([...SECONDARY_DASH]); ctx.strokeStyle = t.text; ctx.lineWidth = t.lineWidth.series;
            spans.forEach((span, index) => {
              const width = span.to - span.from;
              if (parts[index].negative && width > 0) ctx.strokeRect(side > 0 ? layout.axis + span.from : layout.axis - span.to, barTop, width, layout.barHeight);
            });
            ctx.restore();
            barPath(ctx, layout.axis, barTop, side * size, layout.barHeight);
          } else ctx.fill();
          if (value < 0) { ctx.save(); ctx.setLineDash([...SECONDARY_DASH]); ctx.strokeStyle = t.text; ctx.lineWidth = t.lineWidth.series; ctx.stroke(); ctx.restore(); }
        };
        bar(l, shape.left, -1, item.leftParts, leftSpans);
        bar(r, shape.right, 1, item.rightParts, rightSpans);
        if (l > 0.5 && r > 0.5) { ctx.globalAlpha = opacity; ctx.fillStyle = t.surface; ctx.fillRect(layout.axis - 0.5, barTop, 1, layout.barHeight); }
        ctx.globalAlpha = opacity;
        if (flags) {
          if (item.countryFlag) drawCountryFlag(ctx, item.countryFlag, FLAG.x, y - FLAG.height / 2, FLAG.width, FLAG.height, t, loaded);
          else {
            const flag = item.flag ? canvasImage(item.flag, loaded) : null;
            if (flag) ctx.drawImage(flag, FLAG.x, y - FLAG.height / 2, FLAG.width, FLAG.height);
          }
        }
        ctx.fillStyle = t.text; ctx.font = labelFont; ctx.textAlign = 'left';
        let short = shortened.get(item.label);
        if (short === undefined) { short = ellipsizeLabel(item.label, shortRoom, text => ctx.measureText(text).width); shortened.set(item.label, short); }
        ctx.fillText(short, nameX, y);
        ctx.font = valueFont;
        if (values || item.left == null) { ctx.fillStyle = item.left == null ? t.muted : t.text; ctx.textAlign = 'right'; ctx.fillText(item.leftText, layout.axis - l - VALUE_GAP, y); }
        if (values || item.right == null) { ctx.fillStyle = item.right == null ? t.muted : t.text; ctx.textAlign = 'left'; ctx.fillText(item.rightText, layout.axis + r + VALUE_GAP, y); }
        if (ratio) { ctx.fillStyle = t.text; ctx.textAlign = 'right'; ctx.fillText(item.ratioText, layout.ratioX, y); }
      }
      ctx.restore();
      ctx.globalAlpha = 1;
      if (silent) {
        ctx.font = labelFont; ctx.fillStyle = t.muted; ctx.textAlign = 'center';
        ctx.fillText(p.empty!, width / 2, layout.top + visible.height / 2);
      }
      ctx.font = axisFont; ctx.fillStyle = t.muted;
      ctx.textAlign = 'right'; ctx.fillText(p.leftLabel, layout.axis - VALUE_GAP, layout.top / 2);
      ctx.textAlign = 'left'; ctx.fillText(p.rightLabel, layout.axis + VALUE_GAP, layout.top / 2);
      if (ratio) { ctx.textAlign = 'right'; ctx.fillText(p.ratioLabel, layout.ratioX, layout.top / 2); }
      const hit = keyed.current ? hits.current.find(hit => hit.item.id === keyed.current)
        : pointer.current && pointer.current.y >= layout.top && pointer.current.y <= rowsBottom ? hits.current.find(hit => Math.abs(hit.y - pointer.current!.y - scrollTop) < hit.slot / 2) : undefined;
      const heldTooltip = tooltipStore.get();
      if (!keyed.current || keyIntent.current || heldTooltip == null || heldTooltip.owner === owner) {
        const part = hit && pointer.current && !keyed.current ? butterflyPartAt(pointer.current.x - layout.axis, hit.left, hit.right) : null;
        const content = hit && hit.y + hit.slot / 2 >= visible.start && hit.y - hit.slot / 2 <= visible.end
          ? p.tooltip?.(hit.item, part) ?? null : null;
        const flag = flags ? hit?.item.countryFlag : undefined;
        const raw = content && flag ? { ...content, countryFlag: flag } : content;
        const decoded = raw ? countryFlagContent(fitTooltip([raw], room, { keep: 1 }), undefined, loaded) : null;
        const fitted = decoded ? fitTooltip([decoded], room, { keep: 1 }) : null;
        if (fitted) {
          const rect = pointer.current ? null : element.getBoundingClientRect();
          showTooltip(owner, fitted, pointer.current?.clientX ?? rect!.left + rect!.width / 2,
            pointer.current?.clientY ?? rect!.top + (hit!.y - scrollTop) * rect!.height / Math.max(1, height));
          if (keyed.current) setSpoken(tooltipText(fitted));
        } else { hideTooltip(owner); setSpoken(''); }
      }
      keyIntent.current = false;
      return moving;
    } };
    task.current = frameTask;
    const remove = scheduler.add(frameTask, element);
    scheduler.setVisible(frameTask, latest.current.props.active);
    const observer = new ResizeObserver(() => { measure(); scheduler.wake(frameTask); });
    observer.observe(element);
    observer.observe(container);
    const scrolled = () => { if (geometry) geometry.scrollTop = container.scrollTop; };
    container.addEventListener('scroll', scrolled, { passive: true });
    return () => { remove(); observer.disconnect(); container.removeEventListener('scroll', scrolled); task.current = null; hideTooltip(owner); };
  }, [owner]);
  useEffect(() => {
    if (task.current) { scheduler.setVisible(task.current, props.active); scheduler.wake(task.current); }
    if (!props.active) hideTooltip(owner);
  }, [props, tokens, viewport, owner]);
  const wake = () => { if (task.current) scheduler.wake(task.current); };
  useEffect(() => props.liveItems?.subscribe(() => { if (task.current) scheduler.wake(task.current); }), [props.liveItems]);
  const leave = () => { pointer.current = null; keyed.current = null; setSpoken(''); hideTooltip(owner); wake(); };
  const open = (item: T, x: number, y: number) => { const list = props.contextItems?.(item); if (list) menu.open(x, y, list); };
  return <div ref={root} className="butterfly-chart themed-scrollbar" onScroll={wake}>
    <div className="butterfly-content" style={{ minHeight: butterflyMinimumHeight(props.items.length) }}>
      <canvas ref={canvas} tabIndex={0} role="img" aria-label={props.label}
        onPointerDown={() => { menuKey.current = false; }}
        onClick={event => {
          if (!props.onSelect) return;
          const element = canvas.current!, rect = element.getBoundingClientRect();
          const point = toLocal(event.clientX, event.clientY, rect, element.clientWidth, element.clientHeight);
          const hit = point.y >= headerHeight.current && point.y <= rowsEnd.current ? hits.current.find(hit => Math.abs(hit.y - point.y - root.current!.scrollTop) < hit.slot / 2) : undefined;
          if (hit) props.onSelect(hit.item, point.x >= axisX.current ? 'right' : 'left');
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
            : point.y >= headerHeight.current && point.y <= rowsEnd.current ? hits.current.find(hit => Math.abs(hit.y - point.y - root.current!.scrollTop) < hit.slot / 2) : undefined;
          if (hit) open(hit.item, event.clientX, event.clientY);
        }} onKeyDown={event => {
          const items = props.liveItems?.read() ?? props.items;
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
