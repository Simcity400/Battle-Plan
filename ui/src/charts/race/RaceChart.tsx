import { useEffect, useId, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent } from 'react';
import { font, LABEL_WEIGHT, VALUE_WEIGHT } from '../draw';
import { easeTau } from '../easing';
import { useMotionNow } from '../liveMotion.ts';
import { canvasImage } from '../images';
import { countryFlagContent, drawCountryFlag } from '../countryFlag';
import type { CountryFlagLook } from '../../components/CountryFlag';
import { revealRow } from '../../components/countryList';
import { scheduler } from '../scheduler';
import type { FrameInfo, FrameTask } from '../scheduler';
import { drawingSurface, sizeCanvas, toLocal, watchCanvasSize } from '../surface';
import { readTokens, subscribeTokens } from '../tokens';
import type { ChartTokens } from '../tokens';
import { fitTooltip, hideTooltip, showTooltip, tooltipStore, tooltipText } from '../tooltip';
import type { TooltipContent } from '../tooltip';
import { formatAxisValue } from '../valueAxis';
import { ellipsise } from '../labels';
import { drawDateOverlay, drawRaceBar, drawRaceEmpty, drawRaceGrid, FLAG, MIN_LABEL, NAME_PAD, NAME_X, VALUE_GAP, VALUE_PAD } from './draw';
import { keyedEntry, paintOrder, raceHeight, raceLayout, raceMoving, raceText, rowAt, rowCentre, rowInView, rowSeen, selectRaceRow, stepRace } from './race';
import type { RaceItem, RaceLayout, RaceRow, RaceState } from './race';
import type { PlaybackController } from '../../dashboard/playback';
import '../charts.css';

export interface RemoteRaceRows {
  total: number; offset: number; min: number; max: number;
  onRange: (first: number, count: number) => void;
  owner: unknown;
}

export interface LiveRace {
  items: () => readonly RaceItem[];
  label: () => string;
  overlay: () => string | null;
  bind: (wake: () => void) => () => void;
  clock: PlaybackController;
}

const SPOKEN_MS = 1000;

export interface RaceChartProps {
  label: string;
  items: readonly RaceItem[];
  live?: LiveRace;
  remoteRows?: RemoteRaceRows;
  slots: number;
  name: (key: string, item?: RaceItem) => string;
  nameSamples?: readonly string[];
  flag?: (key: string, item?: RaceItem) => string | undefined;
  countryFlag?: (key: string, item?: RaceItem) => CountryFlagLook | undefined;
  formatTick?: (value: number, ticks: readonly number[]) => string;
  tooltip?: (key: string, item?: RaceItem) => TooltipContent | null;
  onPick?: (key: string, item?: RaceItem) => void;
  pickOwner?: unknown;
  onContextMenu?: (key: string, clientX: number, clientY: number, item?: RaceItem) => void;
  dateOverlay?: string | null;
  empty?: string | null;
  hoursPerSecond?: number;
  active: boolean;
  resetKey?: string;
}


const MAX_LABEL_SHARE = 0.4;

type Pointer = { x: number; y: number; clientX: number; clientY: number };

export default function RaceChart(props: RaceChartProps) {
  const owner = useId();
  const canvas = useRef<HTMLCanvasElement>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const scrollTop = useRef(0);
  const latest = useRef(props);
  latest.current = props;
  const motionNow = useMotionNow();
  const task = useRef<FrameTask | null>(null);
  const forced = useRef(true);
  const hover = useRef<{ pointer: Pointer | null; keyed: string | null; menuKey: boolean }>({ pointer: null, keyed: null, menuKey: false });
  const shown = useRef<{ layout: RaceLayout; rows: readonly RaceRow[]; items: readonly RaceItem[]; min: number; max: number; height: number;
    owner: unknown; resetKey?: string } | null>(null);
  const keyIntent = useRef(false);
  const pendingKey = useRef<{ index: number; owner: unknown; resetKey?: string } | null>(null);
  const demand = useRef<{ owner: unknown; first: number; count: number; callback: RemoteRaceRows['onRange'] } | null>(null);
  const requestRange = (remote: RemoteRaceRows, first: number, count: number) => {
    const before = demand.current;
    if (before && before.owner === remote.owner && before.first === first && before.count === count && before.callback === remote.onRange) return;
    demand.current = { owner: remote.owner, first, count, callback: remote.onRange };
    remote.onRange(first, count);
  };
  const [spoken, setSpoken] = useState('');
  const spokenText = useRef('');
  const ranked = useRef<HTMLParagraphElement>(null);
  const extent = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const element = canvas.current!;
    const paper = drawingSurface(element), context = paper.context!;
    let tokens: ChartTokens = readTokens();
    let state: RaceState | null = null;
    let stateKey = props.resetKey;
    let remoteOffset = props.remoteRows?.offset, remoteOwner = props.remoteRows?.owner;
    const surface = watchCanvasSize(element, () => wake());
    let columns = { key: '', label: MIN_LABEL, value: 0 };
    let measured: { items: readonly RaceItem[]; name: RaceChartProps['name']; nameSamples: RaceChartProps['nameSamples']; tokens: ChartTokens } | null = null;
    const nameWidths = new Map<string, number>();
    const namedKeys = new Set<string>();
    const labelWidths = new Map<string, number>();
    let labelTokens: ChartTokens | null = null;
    let tabular: boolean | null = null;
    let painted: unknown[] | null = null;
    const fitted: { width: number; tokens: ChartTokens | null; names: Map<string, string> } = { width: -1, tokens: null, names: new Map() };
    let spokenItems: readonly RaceItem[] | null = null, spokenAt = -Infinity, extentRows = -1;
    let tooltipFor: { key: string; item: RaceItem; props: RaceChartProps; content: TooltipContent | null } | null = null;

    const draw = (info: FrameInfo): boolean => {
      const p = latest.current;
      if (stateKey !== p.resetKey) { state = null; stateKey = p.resetKey; }
      if (remoteOffset !== p.remoteRows?.offset || remoteOwner !== p.remoteRows?.owner) {
        if (remoteOwner !== p.remoteRows?.owner) { pendingKey.current = null; hover.current = { ...hover.current, keyed: null, menuKey: false }; }
        if (remoteOffset !== p.remoteRows?.offset) state = null;
        remoteOffset = p.remoteRows?.offset; remoteOwner = p.remoteRows?.owner;
      }
      const { layoutWidth, layoutHeight, backing: size } = surface.read();
      if (!layoutWidth || !layoutHeight) return false;
      sizeCanvas(paper.canvas, size.width, size.height);
      context.setTransform(size.scaleX, 0, 0, size.scaleY, 0, 0);

      const tau = easeTau(tokens.motion, motionNow(p.hoursPerSecond), tokens.reducedMotion);
      const items = p.live ? p.live.items() : p.items;
      if (p.live && items.length !== extentRows && extent.current) { extentRows = items.length; extent.current.style.height = `calc(${raceHeight(items.length)}px - 100%)`; }
      if (p.live && items !== spokenItems && (info.now - spokenAt >= SPOKEN_MS || !p.live.clock.getSnapshot().playing)) {
        spokenItems = items; spokenAt = info.now;
        element.setAttribute('aria-label', p.live.label());
        if (ranked.current) ranked.current.textContent = raceText(items, p.name) || p.empty || '';
      }
      if (columns.key !== (p.resetKey ?? '')) { columns = { key: p.resetKey ?? '', label: MIN_LABEL, value: 0 }; measured = null; namedKeys.clear(); }
      if (!measured || measured.items !== items || measured.name !== p.name || measured.nameSamples !== p.nameSamples || measured.tokens !== tokens) {
        if (p.remoteRows || measured && measured.tokens !== tokens) { nameWidths.clear(); namedKeys.clear(); }
        if (measured && (measured.name !== p.name || measured.nameSamples !== p.nameSamples)) namedKeys.clear();
        let fontSet = false;
        if (items.length) for (const name of p.nameSamples ?? []) {
          if (!fontSet) { context.font = font(tokens, tokens.fontSize.body, LABEL_WEIGHT); fontSet = true; }
          let width = nameWidths.get(name);
          if (width === undefined) nameWidths.set(name, width = context.measureText(name).width);
          columns.label = Math.max(columns.label, NAME_X + width + NAME_PAD);
        }
        for (const item of items) {
          if (p.live) { if (namedKeys.has(item.key)) continue; namedKeys.add(item.key); }
          if (!fontSet) { context.font = font(tokens, tokens.fontSize.body, LABEL_WEIGHT); fontSet = true; }
          const name = p.name(item.key, item);
          let width = nameWidths.get(name);
          if (width === undefined) nameWidths.set(name, width = context.measureText(name).width);
          columns.label = Math.max(columns.label, NAME_X + width + NAME_PAD);
        }
        if (!p.live) context.font = font(tokens, tokens.fontSize.body, VALUE_WEIGHT);
        if (!p.live) for (const item of items) columns.value = Math.max(columns.value, context.measureText(item.label).width + VALUE_GAP + VALUE_PAD);
        measured = { items, name: p.name, nameSamples: p.nameSamples, tokens };
      }
      const labelWidth = Math.min(columns.label, Math.max(MIN_LABEL, layoutWidth * MAX_LABEL_SHARE));
      const remote = p.remoteRows;
      const layout = raceLayout(layoutWidth, layoutHeight, Math.max(1, p.slots, remote?.total ?? items.length), labelWidth, Math.max(columns.value, 24), scrollTop.current, remote?.total ?? items.length);
      state = stepRace(state, items, info.now, info.dt, tau, undefined,
        remote ? undefined : { from: (layout.view.top - layout.top) / layout.slot, to: (layout.view.bottom - layout.top) / layout.slot });
      const rows = remote ? state.rows.map(row => ({ ...row, y: row.y + remote.offset, target: row.target + remote.offset })) : state.rows;
      const min = remote ? Math.min(0, remote.min) * 1.06 : state.min;
      const max = remote ? Math.max(1, remote.max) * 1.06 : state.max;
      if (remote && p.active) {
        const first = Math.max(0, Math.floor((layout.view.top - layout.top) / layout.slot));
        const end = Math.min(remote.total, Math.ceil((layout.view.bottom - layout.top) / layout.slot) + 1);
        requestRange(remote, first, Math.max(0, end - first));
        const pending = pendingKey.current;
        if (pending && (pending.owner !== remote.owner || pending.resetKey !== p.resetKey)) pendingKey.current = null;
        else if (pending) {
          const item = items[pending.index - remote.offset];
          if (item) { hover.current = { pointer: null, keyed: item.key, menuKey: false }; pendingKey.current = null; keyIntent.current = true; }
        }
      }
      shown.current = { layout, rows, items, min, max, height: layoutHeight,
        owner: remote?.owner ?? p.pickOwner, resetKey: p.resetKey };

      const overlay = p.live ? p.live.overlay() : p.dateOverlay;
      const inView = rows.filter(row => rowInView(layout, rowCentre(layout, row.y)));
      const pointed = hover.current.pointer ?? hover.current.keyed;
      const picture: unknown[] = [layoutWidth, layoutHeight, size.width, size.height, layout.top, layout.left, layout.right, min, max, tokens, overlay,
        items.length === 0, pointed, p.name, p.countryFlag, p.flag];
      for (const row of inView) picture.push(row.key, row.y, row.alpha, row.widths, row.inset, row.item.label, row.item.segments);
      if (p.live && !forced.current && painted && painted.length === picture.length && painted.every((value, at) => Object.is(value, picture[at]))) {
        shown.current = { layout, rows, items, min, max, height: layoutHeight, owner: remote?.owner ?? p.pickOwner, resetKey: p.resetKey };
        return raceMoving(state);
      }
      painted = picture; forced.current = false;
      if (labelTokens !== tokens) { labelWidths.clear(); labelTokens = tokens; tabular = null; }
      context.clearRect(0, 0, layoutWidth, layoutHeight);
      if (overlay) drawDateOverlay(context, layout, tokens, overlay);
      drawRaceGrid(context, layout, tokens, min, max, p.formatTick ?? formatAxisValue);
      if (!items.length && (!remote || remote.total === 0) && p.empty) drawRaceEmpty(context, layout, tokens, p.empty);

      const reached = hover.current.pointer ? null : rows.find(row => row.key === hover.current.keyed && row.phase !== 'leaving');
      if (reached && !rowSeen(layout, reached)) { hover.current = { ...hover.current, keyed: null }; spokenText.current = ''; setSpoken(''); }
      const { pointer, keyed } = hover.current;
      const hovered = pointer ? rowAt(rows, layout, pointer, min, max) : keyed ? rows.find(row => row.key === keyed && row.phase !== 'leaving') ?? null : null;

      context.save();
      context.beginPath();
      context.rect(0, layout.view.top - 4, layoutWidth, layout.view.bottom - layout.view.top + 8);
      context.clip();
      const ends = new Map<string, number>();
      let widest = 0;
      const drawn = paintOrder(inView);
      const alpha = (row: RaceRow) => Math.max(0, Math.min(1, row.alpha));
      for (const row of drawn) {
        const centre = rowCentre(layout, row.y);
        context.globalAlpha = alpha(row);
        if (row === hovered) {
          context.fillStyle = tokens.text;
          context.globalAlpha *= 0.05;
          context.fillRect(0, centre - layout.slot / 2, layoutWidth, layout.slot);
          context.globalAlpha = alpha(row);
        }
        const countryFlag = p.countryFlag?.(row.key, row.item);
        if (countryFlag) drawCountryFlag(context, countryFlag, FLAG.x, centre - FLAG.height / 2, FLAG.width, FLAG.height, tokens, wake);
        else {
          const url = p.flag?.(row.key, row.item);
          const image = url ? canvasImage(url, wake) : null;
          if (image) context.drawImage(image, FLAG.x, centre - FLAG.height / 2, FLAG.width, FLAG.height);
        }
      }
      context.fillStyle = tokens.text;
      context.font = font(tokens, tokens.fontSize.body, LABEL_WEIGHT);
      context.textAlign = 'left';
      context.textBaseline = 'middle';
      const nameWidth = layout.left - NAME_X - NAME_PAD;
      if (fitted.width !== nameWidth || fitted.tokens !== tokens) { fitted.names.clear(); fitted.width = nameWidth; fitted.tokens = tokens; }
      for (const row of drawn) {
        const name = p.name(row.key, row.item);
        let shown = fitted.names.get(name);
        if (shown === undefined) {
          if (fitted.names.size > 4096) fitted.names.clear();
          fitted.names.set(name, shown = ellipsise(name, nameWidth, text => context.measureText(text).width));
        }
        context.globalAlpha = alpha(row);
        context.fillText(shown, NAME_X, rowCentre(layout, row.y));
      }
      const tips: number[] = [];
      for (const row of drawn) { context.globalAlpha = alpha(row); tips.push(drawRaceBar(context, layout, tokens, row, min, max)); }
      context.fillStyle = tokens.text;
      context.font = font(tokens, tokens.fontSize.body, VALUE_WEIGHT);
      drawn.forEach((row, at) => {
        context.globalAlpha = alpha(row);
        context.fillText(row.item.label, tips[at] + VALUE_GAP, rowCentre(layout, row.y));
        tabular ??= new Set([...'0123456789'].map(digit => context.measureText(digit).width)).size === 1;
        const shape = tabular ? row.item.label.replace(/[0-9]/g, '0') : row.item.label;
        let labelWidth = labelWidths.get(shape);
        if (labelWidth === undefined) {
          if (labelWidths.size > 4096) labelWidths.clear();
          labelWidths.set(shape, labelWidth = context.measureText(row.item.label).width);
        }
        ends.set(row.key, tips[at] + VALUE_GAP + labelWidth);
        widest = Math.max(widest, labelWidth + VALUE_GAP + VALUE_PAD);
      });
      context.restore();
      context.globalAlpha = 1;
      const grew = !!p.live && widest > columns.value;
      if (grew) columns.value = widest;

      const heldTooltip = tooltipStore.get();
      if (hovered && p.tooltip && (pointer || keyIntent.current || heldTooltip == null || heldTooltip.owner === owner)) {
        if (!tooltipFor || tooltipFor.key !== hovered.key || tooltipFor.item !== hovered.item || tooltipFor.props !== p) {
          tooltipFor = { key: hovered.key, item: hovered.item, props: p, content: p.tooltip(hovered.key, hovered.item) };
        }
        const flag = p.countryFlag?.(hovered.key, hovered.item);
        const raw = tooltipFor.content && flag ? { ...tooltipFor.content, countryFlag: flag } : tooltipFor.content;
        const decoded = raw ? countryFlagContent(fitTooltip([raw], globalThis.innerHeight), undefined, wake) : null;
        const content = decoded ? fitTooltip([decoded], globalThis.innerHeight) : null;
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
      return raceMoving(state) || grew;
    };
    const current: FrameTask = { frame: draw };
    task.current = current;
    const wake = () => { forced.current = true; scheduler.wake(current); };
    const remove = scheduler.add(current, element);
    const unsubscribe = subscribeTokens(next => { tokens = next; wake(); });
    const local = (event: PointerEvent | MouseEvent): Pointer => ({
      ...toLocal(event.clientX, event.clientY, element.getBoundingClientRect(), element.clientWidth, element.clientHeight), clientX: event.clientX, clientY: event.clientY });
    const move = (event: PointerEvent) => { pendingKey.current = null; hover.current = { ...hover.current, pointer: local(event), keyed: null }; wake(); };
    const leave = () => { pendingKey.current = null; hover.current = { ...hover.current, pointer: null }; hideTooltip(owner); wake(); };
    const down = () => { pendingKey.current = null; hover.current = { ...hover.current, menuKey: false }; };
    const select = (event: MouseEvent) => {
      const at = shown.current;
      if (!at || !latest.current.active || !latest.current.onPick || at.owner !== (latest.current.remoteRows?.owner ?? latest.current.pickOwner) || at.resetKey !== latest.current.resetKey) return;
      const row = rowAt(at.rows, at.layout, local(event), at.min, at.max);
      selectRaceRow(row?.key ?? null, 'click', key => latest.current.onPick?.(key, row?.item));
    };
    const menu = (event: MouseEvent) => {
      if (hover.current.menuKey) { hover.current = { ...hover.current, menuKey: false }; event.preventDefault(); return; }
      const handler = latest.current.onContextMenu;
      const at = shown.current;
      if (!handler || !at || !latest.current.active || at.owner !== (latest.current.remoteRows?.owner ?? latest.current.pickOwner) || at.resetKey !== latest.current.resetKey) return;
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
    if (!props.active) hideTooltip(owner);
  }, [props.active, owner]);
  useEffect(() => { forced.current = true; if (task.current) scheduler.wake(task.current); });
  const live = props.live;
  useEffect(() => {
    if (!live) return;
    let drawn: readonly RaceItem[] | null = null;
    const check = () => { const items = live.items(); if (items !== drawn && task.current) { drawn = items; scheduler.wake(task.current); } };
    const stops = [live.clock.subscribeFrame(check), live.clock.subscribe(check), live.bind(() => { drawn = null; check(); })];
    check();
    return () => { for (const stop of stops) stop(); };
  }, [live]);

  const wake = () => { forced.current = true; if (task.current) scheduler.wake(task.current); };
  const clearKeyed = () => {
    pendingKey.current = null;
    hover.current = { ...hover.current, keyed: null };
    spokenText.current = '';
    setSpoken('');
    wake();
  };
  const keyDown = (event: ReactKeyboardEvent) => {
    if (!props.active) return;
    if (event.key === 'ArrowUp' || event.key === 'ArrowDown' || event.key === 'Home' || event.key === 'End') {
      event.preventDefault();
      const from = hover.current.keyed ?? (hover.current.pointer && shown.current ? rowAt(shown.current.rows, shown.current.layout, hover.current.pointer, shown.current.min, shown.current.max)?.key ?? null : null);
      const remote = props.remoteRows;
      if (remote) {
        const local = props.items.findIndex(item => item.key === from);
        const pending = pendingKey.current;
        const previous = pending && pending.owner === remote.owner ? pending.index : local < 0 ? -1 : remote.offset + local;
        const index = remote.total === 0 ? -1 : event.key === 'Home' ? 0 : event.key === 'End' ? remote.total - 1
          : previous < 0 ? 0 : event.key === 'ArrowDown' ? Math.min(remote.total - 1, previous + 1) : Math.max(0, previous - 1);
        const item = props.items[index - remote.offset];
        hover.current = { pointer: null, keyed: item?.key ?? null, menuKey: false };
        pendingKey.current = index < 0 || item ? null : { index, owner: remote.owner, resetKey: props.resetKey };
        const layout = shown.current?.layout, box = scroller.current;
        if (box && layout && index >= 0) {
          box.scrollTop = scrollTop.current = Math.max(0, revealRow(index, scrollTop.current, layout.view.bottom - layout.view.top, layout.slot));
          const first = Math.max(0, Math.floor(scrollTop.current / layout.slot));
          const count = Math.min(remote.total - first, Math.ceil((layout.view.bottom - layout.view.top) / layout.slot) + 1);
          requestRange(remote, first, count);
        } else if (index >= 0) requestRange(remote, index, 1);
        keyIntent.current = true; wake(); return;
      }
      const items = shown.current?.items ?? (props.live ? props.live.items() : props.items);
      const keyed = keyedEntry(event.key, from, items);
      hover.current = { ...hover.current, pointer: null, keyed };
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
    if (event.key === 'Escape' && (hover.current.keyed != null || pendingKey.current)) { event.preventDefault(); clearKeyed(); return; }
    const keyed = hover.current.keyed;
    const drawn = shown.current;
    const row = drawn?.rows.find(item => item.key === keyed && item.phase !== 'leaving');
    const current = latest.current;
    if (!current.active || drawn?.owner !== (current.remoteRows?.owner ?? current.pickOwner) || drawn?.resetKey !== current.resetKey || !row || !drawn || !rowSeen(drawn.layout, row)) return;
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
  const ranking = live ? '' : raceText(props.items, props.name, props.remoteRows?.offset ?? 0);
  return <div className="race-chart">
    <div ref={scroller} className="race-scroll themed-scrollbar" onScroll={event => {
      if (event.currentTarget.scrollTop !== scrollTop.current) pendingKey.current = null;
      scrollTop.current = event.currentTarget.scrollTop; wake();
    }}>
      <canvas ref={canvas} tabIndex={0} role="img" aria-label={live ? undefined : props.label} aria-describedby={`${owner}-ranking`}
        aria-keyshortcuts={`ArrowUp ArrowDown Home End${props.onPick ? ' Enter Space' : ''}`} onKeyDown={keyDown} onBlur={clearKeyed} />
      { }
      <div ref={extent} className="race-extent" style={live ? undefined : { height: `calc(${raceHeight(props.remoteRows?.total ?? props.items.length)}px - 100%)` }} />
    </div>
    <p ref={ranked} id={`${owner}-ranking`} className="sr-only">{live ? null : props.remoteRows ? `${props.remoteRows.total} ranked entries. ${ranking
      || (props.remoteRows.total === 0 && props.empty) || 'Requested rows are loading.'}` : ranking || props.empty || ''}</p>
    <p className="sr-only" aria-live="polite">{spoken}</p>
  </div>;
}
