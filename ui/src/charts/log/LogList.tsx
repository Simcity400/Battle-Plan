import { failureText } from '../../dashboard/queryError.ts';
import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import type { MenuItem } from '../../components/ContextMenu';
import { fitTooltip, hideTooltip, showTooltip, tooltipStore, tooltipText } from '../tooltip.ts';
import type { TooltipContent } from '../tooltip';
import { useRowMotion, movingRowClass } from '../useRowMotion';
import { untakenRows } from '../rowMotion.ts';
import { useChartTokens, useViewportHeight } from '../useChartTokens';
import { anchoredScrollTop, cardHeightsAt, cardPageTarget, hasLogPartTip, keyTarget, logDate, logItems, logOffsets, logRowColumns, logSectionItems, MONTH_HEIGHT, stableLogSections, newCardKeys, ROW_HEIGHT, rowId, scrollToShow, shownMotion, updateCardHeights, visibleRange } from './logData.ts';
import type { CardMeasurements, LogColumns, LogDensity, LogItem, LogPage, LogRow, LogSection, LogTip } from './logData';
import { logPageEntries, logPageItems, logPageRange, logPageTarget, logPixelTarget, logRowOrdinals, logRowTarget, wantedLogPages } from './logPages.ts';
import type { LogRowTarget } from './logPages';
import { EMPTY_LOG_REVEALS, logRevealHistory, logReveals, logVisibleEntries } from './logReveal.ts';
import { logCut, logHeldRange, logItemState, logShownOffsets, logShownOrdinals, logSpans, logStateAt } from './logAhead.ts';
import type { LogSpans } from './logAhead';
import type { RevealClock } from '../reveal';
import { logicalScroll, physicalScroll, scrollGeometry, scrollItemTop, wheelScrollDelta } from '../scrollGeometry.ts';
import type { ScrollGeometry } from '../scrollGeometry';
import '../charts.css';
import './log.css';
import { inFrame } from '../../dashboard/frameUpdates.ts';
import { scheduler } from '../scheduler.ts';
import { engine } from '../../engine/frame.ts';
import type { FrameTask } from '../scheduler';
import { EntryLayer, backdrop, backgroundsBehind, entryTime } from './entryLayer.ts';
import { useWhyCommit } from '../../dashboard/widgetPerf.ts';

export interface LogListProps<T> {
  rows?: readonly LogRow<T>[];
  pages?: readonly LogPage<T>[];
  sections?: readonly LogListSection<T>[];
  onWantedPages?: (keys: readonly string[]) => void;
  label: string;
  density: LogDensity;
  months: boolean;
  animate: boolean;
  playhead?: number | null;
  layout?: 'row' | 'card';
  rowHeight?: (row: LogRow<T>, density: LogDensity) => number;
  renderRow: (row: LogRow<T>) => ReactNode;
  columns?: LogColumns<T>;
  describe: (row: LogRow<T>) => string;
  tooltip?: (row: LogRow<T>) => readonly TooltipContent[];
  onTooltip?: (row: LogRow<T> | null) => void;
  menu?: (row: LogRow<T>) => MenuItem[];
  empty?: ReactNode;
  onVisible?: (rows: readonly LogRow<T>[]) => void;
  anchorRows?: boolean;
  reveal?: RevealClock;
  states?: boolean;
  bindRow?: (row: LogRow<T>) => Node;
}

export interface LogListSection<T> extends LogSection<T> { readonly note: ReactNode }

const EMPTY_ROWS: readonly LogRow<never>[] = [];
interface Viewport { height: number; width: number; outerHeight: number }
const measureViewport = (element: HTMLElement): Viewport => ({ height: element.clientHeight, width: element.clientWidth, outerHeight: element.offsetHeight });
const READER_SCROLL_MS = 1000;
interface MonthStart { top: number; label: string | null }
interface Laid<T> { items: readonly LogItem<T>[]; offsets: readonly number[]; monthStarts: readonly MonthStart[]; density: LogDensity; paged: boolean; months: boolean; card: boolean;
  ahead: { full: readonly number[]; spans: LogSpans; hour: number; cut: number } | null }
interface Drawn { start: number; end: number; month: string | null; width: number }

const monthStartsOf = <T,>(items: readonly LogItem<T>[], offsets: readonly number[]): MonthStart[] => items.flatMap<MonthStart>((item, index) =>
  item.kind === 'month' ? item.part === 'note' ? [] : [{ top: offsets[index], label: item.label }] : item.kind === 'page' ? [{ top: offsets[index], label: null }] : []);

function monthAt(starts: readonly MonthStart[], months: boolean, top: number): string | null {
  if (!months || top <= 0) return null;
  let low = 0, high = starts.length - 1, found = -1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (starts[middle].top <= top) { found = middle; low = middle + 1; } else high = middle - 1;
  }
  return found < 0 ? null : starts[found].label;
}

const viewAt = <T,>(laid: Laid<T>, top: number, outerHeight: number, overscan: number): [number, number] =>
  laid.paged ? logPageRange(laid.offsets, top, outerHeight, laid.density, overscan) : visibleRange(laid.offsets, top, outerHeight, overscan);

const windowAt = <T,>(laid: Laid<T>, top: number, outerHeight: number, anchored: boolean, overscan = 6): [number, number] => laid.ahead
  ? logHeldRange(laid.offsets, laid.ahead.full, laid.ahead.spans, laid.ahead.hour, top, outerHeight, overscan, anchored)
  : viewAt(laid, top, outerHeight, overscan);

function aheadAt<T>(full: readonly number[], spans: LogSpans | null, hour: number | null): { offsets: readonly number[]; ahead: Laid<T>['ahead'] } {
  if (!spans || hour === null) return { offsets: full, ahead: null };
  return { offsets: logShownOffsets(full, spans, hour), ahead: { full, spans, hour, cut: logCut(spans, hour) } };
}

function slotTop<T>(laid: Laid<T>, slot: number): number {
  const count = laid.items.length;
  return slot < count ? laid.offsets[slot] : laid.offsets[count] + (slot - count) * ROW_HEIGHT[laid.density];
}

function placement<T>(laid: Laid<T>, space: ScrollGeometry, top: number, height: number, slot: number, pageRows: number): { transform: string; height: number; label: string } {
  const row = ROW_HEIGHT[laid.density], at = slotTop(laid, slot);
  const margin = row * 6;
  const from = Math.max(at, top - margin);
  const shown = Math.max(0, Math.min(at + pageRows * row, top + height + margin) - from);
  return { transform: `translateY(${scrollItemTop(space, from, top)}px)`, height: shown,
    label: `translateY(${Math.max(0, Math.min(Math.max(0, shown - row), top - from))}px)` };
}

const stateRowKey = (id: string) => JSON.stringify(['states', id]);

function showState(row: HTMLElement, index: number) {
  const parts = row.children as HTMLCollectionOf<HTMLElement>;
  for (let at = 0; at < parts.length; at++) if (parts[at].hidden !== (at !== index)) parts[at].hidden = at !== index;
  const part = parts[index];
  if (!part) return;
  if (row.getAttribute('aria-label') !== part.dataset.label) row.setAttribute('aria-label', part.dataset.label ?? '');
  if (part.dataset.key != null && row.dataset.logKey !== part.dataset.key) row.dataset.logKey = part.dataset.key;
  if (part.dataset.columns != null) row.style.setProperty('--log-columns', part.dataset.columns);
  if (part.dataset.height != null) row.style.height = `${part.dataset.height}px`;
}

const POOL_OVERSCAN = 3;

interface Pool<T> {
  bind: (row: LogRow<T>) => Node;
  describe: (row: LogRow<T>) => string;
  columns: LogColumns<T> | undefined;
  optionId: (id: string) => string;
  current: string | null;
  slotsById: ReadonlyMap<string, readonly number[]>;
  grouping: boolean;
  bound: Map<string, HTMLElement>;
  short: boolean;
}

const boundRows = new WeakMap<HTMLElement, { id: string; key: string; bind: unknown }>();

function dateCell(row: LogRow<unknown>): HTMLElement {
  const date = logDate(row.hour), cell = document.createElement('span'), day = document.createElement('span');
  cell.className = 'log-when';
  if (row.by) { const by = document.createElement('small'); by.textContent = 'by '; day.append(by); }
  day.append(date.day);
  cell.append(day);
  if (date.year) { const year = document.createElement('small'); year.textContent = date.year; cell.append(year); }
  return cell;
}

function bindPool<T>(glide: HTMLElement, laid: Laid<T>, pool: Pool<T>, top: number, outerHeight: number) {
  const ahead = laid.ahead;
  if (!ahead) return;
  const [from, to] = visibleRange(laid.offsets, top, outerHeight, POOL_OVERSCAN);
  const want = new Map<string, number>();
  for (let index = from; index < to; index++) {
    const item = laid.items[index];
    if (item?.kind !== 'row') continue;
    const id = rowId(item.row);
    if (want.has(id)) continue;
    const slots = pool.grouping ? pool.slotsById.get(id) : undefined;
    const at = slots ? logStateAt(ahead.spans, slots, ahead.hour) : { slot: index, state: logItemState(ahead.spans, index, ahead.hour) };
    if (at.state === 'shown' || (at.state === 'gone' && pool.bound.has(id))) want.set(id, at.slot);
  }
  const boxes = [...glide.children as HTMLCollectionOf<HTMLElement>].filter(child => child.dataset.pool != null);
  const kept = new Map<string, HTMLElement>();
  const free: HTMLElement[] = [];
  for (const box of boxes) {
    const held = boundRows.get(box);
    if (held && want.has(held.id) && !kept.has(held.id)) kept.set(held.id, box);
    else free.push(box);
  }
  const bound = new Map<string, HTMLElement>();
  pool.short = false;
  for (const [id, slot] of want) {
    const row = (laid.items[slot] as Extract<LogItem<T>, { kind: 'row' }>).row;
    const box = kept.get(id) ?? free.shift();
    if (!box) { pool.short = true; break; }
    const fresh = !kept.has(id);
    bound.set(id, box);
    const held = boundRows.get(box);
    if (fresh || held?.key !== row.key || held.bind !== pool.bind) {
      box.replaceChildren(dateCell(row), pool.bind(row));
      boundRows.set(box, { id, key: row.key, bind: pool.bind });
      box.dataset.logId = id;
      box.dataset.logKey = row.key;
      box.id = pool.optionId(id);
      box.setAttribute('aria-label', pool.describe(row));
      if (pool.columns == null) box.style.removeProperty('--log-columns');
      else box.style.setProperty('--log-columns', logRowColumns(pool.columns, row));
      if (fresh) { box.dataset.reveal = 'ahead'; written.delete(box); }
    }
    box.dataset.slot = String(slot);
    const active = id === pool.current;
    if (box.classList.contains('is-active') !== active) box.classList.toggle('is-active', active);
    if (box.getAttribute('aria-selected') !== String(active)) box.setAttribute('aria-selected', String(active));
  }
  pool.bound = bound;
  for (const box of free) {
    if (!boundRows.has(box) && box.dataset.slot == null) continue;
    if ([...bound.values()].includes(box)) continue;
    boundRows.delete(box);
    delete box.dataset.slot;
    delete box.dataset.logId;
    delete box.dataset.logKey;
    box.removeAttribute('id');
    box.dataset.reveal = 'ahead';
    box.inert = true;
    box.replaceChildren();
    written.delete(box);
  }
}

const written = new WeakMap<HTMLElement, number>();
const SAME = 1e-6;
let tops = new Float64Array(256);

const stilled = new WeakMap<HTMLElement, HTMLElement[]>();
let stilling: HTMLElement[] | null = null;
function placeStill(element: HTMLElement) {
  element.style.transition = 'none';
  if (!stilling) {
    const batch: HTMLElement[] = stilling = [];
    engine.next('after', () => {
      if (stilling === batch) stilling = null;
      engine.next('after', () => {
        for (const held of batch) if (stilled.get(held) === batch) { stilled.delete(held); held.style.transition = ''; }
      });
    });
  }
  stilling.push(element);
  stilled.set(element, stilling);
}

function paintItems<T>(host: HTMLElement, glide: HTMLElement | null, laid: Laid<T>, space: ScrollGeometry, top: number, height: number, enter = false,
  onEnter?: (row: HTMLElement, height: number) => void) {
  for (const child of host.children as HTMLCollectionOf<HTMLElement>) {
    const pageRows = child.dataset.pageRows;
    if (pageRows == null) continue;
    const place = placement(laid, space, top, height, Number(child.dataset.slot), Number(pageRows));
    child.style.transform = place.transform;
    child.style.height = `${place.height}px`;
    const label = child.firstElementChild as HTMLElement | null;
    if (label) label.style.transform = place.label;
  }
  if (!glide) return;
  const items = glide.children as HTMLCollectionOf<HTMLElement>;
  if (tops.length < items.length) tops = new Float64Array(Math.max(items.length, tops.length * 2));
  const base = written.get(glide) ?? 0;
  let shift = 0, votes = 0;
  for (let index = 0; index < items.length; index++) {
    const child = items[index];
    tops[index] = NaN;
    let slot = child.dataset.slot, stateAt: ReturnType<typeof logStateAt> | null = null;
    const states = child.dataset.states;
    if (states != null) {
      const slots = states.split(' ').map(Number);
      if (laid.ahead) {
        stateAt = logStateAt(laid.ahead.spans, slots, laid.ahead.hour);
        showState(child, slots.indexOf(stateAt.slot));
      }
      slot = String(stateAt?.slot ?? slots[0]);
    }
    if (slot == null) continue;
    if (laid.ahead) {
      const state = stateAt?.state ?? logItemState(laid.ahead.spans, Number(slot), laid.ahead.hour), was = child.dataset.reveal;
      if (state !== 'shown') { if (was !== state) { child.dataset.reveal = state; child.inert = true; } }
      else if (was === 'ahead' || was === 'gone') {
        if (enter && was === 'ahead') {
          child.dataset.reveal = 'in';
          onEnter?.(child, slotTop(laid, Number(slot) + 1) - slotTop(laid, Number(slot)));
        } else delete child.dataset.reveal;
        child.inert = false;
      }
      if (state === 'ahead') { written.delete(child); continue; }
    } else if (child.dataset.reveal != null) { delete child.dataset.reveal; child.inert = false; }
    const at = scrollItemTop(space, slotTop(laid, Number(slot)), top), was = written.get(child);
    tops[index] = at;
    if (was === undefined) continue;
    const moved = at - base - was;
    if (votes === 0) { shift = moved; votes = 1; } else if (Math.abs(moved - shift) < SAME) votes++; else votes--;
  }
  let offset = votes ? base + shift : base;
  if (Math.abs(offset) > space.extent) offset = 0;
  if (written.get(glide) === undefined || Math.abs(offset - base) >= SAME) {
    glide.style.transform = `translateY(${offset}px)`;
    written.set(glide, offset);
  }
  for (let index = 0; index < items.length; index++) {
    const at = tops[index];
    if (Number.isNaN(at)) continue;
    const child = items[index], to = at - offset, was = written.get(child);
    if (was !== undefined && Math.abs(to - was) < SAME) continue;
    if (was === undefined) placeStill(child);
    child.style.transform = `translateY(${to}px)`;
    written.set(child, to);
  }
}

export default function LogList<T>({ rows = EMPTY_ROWS, pages, sections, onWantedPages, label, density, months, animate, playhead: givenPlayhead, layout = 'row', rowHeight, renderRow, columns, describe, tooltip, onTooltip, menu, empty, onVisible, anchorRows = false, reveal: revealBy, states = false, bindRow }: LogListProps<T>) {
  const revealHour = revealBy && typeof givenPlayhead === 'number' ? revealBy.hour() : null;
  const playhead = revealHour ?? givenPlayhead;
  const owner = useId();
  const listId = useId();
  const viewportHeight = useViewportHeight();
  const tokens = useChartTokens();
  const { font, fontSize, reducedMotion } = tokens;
  const cardFont = JSON.stringify([font, fontSize.small, fontSize.body, fontSize.title]);
  const [scroller, setScroller] = useState<HTMLDivElement | null>(null);
  const body = useRef<HTMLDivElement | null>(null);
  const glide = useRef<HTMLDivElement | null>(null);
  const layered = layout === 'row' && animate === true && !reducedMotion && !!revealBy;
  const under = useRef<HTMLCanvasElement | null>(null);
  const over = useRef<HTMLCanvasElement | null>(null);
  const entries = useRef<{ layer: EntryLayer; task: FrameTask } | null>(null);
  const pool = useRef<Pool<T> | null>(null);
  const [poolExtra, setPoolExtra] = useState(0);
  const viewport = useRef<Viewport>({ height: 0, width: 0, outerHeight: 0 });
  const scrollPosition = useRef({ logical: 0, physical: 0 });
  const scrolledTo = useRef<{ logical: number; physical: number } | null>(null);
  const entering = useRef(false);
  const emptyNote = useRef<HTMLDivElement | null>(null);
  const [redraws, setRedraws] = useState(0);
  const redraw = useCallback(() => setRedraws(count => count + 1), []);
  const [active, setActive] = useState<string | null>(null);
  const [pending, setPending] = useState<{ pageKey: string; offset: number } | null>(null);
  const [spoken, setSpoken] = useState('');
  const tip = useRef<LogTip | null>(null);
  const tipKey = useRef<string | null>(null);
  const showTipNow = useRef<(next: LogTip) => void>(() => {});
  const rowNow = useRef<(id: string) => LogRow<T> | null>(() => null);
  const cardElements = useRef(new Map<string, HTMLDivElement>());
  const cardObserver = useRef<ResizeObserver | null>(null);
  const knownCards = useRef(new Set<string>());
  const measurements = useRef<CardMeasurements>({ width: 0, density, heights: new Map() });
  const actions = useContextMenu(`${label} actions`);
  useWhyCommit('LogList', { rows, pages, sections, redraws, active, pending, spoken, scroller, playhead: givenPlayhead, reveal: revealBy, renderRow, tooltip, onVisible, poolExtra });

  const holding = revealHour !== null;
  const keptSections = useRef<readonly LogListSection<T>[] | undefined>(undefined);
  const groups = keptSections.current = sections ? stableLogSections(keptSections.current, sections) : undefined;
  const sectionPages = useMemo(() => groups?.some(section => section.pages) ? groups.flatMap(section => section.pages ?? []) : undefined, [groups]);
  const allPages = pages ?? sectionPages;
  const items = useMemo(() => groups ? logSectionItems(groups, months, holding, logPageEntries) : pages ? logPageItems(pages, months) : logItems(rows, months),
    [rows, pages, groups, months, holding]);
  const notes = useMemo(() => new Map(sections?.map(section => [section.key, section.note])), [sections]);
  const heldKeys = useMemo(() => allPages ? new Set(items.map(item => item.key)) : undefined, [allPages, items]);
  const motionItems = useMemo(() => items.map(item => ({ key: item.key, item })), [items]);
  const moving = useRowMotion<LogItem<T>>(motionItems, true);
  const { width, outerHeight, height: viewHeight } = viewport.current;
  const heightOf = useCallback((heights: ReadonlyMap<string, number>) => (row: LogRow<T>) =>
    (layout === 'card' ? heights.get(row.key) : undefined) ?? rowHeight?.(row, density) ?? ROW_HEIGHT[density], [layout, rowHeight, density]);
  const measuredHeights = cardHeightsAt(measurements.current, width, density, cardFont);
  const fullOffsets = useMemo(() => logOffsets(items, density, heightOf(measuredHeights)), [items, density, heightOf, measuredHeights]);
  const spans = useMemo(() => holding ? logSpans(items) : null, [holding, items]);
  const cut = spans && revealHour !== null ? logCut(spans, revealHour) : null;
  const hourHeld = useRef(revealHour);
  hourHeld.current = revealHour;
  const { offsets, ahead } = useMemo(() => cut === null ? aheadAt<T>(fullOffsets, null, null) : aheadAt<T>(fullOffsets, spans, hourHeld.current), [fullOffsets, spans, cut]);
  const monthStarts = useMemo(() => monthStartsOf(items, offsets), [items, offsets]);
  const laid = useMemo<Laid<T>>(() => ({ items, offsets, monthStarts, density, paged: allPages !== undefined, months, card: layout === 'card', ahead }),
    [items, offsets, monthStarts, density, allPages, months, layout, ahead]);
  const latest = useRef<Laid<T>>(laid);
  const drawnView = useRef<Drawn | null>(null);
  const priorLayout = useRef<{ items: readonly LogItem<T>[]; offsets: readonly number[]; top: number } | null>(null);
  const slotsById = useMemo(() => {
    const slots = new Map<string, number[]>();
    items.forEach((item, index) => {
      if (item.kind !== 'row') return;
      const id = rowId(item.row), held = slots.get(id);
      if (held) held.push(index); else slots.set(id, [index]);
    });
    return slots;
  }, [items]);
  const grouping = states && ahead !== null && layout === 'row';
  const pooled = bindRow !== undefined && layout === 'row' && ahead !== null && !months && !rowHeight && !allPages && !sections;
  const poolSize = pooled ? Math.ceil(outerHeight / ROW_HEIGHT[density]) + 2 * POOL_OVERSCAN + 2 + poolExtra : 0;
  const slotOf = useCallback((id: string): number | undefined => {
    const slots = slotsById.get(id), now = latest.current.ahead;
    if (!slots || !grouping || !now) return slots?.[slots.length - 1];
    const at = logStateAt(now.spans, slots, now.hour);
    return at.state === 'shown' ? at.slot : undefined;
  }, [slotsById, grouping]);
  const rowOf = useCallback((id: string): LogRow<T> | null => {
    const slot = slotOf(id), item = slot === undefined ? undefined : items[slot];
    return item?.kind === 'row' ? item.row : null;
  }, [slotOf, items]);
  const rowSlots = useMemo(() => items.flatMap((item, index) => item.kind === 'row' ? [index] : []), [items]);
  const ordinals = useMemo(() => logRowOrdinals(items), [items]);
  const keyRows = useCallback((): { ordinals: readonly number[]; shown: (slot: number) => boolean } => {
    const now = latest.current.ahead;
    if (!now) return { ordinals, shown: () => true };
    return { ordinals: logShownOrdinals(items, now.spans, now.hour), shown: (slot: number) => logItemState(now.spans, slot, now.hour) === 'shown' };
  }, [items, ordinals]);
  const placeOf = (id: string): number | undefined => {
    const slots = slotsById.get(id), now = latest.current.ahead;
    if (!slots) return undefined;
    return grouping && now ? logStateAt(now.spans, slots, now.hour).slot : slots[slots.length - 1];
  };
  const pagesByKey = useMemo(() => new Map(allPages?.map(page => [page.key, page])), [allPages]);
  const revealHistory = useRef(EMPTY_LOG_REVEALS);
  const reveals = useMemo(() => logReveals(revealHistory.current, items, playhead), [items, playhead]);
  useLayoutEffect(() => { revealHistory.current = logRevealHistory(revealHistory.current, reveals, playhead); }, [reveals, playhead]);
  const explicitMotion = allPages !== undefined || playhead !== undefined;
  const continueAnimate = useRef(animate && !reducedMotion);
  continueAnimate.current = animate && !reducedMotion;
  const continueMotion = animate && (playhead === undefined || (typeof playhead === 'number'
    && typeof revealHistory.current.playhead === 'number' && playhead >= revealHistory.current.playhead));
  const geometry = scrollGeometry(offsets[items.length], viewHeight);
  const previous = priorLayout.current;
  const top = Math.max(0, Math.min(geometry.logicalRange, anchorRows && previous && (previous.items !== items || ((layout === 'card' || ahead !== null) && previous.offsets !== offsets))
    ? anchoredScrollTop(previous.items, previous.offsets, items, offsets, previous.top) : scrollPosition.current.logical));
  const sticky = months ? MONTH_HEIGHT : 0;
  const [start, end] = windowAt(laid, top, outerHeight, anchorRows);
  const topMonth = monthAt(monthStarts, months, top);

  const covers = useCallback(() => {
    const drawn = drawnView.current, now = latest.current, view = viewport.current;
    if (!drawn || pool.current) return true;
    if (now.card && view.width !== drawn.width) return false;
    const at = scrollPosition.current.logical;
    if (monthAt(now.monthStarts, now.months, at) !== drawn.month) return false;
    const [from, to] = now.ahead ? visibleRange(now.offsets, at, view.outerHeight, 0) : viewAt(now, at, view.outerHeight, 3);
    if (from >= drawn.start && to <= drawn.end) return true;
    const [start, end] = windowAt(now, at, view.outerHeight, anchorRows);
    return start === drawn.start && end === drawn.end;
  }, [anchorRows]);
  const place = useCallback(() => {
    const now = latest.current, view = viewport.current;
    const layer = entries.current;
    const pooled = pool.current;
    if (pooled && glide.current) {
      bindPool(glide.current, now, pooled, scrollPosition.current.logical, view.outerHeight);
      if (pooled.short) { pooled.short = false; inFrame(() => setPoolExtra(extra => extra + POOL_OVERSCAN * 2)); }
    }
    if (body.current) paintItems(body.current, glide.current, now, scrollGeometry(now.offsets[now.items.length], view.height), scrollPosition.current.logical, view.height, entering.current,
      layer ? (row, height) => { layer.layer.add(row, height, entryTime.now()); scheduler.wake(layer.task); } : undefined);
    if (emptyNote.current && now.ahead) emptyNote.current.hidden = now.offsets[now.items.length] > 0;
  }, []);
  const paint = useCallback(() => {
    place();
    if (!covers()) redraw();
  }, [place, covers, redraw]);
  const setLogicalTop = useCallback((requested: number, measured = false) => {
    if (!scroller) return;
    if (!viewport.current.height) viewport.current = measureViewport(scroller);
    const now = latest.current;
    const space = scrollGeometry(now.offsets[now.items.length], viewport.current.height);
    const at = Math.max(0, Math.min(space.logicalRange, requested));
    const extent = `${space.extent}px`, physical = physicalScroll(space, at);
    if (body.current && body.current.style.height !== extent) body.current.style.height = extent;
    const held = scrolledTo.current;
    if (!measured || !held || held.logical !== at || held.physical !== physical || scrollPosition.current.logical !== at) {
      scroller.scrollTop = physical;
      scrollPosition.current = { logical: at, physical };
      scrolledTo.current = { logical: at, physical };
    }
    if (priorLayout.current) priorLayout.current.top = at;
    paint();
  }, [scroller, paint]);
  const readerScrolled = useRef(-Infinity);
  const readerScrolls = () => { readerScrolled.current = performance.now(); };
  const readScroll = useCallback((element: HTMLDivElement) => {
    if (performance.now() - readerScrolled.current > READER_SCROLL_MS && scrolledTo.current) return;
    if (!viewport.current.height) viewport.current = measureViewport(element);
    const now = latest.current;
    const space = scrollGeometry(now.offsets[now.items.length], viewport.current.height);
    const top = element.scrollTop;
    const at = Math.abs(top - scrollPosition.current.physical) < 1
      ? Math.min(space.logicalRange, scrollPosition.current.logical) : logicalScroll(space, top);
    scrollPosition.current = { logical: at, physical: top };
    if (priorLayout.current) priorLayout.current.top = at;
    paint();
  }, [paint]);
  useLayoutEffect(() => {
    if (!pooled || !bindRow) { pool.current = null; return; }
    pool.current = { bind: bindRow, describe, columns, optionId, current, slotsById, grouping, bound: pool.current?.bound ?? new Map(), short: false };
  });
  useLayoutEffect(() => {
    latest.current = laid;
    drawnView.current = { start, end, month: topMonth, width };
    priorLayout.current = { items: laid.items, offsets: laid.offsets, top };
    setLogicalTop(top);
  }, [laid, start, end, topMonth, width, outerHeight, top, redraws, setLogicalTop]);
  useLayoutEffect(() => { place(); });
  useLayoutEffect(() => {
    if (!layered || !under.current || !over.current || !scroller) return;
    const layer = new EntryLayer(under.current, over.current);
    const task: FrameTask = { frame: () => layer.paint(entryTime.now(), row => written.get(row), viewport.current.width) };
    entries.current = { layer, task };
    const remove = scheduler.add(task, over.current);
    return () => { remove(); layer.clear(); entries.current = null; };
  }, [layered, scroller]);
  useLayoutEffect(() => {
    const layer = entries.current?.layer;
    if (!layer || !under.current || !scroller) return;
    layer.flash = getComputedStyle(under.current).color;
    const pixel = document.createElement('canvas').getContext('2d', { willReadFrequently: true });
    if (pixel) layer.cover = backdrop(backgroundsBehind(scroller), pixel);
  }, [layered, scroller, tokens]);

  useLayoutEffect(() => {
    if (!scroller) return;
    viewport.current = measureViewport(scroller);
    setLogicalTop(scrollPosition.current.logical);
    let alive = true;
    const observer = new ResizeObserver(() => {
      viewport.current = measureViewport(scroller);
      inFrame(() => { if (alive) setLogicalTop(scrollPosition.current.logical); });
    });
    observer.observe(scroller);
    return () => { alive = false; observer.disconnect(); };
  }, [scroller, setLogicalTop]);
  useLayoutEffect(() => {
    if (!revealBy) return;
    const step = () => {
      const now = latest.current;
      if (!now.ahead) return;
      const hour = revealBy.hour(), cut = logCut(now.ahead.spans, hour);
      if (cut === now.ahead.cut) return;
      const forward = hour > now.ahead.hour && revealBy.playing();
      const { offsets: moved, ahead: held } = aheadAt<T>(now.ahead.full, now.ahead.spans, hour);
      const prior = priorLayout.current;
      const at = anchorRows && prior ? anchoredScrollTop(prior.items, prior.offsets, now.items, moved, prior.top) : scrollPosition.current.logical;
      latest.current = { ...now, offsets: moved, monthStarts: monthStartsOf(now.items, moved), ahead: held };
      priorLayout.current = { items: now.items, offsets: moved, top: at };
      revealHistory.current = logRevealHistory(revealHistory.current, logReveals(revealHistory.current, now.items, hour), hour);
      entering.current = forward && continueAnimate.current;
      try { setLogicalTop(at, true); } finally { entering.current = false; }
      const shown = tip.current;
      if (shown && tooltipStore.get()?.owner === owner && (rowNow.current(shown.id)?.key ?? null) !== tipKey.current) showTipNow.current(shown);
    };
    return revealBy.subscribe(step);
  }, [revealBy, anchorRows, setLogicalTop, owner]);
  useEffect(() => {
    if (!scroller || !geometry.compressed) return;
    const wheel = (event: WheelEvent) => {
      if (event.ctrlKey || !event.deltaY) return;
      event.preventDefault();
      setLogicalTop(scrollPosition.current.logical + wheelScrollDelta(event.deltaY, event.deltaMode, ROW_HEIGHT[density], scroller.clientHeight));
    };
    scroller.addEventListener('wheel', wheel, { passive: false });
    return () => scroller.removeEventListener('wheel', wheel);
  }, [scroller, geometry.compressed, density, setLogicalTop]);

  const finishCardEntry = useCallback((event: AnimationEvent) => {
    if (event.target === event.currentTarget && event.animationName === 'log-card-in') {
      (event.currentTarget as HTMLDivElement).classList.remove('is-entering');
    }
  }, []);
  const attachCard = useCallback((key: string, node: HTMLDivElement | null) => {
    const previous = cardElements.current.get(key);
    if (previous) {
      cardObserver.current?.unobserve(previous);
      previous.removeEventListener('animationend', finishCardEntry);
      previous.removeEventListener('animationcancel', finishCardEntry);
    }
    if (node) {
      cardElements.current.set(key, node);
      cardObserver.current?.observe(node);
      node.addEventListener('animationend', finishCardEntry);
      node.addEventListener('animationcancel', finishCardEntry);
    } else cardElements.current.delete(key);
  }, [finishCardEntry]);

  useLayoutEffect(() => {
    if (layout !== 'card') return;
    const measure = (nodes: readonly HTMLDivElement[]) => {
      if (cardObserver.current !== observer) return;
      const sizes = nodes.map(node => ({ key: node.dataset.logKey!, height: node.offsetHeight }));
      const before = measurements.current;
      const next = updateCardHeights(before, width, density, sizes, cardFont, heldKeys);
      if (next === before) return;
      measurements.current = next;
      const now = latest.current;
      const full = logOffsets(now.items, density, heightOf(cardHeightsAt(next, width, density, cardFont)));
      const { offsets: moved, ahead: held } = aheadAt<T>(full, now.ahead?.spans ?? null, now.ahead?.hour ?? null);
      const prior = priorLayout.current;
      const at = anchorRows && prior ? anchoredScrollTop(prior.items, prior.offsets, now.items, moved, prior.top) : scrollPosition.current.logical;
      latest.current = { ...now, offsets: moved, monthStarts: monthStartsOf(now.items, moved), ahead: held };
      priorLayout.current = { items: now.items, offsets: moved, top: at };
      setLogicalTop(at);
    };
    const observer = new ResizeObserver(entries => measure(entries.map(entry => entry.target as HTMLDivElement)));
    cardObserver.current = observer;
    const nodes = [...cardElements.current.values()];
    for (const node of nodes) observer.observe(node);
    measure(nodes);
    return () => { observer.disconnect(); cardObserver.current = null; };
  }, [layout, width, density, cardFont, heldKeys, heightOf, anchorRows, setLogicalTop]);

  useLayoutEffect(() => {
    if (layout !== 'card') return;
    if (heldKeys) for (const key of knownCards.current) if (!heldKeys.has(key)) knownCards.current.delete(key);
    const enabled = continueMotion && !reducedMotion;
    if (!enabled) for (const node of cardElements.current.values()) node.classList.remove('is-entering');
    for (const key of newCardKeys(explicitMotion ? moving.filter(row => reveals.entering.has(row.key)) : moving, knownCards.current, enabled)) {
      cardElements.current.get(key)?.classList.add('is-entering');
    }
    for (const item of items) knownCards.current.add(item.key);
  }, [items, layout, continueMotion, reducedMotion, explicitMotion, reveals, heldKeys]);

  const tipFor = useCallback((row: LogRow<T>) => {
    const forms = tooltip?.(row);
    return forms?.length ? fitTooltip(forms, viewportHeight) : null;
  }, [tooltip, viewportHeight]);
  const element = useCallback((id: string) => scroller?.querySelector<HTMLElement>(`[data-log-id="${CSS.escape(id)}"]`) ?? null, [scroller]);
  const clearTip = useCallback(() => { tip.current = null; tipKey.current = null; hideTooltip(owner); setSpoken(''); onTooltip?.(null); }, [owner, onTooltip]);
  const showTip = useCallback((next: LogTip) => {
    if (next.by === 'pointer') setPending(null);
    const row = rowOf(next.id);
    const content = row && tipFor(row);
    if (!row || !content) { clearTip(); return; }
    tipKey.current = row.key;
    let { x, y } = next;
    if (next.by === 'keyboard') {
      const box = element(next.id)?.getBoundingClientRect();
      if (box) { x = box.right - 24; y = box.top + box.height / 2; }
      const forms = tooltip?.(row);
      setSpoken(forms?.length ? tooltipText(forms[0]) : describe(row));
    }
    onTooltip?.(row);
    tip.current = { ...next, x, y };
    showTooltip(owner, content, x, y, `${next.id}:${row.key}:${slotsById.size}`);
  }, [rowOf, slotsById, tipFor, clearTip, element, tooltip, describe, owner, onTooltip]);
  showTipNow.current = showTip;
  rowNow.current = rowOf;

  useEffect(() => {
    const at = tip.current;
    if (at && tooltipStore.get()?.owner === owner) showTip(at);
  }, [showTip, owner]);
  useEffect(() => () => hideTooltip(owner), [owner]);
  useEffect(() => () => onTooltip?.(null), [onTooltip]);

  const reveal = useCallback((id: string) => {
    const index = slotOf(id);
    if (index == null || !scroller) return;
    setLogicalTop(scrollToShow(latest.current.offsets, index, scrollPosition.current.logical, scroller.clientHeight, sticky));
  }, [slotOf, scroller, sticky, setLogicalTop]);
  const openMenu = (id: string, x: number, y: number) => {
    const row = rowOf(id);
    const entries = row && menu?.(row);
    if (entries?.length) actions.open(x, y, entries);
  };
  const focusRow = useCallback((id: string) => {
    setPending(null);
    setActive(id);
    reveal(id);
    if (layout === 'card' && scroller?.contains(document.activeElement) && document.activeElement !== scroller) scroller.focus({ preventScroll: true });
    tip.current = { id, by: 'keyboard', x: 0, y: 0 };
    requestAnimationFrame(() => { if (tip.current?.id === id && tip.current.by === 'keyboard') showTip(tip.current); });
  }, [reveal, showTip, layout, scroller]);

  const focusTarget = (target: LogRowTarget<T>) => {
    if (target.row) { focusRow(rowId(target.row)); return; }
    if (target.pageKey == null) return;
    if (layout === 'card' && scroller?.contains(document.activeElement) && document.activeElement !== scroller) scroller.focus({ preventScroll: true });
    setActive(null);
    clearTip();
    setPending({ pageKey: target.pageKey, offset: target.offset });
    const error = pagesByKey.get(target.pageKey)?.error;
    setSpoken(error ? failureText(error, 'Couldn’t load rows.') : 'Loading rows.');
    if (!scroller) return;
    const at = latest.current.offsets[target.index] + target.offset * ROW_HEIGHT[density];
    setLogicalTop(scrollToShow([at, at + ROW_HEIGHT[density]], 0, scrollPosition.current.logical, scroller.clientHeight, sticky));
  };

  useEffect(() => {
    if (!pending) return;
    const keys = keyRows(), target = logPageTarget(items, keys.ordinals, pending.pageKey, pending.offset, keys.shown);
    if (target?.row) focusRow(rowId(target.row));
    else if (!target) { setPending(null); setSpoken('No rows match in this window.'); }
    else {
      const error = pagesByKey.get(pending.pageKey)?.error;
      setSpoken(error ? failureText(error, 'Couldn’t load rows.') : 'Loading rows.');
    }
  }, [pending, items, keyRows, pagesByKey, focusRow]);

  const current = active && slotsById.has(active) ? active : null;
  const onKeyDown = (event: React.KeyboardEvent) => {
    const keys = keyRows();
    const place = current ? placeOf(current) : undefined;
    const placed = place != null && keys.shown(place);
    const index = place != null ? keys.ordinals[place] - (placed || (event.key !== 'ArrowDown' && event.key !== 'PageDown') ? 0 : 1)
      : pending ? logPageTarget(items, keys.ordinals, pending.pageKey, pending.offset, keys.shown)?.ordinal ?? -1 : -1;
    if (event.key === 'Escape') { setPending(null); clearTip(); return; }
    if ((event.key === 'Enter' || event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) && current && placed) {
      event.preventDefault();
      const box = element(current)?.getBoundingClientRect();
      if (box) openMenu(current, box.left + 72, box.bottom);
      return;
    }
    const measured = latest.current.offsets, height = viewport.current.height;
    const page = Math.max(1, Math.floor(height / ROW_HEIGHT[density]) - 1);
    let target = keyTarget(event.key, Math.max(0, index), keys.ordinals[items.length], page);
    if (allPages && (layout === 'card' || rowHeight) && (event.key === 'PageDown' || event.key === 'PageUp')) {
      event.preventDefault();
      const destination = logPixelTarget(items, measured, keys.ordinals, Math.max(0, index), height, event.key === 'PageDown' ? 1 : -1);
      if (destination) focusTarget(destination);
      return;
    }
    if (!allPages && layout === 'card' && (event.key === 'PageDown' || event.key === 'PageUp')) {
      target = cardPageTarget(measured, rowSlots.filter(keys.shown), Math.max(0, index), height, event.key === 'PageDown' ? 1 : -1);
    }
    if (index < 0 && (event.key === 'ArrowDown' || event.key === 'ArrowUp')) target = 0;
    if (target == null) return;
    event.preventDefault();
    const destination = logRowTarget(items, keys.ordinals, target);
    if (destination) focusTarget(destination);
  };

  const entryHistory = useRef<{ seen: ReadonlySet<string>; entering: ReadonlySet<string> }>({ seen: new Set(), entering: new Set() });
  const visibleEntries = useMemo(() => {
    const arriving = new Set(moving.filter(row => row.phase === 'entering' && reveals.playhead === playhead && reveals.entering.has(row.key)).map(row => row.key));
    return logVisibleEntries(items, start, end, arriving, entryHistory.current.seen, entryHistory.current.entering, continueMotion);
  }, [items, start, end, reveals, continueMotion]);
  useLayoutEffect(() => { entryHistory.current = visibleEntries; }, [visibleEntries]);
  const drawn = useMemo(() => items.slice(start, end).flatMap(item => item.kind === 'row' ? [item.row] : []), [items, start, end]);
  const told = useRef<{ to: unknown; rows: readonly LogRow<T>[] } | null>(null);
  useEffect(() => {
    if (!onVisible) return;
    const before = told.current;
    if (before?.to === onVisible && before.rows.length === drawn.length && before.rows.every((row, index) => row === drawn[index])) return;
    told.current = { to: onVisible, rows: drawn };
    inFrame(() => onVisible(drawn));
  }, [drawn, onVisible]);
  const heldIndex = current == null ? undefined : slotsById.get(current)?.at(-1);
  const heldItem = heldIndex == null ? undefined : items[heldIndex];
  const heldPage = heldItem && heldItem.kind !== 'page' ? heldItem.pageKey : undefined;
  const wanted = useMemo(() => wantedLogPages(items, start, end, [heldPage, pending?.pageKey].filter((key): key is string => key != null)), [items, start, end, heldPage, pending?.pageKey]);
  const asked = useRef<{ to: (keys: readonly string[]) => void; keys: string } | null>(null);
  useEffect(() => {
    if (!onWantedPages) return;
    const keys = wanted.join('\n');
    if (asked.current?.to === onWantedPages && asked.current.keys === keys) return;
    asked.current = { to: onWantedPages, keys };
    inFrame(() => onWantedPages(wanted));
  }, [onWantedPages, wanted]);
  useEffect(() => () => asked.current?.to([]), []);
  const placed = shownMotion(moving, start, end, animate && (!explicitMotion || continueMotion), moved => moved.item.kind === 'row' && rowId(moved.item.row) === current, layout)
    .filter(moved => moved.phase !== 'leaving' || (moved.item.kind !== 'page' && (moved.item.pageKey == null || pagesByKey.get(moved.item.pageKey)?.rows != null)));
  const shown = untakenRows(placed);
  const optionId = (id: string) => `${listId}-${encodeURIComponent(JSON.stringify(id))}`;
  const stateRow = (slot: number) => (items[slot] as Extract<LogItem<T>, { kind: 'row' }>).row;
  const drawnRows = new Set<string>();

  if (!sections && !ordinals[items.length]) return <div className="log-empty">{empty}</div>;
  return <div className={`log${allPages ? ' is-paged' : ''}${sections ? ' is-sectioned' : ''}${geometry.compressed ? ' is-compressed' : ''}${layout === 'card' ? ' is-card-log' : ''}${density === 'compact' ? ' is-compact' : ''}${animate ? '' : ' no-motion'}${layered ? ' has-entries' : ''}`}
    style={{ '--log-columns': typeof columns === 'string' ? columns : undefined } as React.CSSProperties}>
    <div className="log-view">
      {topMonth && <div className="log-month is-sticky" aria-hidden="true">{topMonth}</div>}
      <div ref={setScroller} className="log-scroll" role="listbox" aria-label={label} tabIndex={0}
        aria-activedescendant={current ? optionId(current) : undefined}
        onScroll={event => readScroll(event.currentTarget)}
        onWheel={readerScrolls} onPointerDown={readerScrolls} onPointerMove={event => { if (event.buttons) readerScrolls(); }}
        onTouchMove={readerScrolls} onKeyDownCapture={readerScrolls}
        onKeyDown={onKeyDown}
        onFocus={event => {
          if (event.target !== event.currentTarget || pending) return;
          const keys = keyRows(), first = items.find((item, slot) => item.kind === 'row' && keys.shown(slot));
          if (first?.kind === 'row') setActive(previous => {
            const at = previous == null ? undefined : placeOf(previous);
            return at != null && keys.shown(at) ? previous : rowId(first.row);
          });
          else if (allPages && items.some(item => item.kind === 'page')) setSpoken('Loading rows.');
        }}
        onFocusCapture={event => { if (event.target !== event.currentTarget && event.target instanceof Element && hasLogPartTip(event.target, event.currentTarget)) { setPending(null); clearTip(); } }}
        onBlur={() => { setPending(null); clearTip(); }}>
        { }
        {ahead && <div ref={emptyNote} className="log-empty is-over" hidden={offsets[items.length] > 0}>{empty}</div>}
        <div ref={body} className="log-body" style={{ height: geometry.extent }}>
          {shown.map(moved => {
            const item = moved.item;
            if (item.kind !== 'page') return null;
            const place = placement(laid, geometry, top, viewHeight, moved.slot, item.page.count);
            return <div key={moved.key} className="log-page" data-slot={moved.slot} data-page-rows={item.page.count} data-state={item.page.error ? 'error' : 'loading'}
              style={{ transform: place.transform, height: place.height }} aria-hidden="true">
              <span data-detail={item.page.error?.message} style={{ transform: place.label }}>{item.page.error ? failureText(item.page.error, 'Couldn’t load rows.') : 'Loading rows…'}</span>
            </div>;
          })}
          <div ref={glide} className="log-glide moving-row">{layered && <canvas ref={under} className="log-entries is-under" hidden aria-hidden="true" />}{pooled
            ? Array.from({ length: poolSize }, (_, index) => <div key={index} data-pool={index} data-reveal="ahead" inert role="option" aria-selected={false}
              className="log-row moving-row" onPointerMove={event => {
                const id = event.currentTarget.dataset.logId;
                if (id == null) return;
                if (event.target instanceof Element && hasLogPartTip(event.target, event.currentTarget)) { setPending(null); clearTip(); return; }
                showTip({ id, by: 'pointer', x: event.clientX, y: event.clientY });
              }}
              onPointerLeave={() => { if (tip.current?.by === 'pointer') clearTip(); }}
              onClick={event => { const id = event.currentTarget.dataset.logId; if (id != null) { setPending(null); setActive(id); } }}
              onContextMenu={event => { event.preventDefault(); const id = event.currentTarget.dataset.logId; if (id == null) return; setPending(null); setActive(id); openMenu(id, event.clientX, event.clientY); }} />)
            : shown.map(moved => {
            const item = moved.item;
            if (item.kind === 'page') return null;
            if (item.kind === 'month') {
              const motion = animate ? movingRowClass(layout === 'card' && moved.phase === 'entering' ? { ...moved, phase: 'staying' } : moved) : 'moving-row';
              if (item.part === 'heading') return <div key={moved.key} className={`log-month is-heading ${motion}`} data-slot={moved.slot} role="heading" aria-level={3}>{item.label}</div>;
              if (item.part === 'note') return <div key={moved.key} className={`log-section-note ${motion}`} data-slot={moved.slot}>{notes.get(item.section!)}</div>;
              return <div key={moved.key} className={`log-month ${motion}`} data-slot={moved.slot} aria-hidden="true">{item.label}</div>;
            }
            const row = item.row;
            const id = rowId(row);
            const leaving = moved.phase === 'leaving';
            let key = moved.key, slots: readonly number[] | undefined, slot = moved.slot, entering = visibleEntries.entering.has(moved.key);
            if (grouping) {
              const held = slotsById.get(id);
              if (leaving && held) return null;
              key = stateRowKey(id);
              if (drawnRows.has(key)) return null;
              drawnRows.add(key);
              if (held && ahead) {
                slots = held;
                slot = logStateAt(ahead.spans, held, ahead.hour).slot;
                const first = held.reduce((a, b) => ahead.spans.reveal[b] < ahead.spans.reveal[a] ? b : a);
                entering = visibleEntries.entering.has(items[first].key);
              }
            }
            const shownRow = slots ? stateRow(slot) : row;
            const motion = !animate ? 'moving-row' : explicitMotion && layout === 'row' && !leaving
              ? `moving-row${entering ? ' is-entering' : ''}`
              : movingRowClass(layout === 'card' && moved.phase === 'entering' ? { ...moved, phase: 'staying' } : moved);
            const content = (state: LogRow<T>) => {
              const date = logDate(state.hour);
              return <>{layout === 'row' && <span className="log-when"><span>{state.by && <small>by </small>}{date.day}</span>{date.year && <small>{date.year}</small>}</span>}
                {renderRow(state)}</>;
            };
            return <div key={key} ref={layout === 'card' ? node => attachCard(moved.key, node) : undefined}
              id={leaving ? undefined : optionId(id)} data-log-id={leaving ? undefined : id} data-log-key={shownRow.key} data-slot={slots ? undefined : moved.slot}
              data-states={slots?.join(' ')} role="option"
              aria-selected={!leaving && id === current} aria-hidden={leaving || undefined} aria-label={describe(shownRow)}
              className={`log-row${layout === 'card' ? ' log-card' : ''} ${motion}${!leaving && id === current ? ' is-active' : ''}`}
              style={{ ...(layout === 'row' && rowHeight ? { height: rowHeight(shownRow, density) } : {}),
                '--log-columns': columns == null ? undefined : logRowColumns(columns, shownRow) } as React.CSSProperties}
              onPointerMove={event => {
                if (leaving) return;
                if (event.target instanceof Element && hasLogPartTip(event.target, event.currentTarget)) { setPending(null); clearTip(); return; }
                showTip({ id, by: 'pointer', x: event.clientX, y: event.clientY });
              }}
              onPointerLeave={() => { if (tip.current?.by === 'pointer') clearTip(); }}
              onClick={() => { if (!leaving) { setPending(null); setActive(id); } }}
              onContextMenu={event => { event.preventDefault(); if (leaving) return; setPending(null); setActive(id); openMenu(id, event.clientX, event.clientY); }}>
              {slots ? slots.map(at => {
                const state = stateRow(at);
                return <div key={state.key} className="log-state" hidden={at !== slot} data-key={state.key} data-label={describe(state)}
                  data-columns={typeof columns === 'function' ? columns(state) : undefined} data-height={layout === 'row' && rowHeight ? rowHeight(state, density) : undefined}>
                  {content(state)}</div>;
              }) : content(row)}
            </div>;
          })}{layered && <canvas ref={over} className="log-entries is-over" hidden aria-hidden="true" />}</div>
        </div>
      </div>
    </div>
    <span className="sr-only" aria-live="polite" data-detail={pending ? pagesByKey.get(pending.pageKey)?.error?.message : undefined}>{spoken}</span>
    {actions.menu}
  </div>;
}
