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
import { EntryLayer, backdrop, backgroundsBehind } from './entryLayer.ts';
import { useWhyCommit } from '../../dashboard/widgetPerf.ts';

export interface LogListProps<T> {
  /** The rows, newest first (`newestFirst`). The same array while unchanged. */
  rows?: readonly LogRow<T>[];
  /** Instead of eagerly loading every row, stable server windows with their
   * counts. Loaded pages replace their placeholder with actual filtered rows. */
  pages?: readonly LogPage<T>[];
  /** Instead of `rows` or `pages`, headed groups one after another in the one scroll, each of rows or of pages. A
   * section's `note` shows under its heading while none of its rows does. */
  sections?: readonly LogListSection<T>[];
  /** Hold only the windows in view, with a row-height margin, and any pending
   * keyboard destination. The product hook owns loading, errors and retries. */
  onWantedPages?: (keys: readonly string[]) => void;
  /** What the list is, for assistive technology. */
  label: string;
  density: LogDensity;
  /** A divider before each month's first row. */
  months: boolean;
  /** New rows flash in, moved rows glide and gone rows fade out; off, rows simply
   * appear, move and go. */
  animate: boolean;
  /** When supplied, entry motion means a row first revealed during forward
   * playback. Null shows the whole campaign without live arrival motion. */
  playhead?: number | null;
  /** Cards own their entire contents, including dates; rows keep the date column. */
  layout?: 'row' | 'card';
  /** A card's estimated height in layout pixels until its drawn contents are
   * measured. Row layouts can also supply their exact height. */
  rowHeight?: (row: LogRow<T>, density: LogDensity) => number;
  /** The row's cells after its date, or the whole card. */
  renderRow: (row: LogRow<T>) => ReactNode;
  /** The grid columns after the date; per-row tracks avoid reserving space for
   * optional content such as a live meter on a completed row. */
  columns?: LogColumns<T>;
  /** The row as one sentence, for assistive technology. */
  describe: (row: LogRow<T>) => string;
  /** The row's tooltip forms, fullest first (`fitTooltip` shows the fullest that fits). */
  tooltip?: (row: LogRow<T>) => readonly TooltipContent[];
  /** Demand a detail only for the tooltip currently reached by pointer or keys. */
  onTooltip?: (row: LogRow<T> | null) => void;
  /** The row's right-click menu, such as Jump to. */
  menu?: (row: LogRow<T>) => MenuItem[];
  /** Shown when there are no rows. */
  empty?: ReactNode;
  /** The rows drawn now (in view, with a margin), for a widget that names only
   * those, such as countries and flags as recorded at each row's hour. */
  onVisible?: (rows: readonly LogRow<T>[]) => void;
  /** Preserve the first visible row while recording windows are released. */
  anchorRows?: boolean;
  /** Following, the clock the rows are revealed by: rows past its hour (`revealedHour`, else `hour`) are drawn ahead,
   * hidden and taking no height, and each frame reveals those it reaches with the entry motion (and fades out those
   * whose `leftHour` it reaches), so rows arriving cost no render. Its hour is the list's playhead. */
  reveal?: RevealClock;
  /** Held ahead (`reveal`, rows), rows sharing an id are one row's states over time (`LogStates`): one row is drawn,
   * and each frame shows the state its clock reaches in place, gliding where its place changes. Its tooltip, menu and
   * keyboard follow the row, always at that state. */
  states?: boolean;
  /** Following (held ahead, a row log without month dividers or row heights of its own), each row the view shows is
   * drawn by the frame loop into one of a fixed set of row boxes React renders once: `bindRow` makes the row's cells as
   * `renderRow` draws them (its numbers then painted as before), so rows revealed while playing mount nothing. A new
   * `bindRow` binds every drawn row again. */
  bindRow?: (row: LogRow<T>) => Node;
}

export interface LogListSection<T> extends LogSection<T> { readonly note: ReactNode }

const EMPTY_ROWS: readonly LogRow<never>[] = [];
interface Viewport { height: number; width: number; outerHeight: number }
const measureViewport = (element: HTMLElement): Viewport => ({ height: element.clientHeight, width: element.clientWidth, outerHeight: element.offsetHeight });
/** How long after the reader's last wheel, key, drag or touch a scroll event is still theirs (a smooth scroll runs on). */
const READER_SCROLL_MS = 1000;
interface MonthStart { top: number; label: string | null }
/** The layout last committed, or measured since: what the painter, the scroll and the keys place by between renders. */
interface Laid<T> { items: readonly LogItem<T>[]; offsets: readonly number[]; monthStarts: readonly MonthStart[]; density: LogDensity; paged: boolean; months: boolean; card: boolean;
  /** Held ahead: every item's offsets at full height, when each item shows, and the hour (and its `logCut`) `offsets` are of. */
  ahead: { full: readonly number[]; spans: LogSpans; hour: number; cut: number } | null }
/** What a commit drew: its window of items, the month held at the top, and the width its cards were measured at. */
interface Drawn { start: number; end: number; month: string | null; width: number }

const monthStartsOf = <T,>(items: readonly LogItem<T>[], offsets: readonly number[]): MonthStart[] => items.flatMap<MonthStart>((item, index) =>
  item.kind === 'month' ? item.part === 'note' ? [] : [{ top: offsets[index], label: item.label }] : item.kind === 'page' ? [{ top: offsets[index], label: null }] : []);

/** The month of the rows at `top`, held above them once its own divider has scrolled away. */
function monthAt(starts: readonly MonthStart[], months: boolean, top: number): string | null {
  if (!months || top <= 0) return null;
  let low = 0, high = starts.length - 1, found = -1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (starts[middle].top <= top) { found = middle; low = middle + 1; } else high = middle - 1;
  }
  return found < 0 ? null : starts[found].label;
}

/** The items in view at `top` in a viewport `outerHeight` tall as laid now, with `overscan` items (rows, paged) each way. */
const viewAt = <T,>(laid: Laid<T>, top: number, outerHeight: number, overscan: number): [number, number] =>
  laid.paged ? logPageRange(laid.offsets, top, outerHeight, laid.density, overscan) : visibleRange(laid.offsets, top, outerHeight, overscan);

/** The items a render draws at `top`: held ahead, every item the view can reach before the clock leaves the block
 * (`logHeldRange`), so rows revealed, replaced or leaving never need another render; else those in view. */
const windowAt = <T,>(laid: Laid<T>, top: number, outerHeight: number, anchored: boolean, overscan = 6): [number, number] => laid.ahead
  ? logHeldRange(laid.offsets, laid.ahead.full, laid.ahead.spans, laid.ahead.hour, top, outerHeight, overscan, anchored)
  : viewAt(laid, top, outerHeight, overscan);

/** The held-ahead layout of `items` at `hour`: the offsets with the items not shown closed up. */
function aheadAt<T>(full: readonly number[], spans: LogSpans | null, hour: number | null): { offsets: readonly number[]; ahead: Laid<T>['ahead'] } {
  if (!spans || hour === null) return { offsets: full, ahead: null };
  return { offsets: logShownOffsets(full, spans, hour), ahead: { full, spans, hour, cut: logCut(spans, hour) } };
}

/** The logical top of the item at `slot` as laid; past the items, a row height each. */
function slotTop<T>(laid: Laid<T>, slot: number): number {
  const count = laid.items.length;
  return slot < count ? laid.offsets[slot] : laid.offsets[count] + (slot - count) * ROW_HEIGHT[laid.density];
}

/** Where the window of `pageRows` unloaded rows at `slot` is drawn at logical `top` in a viewport `height` tall: the part
 * of it in or near view, and where its label sits in that part. */
function placement<T>(laid: Laid<T>, space: ScrollGeometry, top: number, height: number, slot: number, pageRows: number): { transform: string; height: number; label: string } {
  const row = ROW_HEIGHT[laid.density], at = slotTop(laid, slot);
  const margin = row * 6;
  const from = Math.max(at, top - margin);
  const shown = Math.max(0, Math.min(at + pageRows * row, top + height + margin) - from);
  return { transform: `translateY(${scrollItemTop(space, from, top)}px)`, height: shown,
    label: `translateY(${Math.max(0, Math.min(Math.max(0, shown - row), top - from))}px)` };
}

/** The React key of a row drawn with its states (`states`): its id, apart from every item key. */
const stateRowKey = (id: string) => JSON.stringify(['states', id]);

/** Shows the `index`th of a row's states (its `.log-state` parts) and gives the row that state's label, key, columns and
 * height. */
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

/** Rows the view can reach beyond its edges, each way, in a pool of row boxes. */
const POOL_OVERSCAN = 3;

/** What the frame loop binds pooled row boxes with, as the last commit left it. */
interface Pool<T> {
  bind: (row: LogRow<T>) => Node;
  describe: (row: LogRow<T>) => string;
  columns: LogColumns<T> | undefined;
  optionId: (id: string) => string;
  current: string | null;
  slotsById: ReadonlyMap<string, readonly number[]>;
  grouping: boolean;
  /** Rows bound now, by id. */
  bound: Map<string, HTMLElement>;
  /** Set when the view wanted more rows than there are boxes: the next commit draws more. */
  short: boolean;
}

/** What each pooled box shows: the row's id and state key, and the binding they were made with. */
const boundRows = new WeakMap<HTMLElement, { id: string; key: string; bind: unknown }>();

/** A row's date cell, as a drawn row's. */
function dateCell(row: LogRow<unknown>): HTMLElement {
  const date = logDate(row.hour), cell = document.createElement('span'), day = document.createElement('span');
  cell.className = 'log-when';
  if (row.by) { const by = document.createElement('small'); by.textContent = 'by '; day.append(by); }
  day.append(date.day);
  cell.append(day);
  if (date.year) { const year = document.createElement('small'); year.textContent = date.year; cell.append(year); }
  return cell;
}

/** Binds the rows the view shows at logical `top` into the pool's boxes (`[data-pool]` children of `glide`), each at its
 * state's slot for `paintItems` to place; a row newly bound waits hidden (`ahead`), so a frame playing forward enters it
 * as a revealed row. A row keeps its box while it stays in reach. Boxes with no row are hidden and take no part. */
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
    // A row fading out keeps the box it has; one never drawn is not drawn to fade.
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

/** Each placed element's transform as last written: an item's in its glide's pixels, a glide's its offset. None for an
 * item held ahead, which is placed afresh as it shows. */
const written = new WeakMap<HTMLElement, number>();
/** Two placements this close are one. */
const SAME = 1e-6;
/** Each drawn item's top in one paint (NaN: held ahead, not placed), kept between paints. */
let tops = new Float64Array(256);

/** The batch each element placed afresh waits in, and the batch still open to elements placed this frame. */
const stilled = new WeakMap<HTMLElement, HTMLElement[]>();
let stilling: HTMLElement[] | null = null;
/** An item placed afresh (just drawn, or shown after being held ahead with a transform long stale) takes its place at
 * once: its transitions are held off until a frame has drawn it there, never sliding it from where it was last written. */
function placeStill(element: HTMLElement) {
  element.style.transition = 'none';
  if (!stilling) {
    const batch: HTMLElement[] = stilling = [];
    // Re-enabled in the next frame's `after`, once the browser has drawn the frame that placed it.
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

/** Writes each drawn item's placement (`data-slot`, `data-page-rows`) straight to the page: what a scroll, a resize or a
 * measured card moves is painted, not rendered. Held ahead, each item's state too (`data-reveal`, never rendered, so a
 * render keeps a running entry): `ahead` hidden, `gone` faded out, `in` entering as `enter` allows, absent shown. A row
 * drawn with its states (`data-states`) is placed at, and shows, the state the hour reaches.
 *
 * Rows, dividers and notes sit in `glide`, which carries the rows' own slide: it moves by the shift most of them share
 * (a row revealed above them pushes every one down by its height), and only an item moving otherwise is written. So a
 * frame that reveals rows restyles them and the glide, never every row drawn, and each row still slides as its own
 * transition would have, since the glide's and a row's transitions ease alike and start in the same frame. A row entering
 * is handed to `onEnter` with its height, where the list draws its entry in one shared layer (`EntryLayer`). */
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
    // The shift most items share (a majority vote): the glide takes it, and they need no write.
    const moved = at - base - was;
    if (votes === 0) { shift = moved; votes = 1; } else if (Math.abs(moved - shift) < SAME) votes++; else votes--;
  }
  // Kept within the list's height, so the glide never reaches past the scroll; beyond it, every item is written afresh.
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

/** A log: HTML rows newest first, each with its date,
 * that flash in as playback reaches them and glide when their order changes
 * (`useRowMotion`). Only the rows in view are drawn. Hovering a row, or reaching it
 * with the arrow keys, shows its tooltip fitted to the window, read out for the
 * keyboard. Whoever acted last owns the tooltip (`LogTip`, as a chart's hover passes
 * between pointer and keys), and it follows what its row is about (`rowId`) across
 * the row's keys, kept current or hidden as playback changes the rows. Right-click,
 * the context-menu key or Enter opens the row's menu.
 *
 * It renders when its rows change, and when a scroll or a resize needs other items drawn. The viewport, the scroll,
 * measured cards, anchoring and a faded row's leaving are painted, so a render while playing is its only commit. */
export default function LogList<T>({ rows = EMPTY_ROWS, pages, sections, onWantedPages, label, density, months, animate, playhead: givenPlayhead, layout = 'row', rowHeight, renderRow, columns, describe, tooltip, onTooltip, menu, empty, onVisible, anchorRows = false, reveal: revealBy, states = false, bindRow }: LogListProps<T>) {
  // Held ahead, the list's playhead is the reveal clock's hour: what the frame loop revealed is not an arrival again.
  const revealHour = revealBy && typeof givenPlayhead === 'number' ? revealBy.hour() : null;
  const playhead = revealHour ?? givenPlayhead;
  const owner = useId();
  const listId = useId();
  const viewportHeight = useViewportHeight();
  const tokens = useChartTokens();
  const { font, fontSize, reducedMotion } = tokens;
  const cardFont = JSON.stringify([font, fontSize.small, fontSize.body, fontSize.title]);
  // A callback ref: the scroller comes and goes with the rows (an empty log has
  // none), and each one is measured and observed as it attaches.
  const [scroller, setScroller] = useState<HTMLDivElement | null>(null);
  const body = useRef<HTMLDivElement | null>(null);
  // Holds the rows, dividers and notes, and carries the slide they share (`paintItems`).
  const glide = useRef<HTMLDivElement | null>(null);
  // Rows revealed in a row log enter through one shared layer, never an animation on each row (`EntryLayer`).
  const layered = layout === 'row' && animate === true && !reducedMotion && !!revealBy;
  const under = useRef<HTMLCanvasElement | null>(null);
  const over = useRef<HTMLCanvasElement | null>(null);
  const entries = useRef<{ layer: EntryLayer; task: FrameTask } | null>(null);
  const pool = useRef<Pool<T> | null>(null);
  // Boxes the pool draws beyond what its view holds, after it ran short.
  const [poolExtra, setPoolExtra] = useState(0);
  // The viewport in the list's own layout pixels (the widget's Size zoom included).
  const viewport = useRef<Viewport>({ height: 0, width: 0, outerHeight: 0 });
  // Keep exact logical pixels for wheel and keyboard movement even when the
  // browser rounds a much smaller native scrollTop to its layout precision.
  const scrollPosition = useRef({ logical: 0, physical: 0 });
  /** The scroll this list last wrote, as asked (logical) and as written (physical). */
  const scrolledTo = useRef<{ logical: number; physical: number } | null>(null);
  // Set while a frame reveals rows playing forward: they enter with motion.
  const entering = useRef(false);
  const emptyNote = useRef<HTMLDivElement | null>(null);
  // Moved on only when other items must be drawn (`covers`).
  const [redraws, setRedraws] = useState(0);
  const redraw = useCallback(() => setRedraws(count => count + 1), []);
  const [active, setActive] = useState<string | null>(null);
  const [pending, setPending] = useState<{ pageKey: string; offset: number } | null>(null);
  const [spoken, setSpoken] = useState('');
  const tip = useRef<LogTip | null>(null);
  // The state the tooltip shows, and how the frame loop shows it again when its row's state changes.
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
  // A faded row leaves with the next render, never with a commit of its own.
  const moving = useRowMotion<LogItem<T>>(motionItems, true);
  const { width, outerHeight, height: viewHeight } = viewport.current;
  const heightOf = useCallback((heights: ReadonlyMap<string, number>) => (row: LogRow<T>) =>
    (layout === 'card' ? heights.get(row.key) : undefined) ?? rowHeight?.(row, density) ?? ROW_HEIGHT[density], [layout, rowHeight, density]);
  const measuredHeights = cardHeightsAt(measurements.current, width, density, cardFont);
  const fullOffsets = useMemo(() => logOffsets(items, density, heightOf(measuredHeights)), [items, density, heightOf, measuredHeights]);
  const spans = useMemo(() => holding ? logSpans(items) : null, [holding, items]);
  const cut = spans && revealHour !== null ? logCut(spans, revealHour) : null;
  // The hour matters only through the items it shows (`cut`): a render inside the same cut keeps the layout.
  const hourHeld = useRef(revealHour);
  hourHeld.current = revealHour;
  const { offsets, ahead } = useMemo(() => cut === null ? aheadAt<T>(fullOffsets, null, null) : aheadAt<T>(fullOffsets, spans, hourHeld.current), [fullOffsets, spans, cut]);
  const monthStarts = useMemo(() => monthStartsOf(items, offsets), [items, offsets]);
  const laid = useMemo<Laid<T>>(() => ({ items, offsets, monthStarts, density, paged: allPages !== undefined, months, card: layout === 'card', ahead }),
    [items, offsets, monthStarts, density, allPages, months, layout, ahead]);
  const latest = useRef<Laid<T>>(laid);
  const drawnView = useRef<Drawn | null>(null);
  const priorLayout = useRef<{ items: readonly LogItem<T>[]; offsets: readonly number[]; top: number } | null>(null);
  // Each row's items, in order: one, or with `states` every state of it held.
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
  /** Where the row `id` is now: drawn with its states, the state the frame loop last showed (none while no state of it
   * shows); else its item. */
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
  /** The rows as the keyboard steps over them now: held ahead, only the items the clock's hour shows as the frame loop
   * last laid them, with `ordinals` counting only those. */
  const keyRows = useCallback((): { ordinals: readonly number[]; shown: (slot: number) => boolean } => {
    const now = latest.current.ahead;
    if (!now) return { ordinals, shown: () => true };
    return { ordinals: logShownOrdinals(items, now.spans, now.hour), shown: (slot: number) => logItemState(now.spans, slot, now.hour) === 'shown' };
  }, [items, ordinals]);
  /** Where the row `id` is drawn now, shown or not: drawn with its states, at the state the frame loop last placed. */
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
  // Where this render draws: where the scroll is or, anchored, where the row that was at the top has gone.
  const previous = priorLayout.current;
  const top = Math.max(0, Math.min(geometry.logicalRange, anchorRows && previous && (previous.items !== items || ((layout === 'card' || ahead !== null) && previous.offsets !== offsets))
    ? anchoredScrollTop(previous.items, previous.offsets, items, offsets, previous.top) : scrollPosition.current.logical));
  const sticky = months ? MONTH_HEIGHT : 0;
  // A mounted overscan row can create a horizontal scrollbar. Window by the
  // outer box so that scrollbar cannot remove its own row and oscillate; scroll
  // bounds and keyboard movement still use the usable client height.
  const [start, end] = windowAt(laid, top, outerHeight, anchorRows);
  const topMonth = monthAt(monthStarts, months, top);

  /** Whether what was drawn still serves the viewport and the scroll as they are: cards of the same width, the same month
   * held at the top, and the items in view as laid now (held ahead, exactly those, which the held window always draws;
   * else with half the overscan beyond them). Where they are not drawn, a render that would draw the same items again
   * gains nothing, so it is never asked for. */
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
  /** Paints the drawn items at the scroll as it is. */
  const place = useCallback(() => {
    const now = latest.current, view = viewport.current;
    const layer = entries.current;
    const pooled = pool.current;
    if (pooled && glide.current) {
      bindPool(glide.current, now, pooled, scrollPosition.current.logical, view.outerHeight);
      if (pooled.short) { pooled.short = false; inFrame(() => setPoolExtra(extra => extra + POOL_OVERSCAN * 2)); }
    }
    if (body.current) paintItems(body.current, glide.current, now, scrollGeometry(now.offsets[now.items.length], view.height), scrollPosition.current.logical, view.height, entering.current,
      layer ? (row, height) => { layer.layer.add(row, height, performance.now()); scheduler.wake(layer.task); } : undefined);
    if (emptyNote.current && now.ahead) emptyNote.current.hidden = now.offsets[now.items.length] > 0;
  }, []);
  /** Paints the drawn items at the scroll as it is; draws others only when they no longer cover the view. */
  const paint = useCallback(() => {
    place();
    if (!covers()) redraw();
  }, [place, covers, redraw]);
  /** Moves the scroll to logical `requested` (clamped) and paints. The viewport is the one its resize observer last read
   * (read here only before the first). The scroll is written only when it moves, unless not `measured` (a commit or a
   * reader's move), and never read back: nothing here makes the page lay out before it is drawn. */
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
  // When the reader last moved the scroll (wheel, keys, pointer, touch): a scroll event with none just before is the
  // list's own write, already painted, and is not read back.
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
  // Each commit: what the pool binds rows with (none when the rows are rendered), before what it drew is painted.
  useLayoutEffect(() => {
    if (!pooled || !bindRow) { pool.current = null; return; }
    pool.current = { bind: bindRow, describe, columns, optionId, current, slotsById, grouping, bound: pool.current?.bound ?? new Map(), short: false };
  });
  // Each commit: what it drew becomes the layout painted and anchored from, and the scroll goes where it drew.
  useLayoutEffect(() => {
    latest.current = laid;
    drawnView.current = { start, end, month: topMonth, width };
    priorLayout.current = { items: laid.items, offsets: laid.offsets, top };
    setLogicalTop(top);
  }, [laid, start, end, topMonth, width, outerHeight, top, redraws, setLogicalTop]);
  // A row's place is only ever painted, never rendered: every commit places what it drew.
  useLayoutEffect(() => { place(); });
  // The entry layer lives while its canvases do: drawn each frame a row is entering, in the colours the page shows now.
  useLayoutEffect(() => {
    if (!layered || !under.current || !over.current || !scroller) return;
    const layer = new EntryLayer(under.current, over.current);
    const task: FrameTask = { frame: () => layer.paint(performance.now(), row => written.get(row), viewport.current.width) };
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

  // The viewport, measured as the scroller attaches and as it is resized.
  useLayoutEffect(() => {
    if (!scroller) return;
    viewport.current = measureViewport(scroller);
    setLogicalTop(scrollPosition.current.logical);
    // Read as the observer reports, when the page has just been laid out; painted within a frame's budget, and a render
    // it needs is not a commit of its own.
    let alive = true;
    const observer = new ResizeObserver(() => {
      viewport.current = measureViewport(scroller);
      inFrame(() => { if (alive) setLogicalTop(scrollPosition.current.logical); });
    });
    observer.observe(scroller);
    return () => { alive = false; observer.disconnect(); };
  }, [scroller, setLogicalTop]);
  // Held ahead, each frame: once the clock's hour shows other items, they are laid out again by paint alone (closed up
  // or opened, the anchor kept), revealed rows entering while playing forward; a render follows only when the view
  // then needs items not drawn (`covers`).
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
      // A tooltip on a row whose shown state changed shows that state (or goes with it).
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

  // Measure layout pixels, so a widget's Size zoom never changes its virtual
  // height. Only drawn cards need measuring; unseen cards use the estimate. A
  // measured card moves the cards after it, and the anchor, by paint alone.
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
      // Add the animation on its committed mount. The class is deliberately
      // outside React's row phase: virtualisation can remount an entering row.
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
  /** Shows the tooltip of the row `next` names for whoever owns it: beside the
   * keyboard's row (read out) or at the pointer. A row that is gone takes it away. */
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

  // The tooltip shown, the pointer's or the keyboard's, stays current as playback changes the rows.
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
  // A navigation key takes the tooltip from the pointer: the keyboard's row, shown
  // whole with its tooltip beside it (placed once it is drawn) and read out.
  const focusRow = useCallback((id: string) => {
    setPending(null);
    setActive(id);
    reveal(id);
    // Leave a focused part's tooltip before showing the whole card.
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
    // A keyboard row the clock has hidden sits between the shown rows: a step down reaches the next, a step up the one before.
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
    // The first arrow press reaches the newest row shown.
    if (index < 0 && (event.key === 'ArrowDown' || event.key === 'ArrowUp')) target = 0;
    if (target == null) return;
    event.preventDefault();
    const destination = logRowTarget(items, keys.ordinals, target);
    if (destination) focusTarget(destination);
  };

  const entryHistory = useRef<{ seen: ReadonlySet<string>; entering: ReadonlySet<string> }>({ seen: new Set(), entering: new Set() });
  const visibleEntries = useMemo(() => {
    // Preserve the initial-render rule of useRowMotion: already-present rows
    // do not arrive. A mounted animation then survives later staying phases.
    const arriving = new Set(moving.filter(row => row.phase === 'entering' && reveals.playhead === playhead && reveals.entering.has(row.key)).map(row => row.key));
    return logVisibleEntries(items, start, end, arriving, entryHistory.current.seen, entryHistory.current.entering, continueMotion);
  }, [items, start, end, reveals, continueMotion]);
  useLayoutEffect(() => { entryHistory.current = visibleEntries; }, [visibleEntries]);
  const drawn = useMemo(() => items.slice(start, end).flatMap(item => item.kind === 'row' ? [item.row] : []), [items, start, end]);
  // What the list tells its owner renders with the next frame (a tick while playing): a layout the
  // owner's answer changes again can step once a frame, never loop within one. The same rows again are
  // not news, so an owner holding them in state does not commit for them.
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
  // The owner's callback changes as its rows do (`useLogPages`' head page): the pages are asked of the new one, and
  // released only when the list goes, never between two of its renders.
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
          // Focus lands on a row the clock shows: the keyboard's row if it still shows, else the newest shown.
          const keys = keyRows(), first = items.find((item, slot) => item.kind === 'row' && keys.shown(slot));
          if (first?.kind === 'row') setActive(previous => {
            const at = previous == null ? undefined : placeOf(previous);
            return at != null && keys.shown(at) ? previous : rowId(first.row);
          });
          else if (allPages && items.some(item => item.kind === 'page')) setSpoken('Loading rows.');
        }}
        onFocusCapture={event => { if (event.target !== event.currentTarget && event.target instanceof Element && hasLogPartTip(event.target, event.currentTarget)) { setPending(null); clearTip(); } }}
        onBlur={() => { setPending(null); clearTip(); }}>
        {/* Placed as drawn and then painted (`paintItems`) as the scroll, the viewport and measured cards move. */}
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
            // A row fading out is no longer the row its id names: it neither answers nor is read.
            const leaving = moved.phase === 'leaving';
            let key = moved.key, slots: readonly number[] | undefined, slot = moved.slot, entering = visibleEntries.entering.has(moved.key);
            if (grouping) {
              // One row for all its states; a state the next block no longer holds leaves with nothing of its own.
              const held = slotsById.get(id);
              if (leaving && held) return null;
              key = stateRowKey(id);
              if (drawnRows.has(key)) return null;
              drawnRows.add(key);
              if (held && ahead) {
                slots = held;
                slot = logStateAt(ahead.spans, held, ahead.hour).slot;
                // It enters as its first state does, never as a later one.
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
