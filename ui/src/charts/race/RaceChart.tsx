import { useEffect, useId, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent } from 'react';
import { font } from '../draw';
import { easeTau } from '../easing';
import { canvasImage } from '../images';
import { scheduler } from '../scheduler';
import type { FrameInfo, FrameTask } from '../scheduler';
import { backingSize, toLocal } from '../surface';
import { readTokens, subscribeTokens } from '../tokens';
import type { ChartTokens } from '../tokens';
import { hideTooltip, showTooltip, tooltipText } from '../tooltip';
import type { TooltipContent } from '../tooltip';
import { formatAxisValue } from '../valueAxis';
import { drawDateOverlay, drawRaceBar, drawRaceGrid, FLAG, NAME_X, VALUE_GAP } from './draw';
import { fitRows, keyedEntry, MORE_LINE, moreLineY, moreText, paintOrder, raceLayout, raceMoving, raceText, rowAt, rowCentre, stepRace } from './race';
import type { RaceItem, RaceLayout, RaceRow, RaceState } from './race';
import '../charts.css';

export interface RaceChartProps {
  /** Names the chart for screen readers. */
  label: string;
  /** The entries at the moment shown, ranked highest first (`rankItems`). Pass the
   * same array while the moment is unchanged: a new array is a new moment. */
  items: readonly RaceItem[];
  /** How many rows to make room for (the top N), so rows keep their height as
   * entries come and go. */
  slots: number;
  name: (key: string) => string;
  /** An entry's flag as an image URL; nothing is drawn without one. */
  flag?: (key: string) => string | undefined;
  /** A value-axis label; `ticks` is every tick drawn. */
  formatTick?: (value: number, ticks: readonly number[]) => string;
  /** Tooltip content for an entry, hovered or reached with the keys. */
  tooltip?: (key: string) => TooltipContent | null;
  /** Right-click on a row, or the context-menu key on the row the keys reached. */
  onContextMenu?: (key: string, clientX: number, clientY: number) => void;
  /** The moment shown as a large corner date; none when null. */
  dateOverlay?: string | null;
  /** Said in the plot when there are no entries. */
  empty?: string | null;
  hoursPerSecond?: number;
  /** Off-screen charts stop drawing. */
  active: boolean;
  /** The label and value columns only grow, so they never jump during playback;
   * a new `resetKey` (another branch) measures them afresh. */
  resetKey?: string;
}

/** Room left of the flag and after the widest name, and the column's bounds. */
const NAME_PAD = 12;
const MIN_LABEL = 72;
const MAX_LABEL_SHARE = 0.4;
const VALUE_PAD = 6;

type Pointer = { x: number; y: number; clientX: number; clientY: number };

/** A ranked bar race (style A · Clean): flag and name left of each bar, the exact
 * value at its tip, hairline gridlines. Rows re-rank at each new moment and glide to
 * their place; a bar glides only within one unbroken stretch of its values. Hover,
 * the arrow keys and right-click reach every row; the tooltip is read out. */
export default function RaceChart(props: RaceChartProps) {
  const owner = useId();
  const canvas = useRef<HTMLCanvasElement>(null);
  const latest = useRef(props);
  latest.current = props;
  const task = useRef<FrameTask | null>(null);
  // Who drives the hover: the pointer, or the keyboard's entry (read out).
  const hover = useRef<{ pointer: Pointer | null; keyed: string | null; menuKey: boolean }>({ pointer: null, keyed: null, menuKey: false });
  const shown = useRef<{ layout: RaceLayout; rows: readonly RaceRow[]; items: readonly RaceItem[]; min: number; max: number; rect: DOMRect; width: number; height: number } | null>(null);
  const [spoken, setSpoken] = useState('');
  const spokenText = useRef('');

  useEffect(() => {
    const element = canvas.current!;
    const context = element.getContext('2d')!;
    let tokens: ChartTokens = readTokens();
    let state: RaceState | null = null;
    let device: { width: number; height: number } | null = null;
    // The widest name and value seen, so the columns only grow.
    let columns = { key: '', label: MIN_LABEL, value: 0 };
    let tooltipFor: { key: string; item: RaceItem; props: RaceChartProps; content: TooltipContent | null } | null = null;
    // The entries shown: the same array while the entries and the rows that fit are,
    // so the race sees a new moment only when there is one.
    let fitted: { items: readonly RaceItem[] | null; rows: number; shown: readonly RaceItem[] } = { items: null, rows: 0, shown: [] };

    const draw = (info: FrameInfo): boolean => {
      const p = latest.current;
      const layoutWidth = element.clientWidth;
      const layoutHeight = element.clientHeight;
      if (!layoutWidth || !layoutHeight) return false;
      const rect = element.getBoundingClientRect();
      const size = backingSize({ layoutWidth, layoutHeight, rectWidth: rect.width, rectHeight: rect.height, devicePixelRatio: window.devicePixelRatio || 1,
        devicePixelWidth: device?.width, devicePixelHeight: device?.height });
      if (element.width !== size.width) element.width = size.width;
      if (element.height !== size.height) element.height = size.height;
      context.setTransform(size.scaleX, 0, 0, size.scaleY, 0, 0);

      // As many rows as fit at their least height; the rest are counted under them.
      const fit = fitRows(layoutHeight, Math.max(1, p.slots));
      if (fitted.items !== p.items || fitted.rows !== fit.rows) fitted = { items: p.items, rows: fit.rows, shown: p.items.slice(0, fit.rows) };
      const hidden = Math.max(0, p.items.length - fit.rows);
      const tau = easeTau(tokens.motion, p.hoursPerSecond ?? 24, tokens.reducedMotion);
      state = stepRace(state, fitted.shown, info.now, info.dt, tau);
      // Columns fitted to the widest name and value seen.
      if (columns.key !== (p.resetKey ?? '')) columns = { key: p.resetKey ?? '', label: MIN_LABEL, value: 0 };
      context.font = font(tokens, tokens.fontSize.body, 500);
      for (const row of state.rows) columns.label = Math.max(columns.label, NAME_X + context.measureText(p.name(row.key)).width + NAME_PAD);
      context.font = font(tokens, tokens.fontSize.body, 600);
      for (const row of state.rows) columns.value = Math.max(columns.value, context.measureText(row.item.label).width + VALUE_GAP + VALUE_PAD);
      const labelWidth = Math.min(columns.label, Math.max(MIN_LABEL, layoutWidth * MAX_LABEL_SHARE));
      const layout = raceLayout(layoutWidth, layoutHeight, fit.rows, labelWidth, Math.max(columns.value, 24), fit.hidden ? MORE_LINE : 0);
      shown.current = { layout, rows: state.rows, items: fitted.shown, min: state.min, max: state.max, rect, width: layoutWidth, height: layoutHeight };

      context.clearRect(0, 0, layoutWidth, layoutHeight);
      if (p.dateOverlay) drawDateOverlay(context, layout, tokens, p.dateOverlay);
      drawRaceGrid(context, layout, tokens, state.min, state.max, p.formatTick ?? formatAxisValue);
      if (!p.items.length && p.empty) {
        context.save();
        context.font = font(tokens, tokens.fontSize.body);
        context.fillStyle = tokens.muted;
        context.textAlign = 'center';
        context.textBaseline = 'middle';
        context.fillText(p.empty, (layout.left + layout.right) / 2, (layout.top + layout.bottom) / 2, layout.right - layout.left);
        context.restore();
      }
      if (hidden) {
        context.save();
        context.font = font(tokens, tokens.fontSize.small);
        context.fillStyle = tokens.muted;
        context.textAlign = 'left';
        context.textBaseline = 'middle';
        context.fillText(moreText(hidden), NAME_X, moreLineY(layout), Math.max(1, layoutWidth - NAME_X));
        context.restore();
      }

      // The row the hover is about: under the pointer, else the one the keys reached.
      const { pointer, keyed } = hover.current;
      const hovered = pointer ? rowAt(state.rows, layout, pointer, state.min, state.max) : keyed ? state.rows.find(row => row.key === keyed && row.phase !== 'leaving') ?? null : null;

      context.save();
      context.beginPath();
      context.rect(0, layout.top - 4, layoutWidth, layout.bottom - layout.top + 8);
      context.clip();
      // Where each bar's value label ends, for the keyboard's tooltip.
      const ends = new Map<string, number>();
      // Lower rows first, so a row moving up passes over the others; hovering reads
      // the same order backwards (`rowAt`).
      for (const row of paintOrder(state.rows)) {
        const centre = rowCentre(layout, row.y);
        context.globalAlpha = Math.max(0, Math.min(1, row.alpha));
        if (row === hovered) {
          context.fillStyle = tokens.text;
          context.globalAlpha *= 0.05;
          context.fillRect(0, centre - layout.slot / 2, layoutWidth, layout.slot);
          context.globalAlpha = Math.max(0, Math.min(1, row.alpha));
        }
        const url = p.flag?.(row.key);
        const image = url ? canvasImage(url, wake) : null;
        if (image) context.drawImage(image, FLAG.x, centre - FLAG.height / 2, FLAG.width, FLAG.height);
        context.fillStyle = tokens.text;
        context.font = font(tokens, tokens.fontSize.body, 500);
        context.textAlign = 'left';
        context.textBaseline = 'middle';
        context.fillText(p.name(row.key), NAME_X, centre, Math.max(1, layout.left - NAME_X - NAME_PAD));
        const tip = drawRaceBar(context, layout, tokens, row, state.min, state.max);
        context.fillStyle = tokens.text;
        context.font = font(tokens, tokens.fontSize.body, 600);
        context.fillText(row.item.label, tip + VALUE_GAP, centre);
        ends.set(row.key, tip + VALUE_GAP + context.measureText(row.item.label).width);
      }
      context.restore();
      context.globalAlpha = 1;

      if (hovered && p.tooltip) {
        if (!tooltipFor || tooltipFor.key !== hovered.key || tooltipFor.item !== hovered.item || tooltipFor.props !== p) {
          tooltipFor = { key: hovered.key, item: hovered.item, props: p, content: p.tooltip(hovered.key) };
        }
        const content = tooltipFor.content;
        // The keyboard's tooltip sits at the row's tip; layout pixels become screen
        // pixels through the widget's Size zoom.
        const at = pointer ?? { clientX: rect.left + Math.min(layoutWidth, ends.get(hovered.key) ?? layout.left) * rect.width / layoutWidth,
          clientY: rect.top + rowCentre(layout, hovered.y) * rect.height / layoutHeight };
        if (content) showTooltip(owner, content, at.clientX, at.clientY, `${hovered.key}:${hovered.item.label}:${hovered.item.value}`);
        else hideTooltip(owner);
        const text = !pointer && content ? tooltipText(content) : spokenText.current;
        if (text !== spokenText.current) { spokenText.current = text; setSpoken(text); }
      } else { tooltipFor = null; hideTooltip(owner); }
      return raceMoving(state);
    };
    const current: FrameTask = { frame: draw };
    task.current = current;
    const wake = () => scheduler.wake(current);
    const remove = scheduler.add(current);
    const unsubscribe = subscribeTokens(next => { tokens = next; wake(); });
    let observer: ResizeObserver | null = null;
    const resize = (entries: ResizeObserverEntry[]) => {
      const box = entries[0]?.devicePixelContentBoxSize?.[0];
      device = box ? { width: box.inlineSize, height: box.blockSize } : null;
      wake();
    };
    try { observer = new ResizeObserver(resize); observer.observe(element, { box: 'device-pixel-content-box' }); }
    catch { observer = new ResizeObserver(resize); observer.observe(element); }
    const local = (event: PointerEvent | MouseEvent): Pointer => ({
      ...toLocal(event.clientX, event.clientY, element.getBoundingClientRect(), element.clientWidth, element.clientHeight), clientX: event.clientX, clientY: event.clientY });
    // The pointer takes the hover back from the keyboard.
    const move = (event: PointerEvent) => { hover.current = { ...hover.current, pointer: local(event), keyed: null }; wake(); };
    const leave = () => { hover.current = { ...hover.current, pointer: null }; hideTooltip(owner); wake(); };
    const down = () => { hover.current = { ...hover.current, menuKey: false }; };
    const menu = (event: MouseEvent) => {
      // The menu key already opened a menu for the row the keys reached; the
      // browser's own event after it must not open a second.
      if (hover.current.menuKey) { hover.current = { ...hover.current, menuKey: false }; event.preventDefault(); return; }
      const handler = latest.current.onContextMenu;
      const at = shown.current;
      if (!handler || !at) return;
      const row = rowAt(at.rows, at.layout, local(event), at.min, at.max);
      if (!row) return;
      event.preventDefault();
      handler(row.key, event.clientX, event.clientY);
    };
    element.addEventListener('pointermove', move);
    element.addEventListener('pointerleave', leave);
    element.addEventListener('pointerdown', down);
    element.addEventListener('contextmenu', menu);
    return () => {
      remove(); unsubscribe(); observer?.disconnect(); hideTooltip(owner);
      element.removeEventListener('pointermove', move);
      element.removeEventListener('pointerleave', leave);
      element.removeEventListener('pointerdown', down);
      element.removeEventListener('contextmenu', menu);
      task.current = null;
    };
  }, [owner]);

  useEffect(() => {
    if (task.current) scheduler.setVisible(task.current, props.active);
    // An inactive chart no longer draws, so it must not leave its tooltip on screen.
    if (!props.active) hideTooltip(owner);
  }, [props.active, owner]);
  useEffect(() => { if (task.current) scheduler.wake(task.current); });

  const wake = () => { if (task.current) scheduler.wake(task.current); };
  const clearKeyed = () => {
    hover.current = { ...hover.current, keyed: null };
    spokenText.current = '';
    setSpoken('');
    wake();
  };
  const keyDown = (event: ReactKeyboardEvent) => {
    if (event.key === 'ArrowUp' || event.key === 'ArrowDown' || event.key === 'Home' || event.key === 'End') {
      event.preventDefault();
      // The keyboard takes the hover from the pointer, stepping on from the row shown.
      const from = hover.current.keyed ?? (hover.current.pointer && shown.current ? rowAt(shown.current.rows, shown.current.layout, hover.current.pointer, shown.current.min, shown.current.max)?.key ?? null : null);
      hover.current = { ...hover.current, pointer: null, keyed: keyedEntry(event.key, from, shown.current?.items ?? props.items) };
      wake();
      return;
    }
    if (event.key === 'Escape' && hover.current.keyed != null) { event.preventDefault(); clearKeyed(); return; }
    const keyed = hover.current.keyed;
    if ((event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) && props.onContextMenu && keyed && shown.current) {
      event.preventDefault();
      hover.current = { ...hover.current, menuKey: true };
      const { rect, layout, rows, height } = shown.current;
      const row = rows.find(item => item.key === keyed);
      const y = row ? rect.top + rowCentre(layout, row.y) * rect.height / height : rect.top + rect.height / 2;
      props.onContextMenu(keyed, rect.left + rect.width / 2, y);
    }
  };
  const ranking = raceText(props.items, props.name);
  return <div className="race-chart">
    <canvas ref={canvas} tabIndex={0} role="img" aria-label={props.label} aria-describedby={`${owner}-ranking`}
      aria-keyshortcuts="ArrowUp ArrowDown Home End" onKeyDown={keyDown} onBlur={clearKeyed} />
    <p id={`${owner}-ranking`} className="sr-only">{ranking || props.empty || ''}</p>
    <p className="sr-only" aria-live="polite">{spoken}</p>
  </div>;
}
