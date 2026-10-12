import { useEffect, useId, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent } from 'react';
import { font, LABEL_WEIGHT, VALUE_WEIGHT } from '../draw';
import { easeTau } from '../easing';
import { canvasImage } from '../images';
import { countryFlagContent, drawCountryFlag } from '../countryFlag';
import type { CountryFlagLook } from '../../components/CountryFlag';
import { revealRow } from '../../components/countryList';
import { scheduler } from '../scheduler';
import type { FrameInfo, FrameTask } from '../scheduler';
import { toLocal, watchCanvasSize } from '../surface';
import { readTokens, subscribeTokens } from '../tokens';
import type { ChartTokens } from '../tokens';
import { fitTooltip, hideTooltip, showTooltip, tooltipStore, tooltipText } from '../tooltip';
import type { TooltipContent } from '../tooltip';
import { formatAxisValue } from '../valueAxis';
import { drawDateOverlay, drawRaceBar, drawRaceGrid, FLAG, NAME_X, VALUE_GAP } from './draw';
import { keyedEntry, paintOrder, raceHeight, raceLayout, raceMoving, raceText, rowAt, rowCentre, rowInView, rowSeen, selectRaceRow, stepRace } from './race';
import type { RaceItem, RaceLayout, RaceRow, RaceState } from './race';
import '../charts.css';

export interface RaceChartProps {
  /** Names the chart for screen readers. */
  label: string;
  /** The entries at the moment shown, ranked highest first (`rankItems`). Pass the
   * same array while the moment is unchanged: a new array is a new moment. */
  items: readonly RaceItem[];
  /** How many rows to make room for (the top N), so rows keep their height as
   * entries come and go. Rows that need more room than the chart has scroll. */
  slots: number;
  name: (key: string, item?: RaceItem) => string;
  /** An entry's flag as an image URL; nothing is drawn without one. */
  flag?: (key: string, item?: RaceItem) => string | undefined;
  /** Present only for genuine countries, even when their flag is unavailable. */
  countryFlag?: (key: string, item?: RaceItem) => CountryFlagLook | undefined;
  /** A value-axis label; `ticks` is every tick drawn. */
  formatTick?: (value: number, ticks: readonly number[]) => string;
  /** Tooltip content for an entry, hovered or reached with the keys. */
  tooltip?: (key: string, item?: RaceItem) => TooltipContent | null;
  /** Click or Enter opens the widget's breakdown for this row. */
  onPick?: (key: string, item?: RaceItem) => void;
  pickOwner?: unknown;
  /** Right-click on a row, or the context-menu key on the row the keys reached. */
  onContextMenu?: (key: string, clientX: number, clientY: number, item?: RaceItem) => void;
  /** The moment shown as a large corner date; none when null. */
  dateOverlay?: string | null;
  /** Said in the plot when there are no entries. */
  empty?: string | null;
  hoursPerSecond?: number;
  /** Off-screen charts stop drawing. */
  active: boolean;
  /** The columns only grow during playback; another context resets motion and measurements. */
  resetKey?: string;
}

/** Rows are sized for the top N, so they keep their height while entries come and go;
 * the scroll reaches only the entries there are. */
const raceSlots = (props: Pick<RaceChartProps, 'items' | 'slots'>) => Math.max(1, props.slots, props.items.length);

/** Room left of the flag and after the widest name, and the column's bounds. */
const NAME_PAD = 12;
const MIN_LABEL = 72;
const MAX_LABEL_SHARE = 0.4;
const VALUE_PAD = 6;

type Pointer = { x: number; y: number; clientX: number; clientY: number };

/** A ranked bar race (style A · Clean): flag and name left of each bar, the exact
 * value at its tip, hairline gridlines. Rows re-rank at each new moment and glide to
 * their place; a bar glides only within one unbroken stretch of its values. Rows that
 * do not fit scroll under the fixed axis, and only those in view are drawn. Hover,
 * the arrow keys and right-click reach every row; the tooltip is read out. */
export default function RaceChart(props: RaceChartProps) {
  const owner = useId();
  const canvas = useRef<HTMLCanvasElement>(null);
  const scroller = useRef<HTMLDivElement>(null);
  // Kept from scroll events, so a frame never reads the layout to find it.
  const scrollTop = useRef(0);
  const latest = useRef(props);
  latest.current = props;
  const task = useRef<FrameTask | null>(null);
  // Who drives the hover: the pointer, or the keyboard's entry (read out).
  const hover = useRef<{ pointer: Pointer | null; keyed: string | null; menuKey: boolean }>({ pointer: null, keyed: null, menuKey: false });
  const shown = useRef<{ layout: RaceLayout; rows: readonly RaceRow[]; items: readonly RaceItem[]; min: number; max: number; height: number;
    owner: unknown; resetKey?: string } | null>(null);
  const keyIntent = useRef(false);
  const [spoken, setSpoken] = useState('');
  const spokenText = useRef('');

  useEffect(() => {
    const element = canvas.current!;
    const context = element.getContext('2d')!;
    let tokens: ChartTokens = readTokens();
    let state: RaceState | null = null;
    let stateKey = props.resetKey;
    const surface = watchCanvasSize(element, () => wake());
    // The widest name and value seen, so the columns only grow.
    let columns = { key: '', label: MIN_LABEL, value: 0 };
    // The moment the columns were measured at, and each name's width in the current
    // font: a long list is measured once per moment, not in every frame.
    let measured: { items: readonly RaceItem[]; name: RaceChartProps['name']; tokens: ChartTokens } | null = null;
    const nameWidths = new Map<string, number>();
    let tooltipFor: { key: string; item: RaceItem; props: RaceChartProps; content: TooltipContent | null } | null = null;

    const draw = (info: FrameInfo): boolean => {
      const p = latest.current;
      if (stateKey !== p.resetKey) { state = null; stateKey = p.resetKey; }
      const { layoutWidth, layoutHeight, backing: size } = surface.read();
      if (!layoutWidth || !layoutHeight) return false;
      if (element.width !== size.width) element.width = size.width;
      if (element.height !== size.height) element.height = size.height;
      context.setTransform(size.scaleX, 0, 0, size.scaleY, 0, 0);

      const tau = easeTau(tokens.motion, p.hoursPerSecond ?? 24, tokens.reducedMotion);
      state = stepRace(state, p.items, info.now, info.dt, tau);
      // Columns fitted to the widest name and value seen.
      if (columns.key !== (p.resetKey ?? '')) { columns = { key: p.resetKey ?? '', label: MIN_LABEL, value: 0 }; measured = null; }
      if (!measured || measured.items !== p.items || measured.name !== p.name || measured.tokens !== tokens) {
        if (measured && measured.tokens !== tokens) nameWidths.clear();
        context.font = font(tokens, tokens.fontSize.body, LABEL_WEIGHT);
        for (const item of p.items) {
          const name = p.name(item.key, item);
          let width = nameWidths.get(name);
          if (width === undefined) nameWidths.set(name, width = context.measureText(name).width);
          columns.label = Math.max(columns.label, NAME_X + width + NAME_PAD);
        }
        context.font = font(tokens, tokens.fontSize.body, VALUE_WEIGHT);
        for (const item of p.items) columns.value = Math.max(columns.value, context.measureText(item.label).width + VALUE_GAP + VALUE_PAD);
        measured = { items: p.items, name: p.name, tokens };
      }
      const labelWidth = Math.min(columns.label, Math.max(MIN_LABEL, layoutWidth * MAX_LABEL_SHARE));
      const layout = raceLayout(layoutWidth, layoutHeight, raceSlots(p), labelWidth, Math.max(columns.value, 24), scrollTop.current, p.items.length);
      shown.current = { layout, rows: state.rows, items: p.items, min: state.min, max: state.max, height: layoutHeight,
        owner: p.pickOwner, resetKey: p.resetKey };

      context.clearRect(0, 0, layoutWidth, layoutHeight);
      if (p.dateOverlay) drawDateOverlay(context, layout, tokens, p.dateOverlay);
      drawRaceGrid(context, layout, tokens, state.min, state.max, p.formatTick ?? formatAxisValue);
      if (!p.items.length && p.empty) {
        context.save();
        context.font = font(tokens, tokens.fontSize.body);
        context.fillStyle = tokens.muted;
        context.textAlign = 'center';
        context.textBaseline = 'middle';
        context.fillText(p.empty, (layout.left + layout.right) / 2, (layout.view.top + layout.view.bottom) / 2, layout.right - layout.left);
        context.restore();
      }

      // The row the hover is about: under the pointer, else the one the keys reached.
      // The keys let go of a row scrolled out of view, so its tooltip goes and the keys
      // act only on rows that are seen.
      const reached = hover.current.pointer ? null : state.rows.find(row => row.key === hover.current.keyed && row.phase !== 'leaving');
      if (reached && !rowSeen(layout, reached)) { hover.current = { ...hover.current, keyed: null }; spokenText.current = ''; setSpoken(''); }
      const { pointer, keyed } = hover.current;
      const hovered = pointer ? rowAt(state.rows, layout, pointer, state.min, state.max) : keyed ? state.rows.find(row => row.key === keyed && row.phase !== 'leaving') ?? null : null;

      context.save();
      context.beginPath();
      context.rect(0, layout.view.top - 4, layoutWidth, layout.view.bottom - layout.view.top + 8);
      context.clip();
      // Where each bar's value label ends, for the keyboard's tooltip.
      const ends = new Map<string, number>();
      // Lower rows first, so a row moving up passes over the others; hovering reads
      // the same order backwards (`rowAt`).
      for (const row of paintOrder(state.rows)) {
        const centre = rowCentre(layout, row.y);
        if (!rowInView(layout, centre)) continue;
        context.globalAlpha = Math.max(0, Math.min(1, row.alpha));
        if (row === hovered) {
          context.fillStyle = tokens.text;
          context.globalAlpha *= 0.05;
          context.fillRect(0, centre - layout.slot / 2, layoutWidth, layout.slot);
          context.globalAlpha = Math.max(0, Math.min(1, row.alpha));
        }
        const countryFlag = p.countryFlag?.(row.key, row.item);
        if (countryFlag) drawCountryFlag(context, countryFlag, FLAG.x, centre - FLAG.height / 2, FLAG.width, FLAG.height, tokens, wake);
        else {
          const url = p.flag?.(row.key, row.item);
          const image = url ? canvasImage(url, wake) : null;
          if (image) context.drawImage(image, FLAG.x, centre - FLAG.height / 2, FLAG.width, FLAG.height);
        }
        context.fillStyle = tokens.text;
        context.font = font(tokens, tokens.fontSize.body, LABEL_WEIGHT);
        context.textAlign = 'left';
        context.textBaseline = 'middle';
        context.fillText(p.name(row.key, row.item), NAME_X, centre, Math.max(1, layout.left - NAME_X - NAME_PAD));
        const tip = drawRaceBar(context, layout, tokens, row, state.min, state.max);
        context.fillStyle = tokens.text;
        context.font = font(tokens, tokens.fontSize.body, VALUE_WEIGHT);
        context.fillText(row.item.label, tip + VALUE_GAP, centre);
        ends.set(row.key, tip + VALUE_GAP + context.measureText(row.item.label).width);
      }
      context.restore();
      context.globalAlpha = 1;

      const heldTooltip = tooltipStore.get();
      if (hovered && p.tooltip && (pointer || keyIntent.current || heldTooltip == null || heldTooltip.owner === owner)) {
        if (!tooltipFor || tooltipFor.key !== hovered.key || tooltipFor.item !== hovered.item || tooltipFor.props !== p) {
          tooltipFor = { key: hovered.key, item: hovered.item, props: p, content: p.tooltip(hovered.key, hovered.item) };
        }
        const flag = p.countryFlag?.(hovered.key, hovered.item);
        const raw = tooltipFor.content && flag ? { ...tooltipFor.content, countryFlag: flag } : tooltipFor.content;
        const decoded = raw ? countryFlagContent(fitTooltip([raw], globalThis.innerHeight), undefined, wake) : null;
        const content = decoded ? fitTooltip([decoded], globalThis.innerHeight) : null;
        // The keyboard's tooltip sits at the row's tip; layout pixels become screen
        // pixels through the widget's Size zoom.
        let at: { clientX: number; clientY: number } | null = pointer;
        if (!at) {
          const rect = element.getBoundingClientRect();
          at = { clientX: rect.left + Math.min(layoutWidth, ends.get(hovered.key) ?? layout.left) * rect.width / layoutWidth,
            clientY: rect.top + rowCentre(layout, hovered.y) * rect.height / layoutHeight };
        }
        if (content) showTooltip(owner, content, at.clientX, at.clientY, `${hovered.key}:${hovered.item.label}:${hovered.item.value}`);
        else hideTooltip(owner);
        const text = !pointer && content ? tooltipText(content) : spokenText.current;
        if (text !== spokenText.current) { spokenText.current = text; setSpoken(text); }
      } else { tooltipFor = null; hideTooltip(owner); }
      keyIntent.current = false;
      return raceMoving(state);
    };
    const current: FrameTask = { frame: draw };
    task.current = current;
    const wake = () => scheduler.wake(current);
    const remove = scheduler.add(current);
    const unsubscribe = subscribeTokens(next => { tokens = next; wake(); });
    const local = (event: PointerEvent | MouseEvent): Pointer => ({
      ...toLocal(event.clientX, event.clientY, element.getBoundingClientRect(), element.clientWidth, element.clientHeight), clientX: event.clientX, clientY: event.clientY });
    // The pointer takes the hover back from the keyboard.
    const move = (event: PointerEvent) => { hover.current = { ...hover.current, pointer: local(event), keyed: null }; wake(); };
    const leave = () => { hover.current = { ...hover.current, pointer: null }; hideTooltip(owner); wake(); };
    const down = () => { hover.current = { ...hover.current, menuKey: false }; };
    const select = (event: MouseEvent) => {
      const at = shown.current;
      if (!at || !latest.current.active || !latest.current.onPick || at.owner !== latest.current.pickOwner || at.resetKey !== latest.current.resetKey) return;
      const row = rowAt(at.rows, at.layout, local(event), at.min, at.max);
      selectRaceRow(row?.key ?? null, 'click', key => latest.current.onPick?.(key, row?.item));
    };
    const menu = (event: MouseEvent) => {
      // The menu key already opened a menu for the row the keys reached; the
      // browser's own event after it must not open a second.
      if (hover.current.menuKey) { hover.current = { ...hover.current, menuKey: false }; event.preventDefault(); return; }
      const handler = latest.current.onContextMenu;
      const at = shown.current;
      if (!handler || !at || !latest.current.active || at.owner !== latest.current.pickOwner || at.resetKey !== latest.current.resetKey) return;
      const row = rowAt(at.rows, at.layout, local(event), at.min, at.max);
      if (!row) return;
      event.preventDefault();
      handler(row.key, event.clientX, event.clientY, row.item);
    };
    element.addEventListener('pointermove', move);
    element.addEventListener('pointerleave', leave);
    element.addEventListener('pointerdown', down);
    element.addEventListener('contextmenu', menu);
    element.addEventListener('click', select);
    return () => {
      remove(); unsubscribe(); surface.disconnect(); hideTooltip(owner);
      element.removeEventListener('pointermove', move);
      element.removeEventListener('pointerleave', leave);
      element.removeEventListener('pointerdown', down);
      element.removeEventListener('contextmenu', menu);
      element.removeEventListener('click', select);
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
      const items = shown.current?.items ?? props.items;
      const keyed = keyedEntry(event.key, from, items);
      hover.current = { ...hover.current, pointer: null, keyed };
      // Scroll the row reached into view, at its place in the ranking.
      const box = scroller.current, layout = shown.current?.layout;
      const index = items.findIndex(item => item.key === keyed);
      if (box && layout && index >= 0) {
        const to = Math.max(0, revealRow(index, scrollTop.current, layout.view.bottom - layout.view.top, layout.slot));
        if (to !== scrollTop.current) box.scrollTop = scrollTop.current = to;
      }
      keyIntent.current = true;
      wake();
      return;
    }
    if (event.key === 'Escape' && hover.current.keyed != null) { event.preventDefault(); clearKeyed(); return; }
    const keyed = hover.current.keyed;
    const drawn = shown.current;
    const row = drawn?.rows.find(item => item.key === keyed && item.phase !== 'leaving');
    const current = latest.current;
    if (!current.active || drawn?.owner !== current.pickOwner || drawn?.resetKey !== current.resetKey || !row || !drawn || !rowSeen(drawn.layout, row)) return;
    if (selectRaceRow(keyed, event.key, current.onPick ? key => current.onPick?.(key, row.item) : undefined)) {
      event.preventDefault();
      return;
    }
    if ((event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) && current.onContextMenu && keyed && shown.current) {
      event.preventDefault();
      hover.current = { ...hover.current, menuKey: true };
      const { layout, height } = shown.current;
      const rect = canvas.current!.getBoundingClientRect();
      const y = rect.top + rowCentre(layout, row.y) * rect.height / height;
      current.onContextMenu(keyed, rect.left + rect.width / 2, y, row.item);
    }
  };
  const ranking = raceText(props.items, props.name);
  return <div className="race-chart">
    <div ref={scroller} className="race-scroll themed-scrollbar" onScroll={event => { scrollTop.current = event.currentTarget.scrollTop; wake(); }}>
      <canvas ref={canvas} tabIndex={0} role="img" aria-label={props.label} aria-describedby={`${owner}-ranking`}
        aria-keyshortcuts={`ArrowUp ArrowDown Home End${props.onPick ? ' Enter Space' : ''}`} onKeyDown={keyDown} onBlur={clearKeyed} />
      {/* The entries' height past the chart's own: the canvas stays in view while this scrolls. */}
      <div className="race-extent" style={{ height: `calc(${raceHeight(props.items.length)}px - 100%)` }} />
    </div>
    <p id={`${owner}-ranking`} className="sr-only">{ranking || props.empty || ''}</p>
    <p className="sr-only" aria-live="polite">{spoken}</p>
  </div>;
}
