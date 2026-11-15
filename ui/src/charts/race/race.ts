/** Ranked bars that race: one row per
 * entry, re-ranked whenever the values change; rows glide to their new place, a
 * new row slides in from below and fades in, a row that is gone slides away and
 * fades out. Bars may be split into segments (a factory type each), and each
 * segment may carry a hatched part (damaged factories). Pure: the chart keeps a
 * `RaceState` between frames and steps it with `stepRace`.
 *
 * Shapes ease and numbers stay exact: the value label is the recorded value the
 * widget gives, and a bar glides only within one unbroken stretch of its runs
 * (`stretch`, from `stretchAt`); across a gap or a change of country it jumps. */
import { approach } from '../easing.ts';
import { nextRows } from '../rowMotion.ts';
import type { MovingRow } from '../rowMotion';
import { compareTags } from '../../dashboard/tagOrder.ts';
import { stackEdges } from '../stack.ts';
import type { Edge } from '../stack';
import { linearTicks } from '../valueAxis.ts';

/** Pointer and keyboard activation share the same optional row callback. */
export function selectRaceRow(key: string | null, trigger: string, select?: (key: string) => void): boolean {
  if (key === null || !select || !['click', 'Enter', ' '].includes(trigger)) return false;
  select(key);
  return true;
}

/** One part of a bar: `value` drawn solid from where the part starts, then `hatched`
 * (a size) drawn hatched after it, away from zero. Values are signed: segments are
 * stacked as the time charts stack them (`stackEdges`), positive ones right of the
 * baseline and negative ones left of it, so a bar split into types covers the same
 * values as their sum and no recorded value is dropped. */
export interface RaceSegment { id: string; value: number; hatched?: number; color: string }

/** One entry of the race at the moment shown. */
export interface RaceItem {
  key: string;
  /** Country lookup stays separate from the stable animation key. */
  tag?: string; hour?: number;
  /** What the entry ranks by. */
  value: number;
  /** The exact value, as the label at the bar's tip shows it. */
  label: string;
  segments: readonly RaceSegment[];
  /** An inner secondary measure, independent of the stacked total. */
  inset?: { value: number; color: string };
  /** The unbroken stretch of recorded hours the value belongs to; the bar glides
   * from the value last drawn only when it is the same. Null never glides. */
  stretch: number | string | null;
}

/** One row as drawn: where it is (in slots from the top), how opaque, and each
 * segment's drawn solid and hatched lengths, in value units. */
export interface RaceRow {
  key: string;
  item: RaceItem;
  phase: MovingRow<RaceItem>['phase'];
  since: number;
  /** The slot it is going to: its rank, or, for a row that is gone, the slot just
   * below the last row when it went, fixed then, so it eases there however often
   * frames are drawn. */
  target: number;
  y: number;
  alpha: number;
  widths: ReadonlyMap<string, { solid: number; hatched: number }>;
  inset?: number;
  /** The widths `item` goes to, as `stepRace` made them: a row whose `widths` are these has arrived. */
  goal?: ReadonlyMap<string, PartWidth>;
}

export interface RaceState {
  items: readonly RaceItem[];
  moving: readonly MovingRow<RaceItem>[];
  rows: readonly RaceRow[];
  /** The value axis' ends, eased: `min` is zero unless a bar reaches below it. */
  min: number;
  max: number;
}

/** How long a row that is gone takes to leave. */
export const RACE_LEAVE_MS = 450;
const SETTLED = 1e-3;
/** The value axis ends this far past the longest bar. */
const AXIS_ROOM = 1.06;

/** The entries ranked highest first (ties by key, in the data service's tag order,
 * `compareTags`), at most `count`. Entries with
 * no finite value are left out, never ranked as zero. */
export function rankItems(items: readonly RaceItem[], count: number): RaceItem[] {
  return items.filter(item => Number.isFinite(item.value))
    .sort((a, b) => b.value - a.value || compareTags(a.key, b.key))
    .slice(0, Math.max(0, Math.floor(count)));
}

/** A drawn part: its signed solid value and the size of its hatched part. */
export interface PartWidth { solid: number; hatched: number }

/** A part's signed extent from where it starts: its solid part and its hatched
 * part after it, away from zero. */
export const partTotal = (width: PartWidth) => width.solid + (width.solid < 0 ? -width.hatched : width.hatched);

/** Where each part of a bar lies, in order (signed, stacked as `stackEdges` does). */
export const partEdges = (widths: readonly PartWidth[]): Edge[] => stackEdges(widths.map(partTotal));

/** The values a bar covers: its lowest and highest edge, zero included. */
export function barExtent(widths: readonly PartWidth[]): Edge {
  const edges = partEdges(widths);
  return { low: Math.min(0, ...edges.map(edge => edge.low)), high: Math.max(0, ...edges.map(edge => edge.high)) };
}

/** A drawn piece of a bar in values: a segment's solid or hatched part. */
export interface BarPart { segment: number; low: number; high: number; hatched: boolean }

/** A bar's pieces: each segment where it is stacked, its solid part next to where
 * it starts and its hatched part after it, away from zero. Empty pieces are left out. */
export function barParts(widths: readonly PartWidth[]): BarPart[] {
  const parts: BarPart[] = [];
  partEdges(widths).forEach((edge, segment) => {
    const { solid } = widths[segment];
    const pieces: [number, number, boolean][] = partTotal(widths[segment]) < 0
      ? [[edge.high + solid, edge.high, false], [edge.low, edge.high + solid, true]]
      : [[edge.low, edge.low + solid, false], [edge.low + solid, edge.high, true]];
    for (const [low, high, hatched] of pieces) if (high > low) parts.push({ segment, low, high, hatched });
  });
  return parts;
}

const itemWidths = (item: RaceItem): PartWidth[] => item.segments.map(segment => ({ solid: segment.value, hatched: Math.max(0, segment.hatched ?? 0) }));
const withInset = (extent: Edge, inset: number | undefined): Edge =>
  ({ low: Math.min(extent.low, inset ?? 0), high: Math.max(extent.high, inset ?? 0) });
function itemExtent(item: RaceItem): Edge {
  if (item.segments.length !== 1) return withInset(barExtent(itemWidths(item)), item.inset?.value);
  const width = item.segments[0]!, total = partTotal({ solid: width.value, hatched: Math.max(0, width.hatched ?? 0) });
  return withInset({ low: Math.min(0, total), high: Math.max(0, total) }, item.inset?.value);
}
/** Every amount actually drawn, including an inner measure exceeding the total. */
export const rowExtent = (row: Pick<RaceRow, 'item' | 'widths' | 'inset'>) => withInset(barExtent(drawnWidths(row)), row.inset);

/** The length a bar draws: the values its segments cover. */
export const barLength = (item: RaceItem) => { const extent = itemExtent(item); return extent.high - extent.low; };

/** The value axis' end for these entries: a little past the furthest bar, so its
 * label fits; at least 1. */
export function raceMax(items: readonly RaceItem[]): number {
  return axisOf(items).max;
}

/** The value axis' start: zero, or a little past the bar reaching furthest below it. */
export function raceMin(items: readonly RaceItem[]): number {
  return axisOf(items).min;
}

// A moment's axis ends and each item's goal widths are worked out once, not in every frame.
const axes = new WeakMap<readonly RaceItem[], { min: number; max: number }>();
function axisOf(items: readonly RaceItem[]) {
  let axis = axes.get(items);
  if (!axis) {
    let low = 0, high = 1;
    for (const item of items) { const extent = itemExtent(item); if (extent.low < low) low = extent.low; if (extent.high > high) high = extent.high; }
    axes.set(items, axis = { min: low * AXIS_ROOM, max: high * AXIS_ROOM });
  }
  return axis;
}
const targetWidths = (item: RaceItem): ReadonlyMap<string, PartWidth> =>
  new Map(item.segments.map(segment => [segment.id, { solid: segment.value, hatched: Math.max(0, segment.hatched ?? 0) }]));
/** Whether a row's bar is drawn at its item's values. */
const arrived = (row: RaceRow) => row.goal !== undefined && row.widths === row.goal && row.inset === row.item.inset?.value;
const rowsByKey = new WeakMap<RaceState, ReadonlyMap<string, RaceRow>>();
function rowsOf(state: RaceState): ReadonlyMap<string, RaceRow> {
  let rows = rowsByKey.get(state);
  if (!rows) rowsByKey.set(state, rows = new Map(state.rows.map(row => [row.key, row])));
  return rows;
}

/** A row's drawn parts in its segments' order. */
export const drawnWidths = (row: Pick<RaceRow, 'item' | 'widths'>): PartWidth[] =>
  row.item.segments.map(segment => row.widths.get(segment.id) ?? { solid: 0, hatched: 0 });

/** The slots a chart shows: rows outside them, now and where they go, land at once. */
export interface RaceView { from: number; to: number }

/** The state after `dt` milliseconds at `now`, showing `items` (ranked, as drawn
 * from the top). `tau` is the easing time (0 snaps, as under reduced motion). A new
 * `items` array is a new moment: rows are re-ranked then (`nextRows`); the same
 * array only moves the animation on. The first state shows the entries in place. */
export function stepRace(state: RaceState | null, items: readonly RaceItem[], now: number, dt: number, tau: number, leaveMs = RACE_LEAVE_MS,
  view?: RaceView): RaceState {
  const entries = items.map(item => ({ key: item.key, item }));
  const first = state == null;
  const moving = !state ? nextRows([], entries, now, leaveMs, true)
    : state.items !== items ? nextRows(state.moving, entries, now, leaveMs)
      : state.moving.filter(row => row.phase !== 'leaving' || now - row.since < leaveMs);
  const before = state ? rowsOf(state) : new Map<string, RaceRow>();
  // The slot just below the last row: a new row comes in from there.
  const below = items.length;
  // A value lands within what `raceMoving` calls settled, so a row at rest stays exactly at rest.
  const span = state ? Math.max(1e-9, state.max - state.min) : 1;
  const ease = (current: number, target: number) => approach(current, target, dt, tau, SETTLED);
  const easeAmount = (current: number, target: number) => approach(current, target, dt, tau, SETTLED * span);
  const rows: RaceRow[] = moving.map(row => {
    const previous = before.get(row.key);
    const leaving = row.phase === 'leaving';
    // A row that is gone goes to the slot just below the last row, itself included (it
    // still slides when it was the last), fixed when it went: where it is after a given
    // time does not depend on how many frames were drawn, and one that comes back
    // returns from near the chart.
    const target = !leaving ? row.slot
      : previous?.phase === 'leaving' ? previous.target
        : Math.max(below, (previous?.target ?? below - 1) + 1);
    const goal = previous?.goal && previous.item === row.item ? previous.goal : targetWidths(row.item);
    const unseen = view !== undefined && !leaving && previous !== undefined && (previous.y + 1 < view.from || previous.y > view.to)
      && (target + 1 < view.from || target > view.to);
    if (first || !previous || unseen) {
      if (unseen && previous.item === row.item && previous.phase === row.phase && previous.widths === goal && previous.y === target
        && previous.alpha === 1 && previous.inset === row.item.inset?.value) return previous;
      // New rows appear at their values: nothing grows out of zero.
      return { key: row.key, item: row.item, phase: row.phase, since: row.since, target,
        y: first || unseen ? target : below, alpha: first || unseen ? 1 : ease(0, 1), widths: goal, goal,
        ...(row.item.inset ? { inset: row.item.inset.value } : {}) };
    }
    // A row at rest stays the same object: nothing is worked out or allocated for it.
    if (!leaving && previous.item === row.item && previous.phase === row.phase && previous.widths === goal && previous.y === target
      && previous.alpha === 1 && previous.inset === row.item.inset?.value) return previous;
    // A bar glides only within one unbroken stretch; else it jumps. A segment it did
    // not have before appears at its value. Bars that reach their lengths take the goal's widths.
    const glide = previous.item.stretch != null && previous.item.stretch === row.item.stretch;
    let widths = previous.widths;
    if (!leaving && previous.widths !== goal) {
      let landed = true;
      const eased = new Map<string, PartWidth>();
      for (const [id, width] of goal) {
        const drawn = previous.widths.get(id);
        const next = glide && drawn ? { solid: easeAmount(drawn.solid, width.solid), hatched: easeAmount(drawn.hatched, width.hatched) } : width;
        landed &&= next.solid === width.solid && next.hatched === width.hatched;
        eased.set(id, next);
      }
      widths = landed ? goal : eased;
    }
    const inset = leaving ? previous.inset : row.item.inset
      ? glide && previous.inset !== undefined ? easeAmount(previous.inset, row.item.inset.value) : row.item.inset.value : undefined;
    return { key: row.key, item: leaving ? previous.item : row.item, phase: row.phase, since: row.since, target,
      y: ease(previous.y, target), alpha: ease(previous.alpha, leaving ? 0 : 1), widths, goal: leaving ? previous.goal : goal,
      ...(inset !== undefined ? { inset } : {}) };
  });
  const goalMax = raceMax(items);
  const goalMin = raceMin(items);
  if (first) return { items, moving, rows, min: goalMin, max: goalMax };
  // The axis eases, but never leaves a bar as drawn outside it: whatever grew at
  // once (a new row, a jump across a gap or a change of country, a new segment) is
  // taken in at once, so no bar or value is cut off while the axis catches up.
  let max = easeAmount(state!.max, goalMax);
  let min = easeAmount(state!.min, goalMin);
  for (const row of rows) {
    if (row.phase === 'leaving') continue;
    const extent = arrived(row) ? itemExtent(row.item) : rowExtent(row);
    max = Math.max(max, extent.high * AXIS_ROOM);
    min = Math.min(min, extent.low * AXIS_ROOM);
  }
  return { items, moving, rows, min, max };
}

/** Whether the state still moves: a row off its place, fading, a bar short of its
 * length, the axis easing or a row still leaving. */
export function raceMoving(state: RaceState): boolean {
  const span = state.max - state.min;
  if (Math.abs(state.max - raceMax(state.items)) > SETTLED * span || Math.abs(state.min - raceMin(state.items)) > SETTLED * span) return true;
  return state.rows.some(row => {
    if (row.phase === 'leaving' || Math.abs(row.y - row.target) > SETTLED || Math.abs(row.alpha - 1) > SETTLED) return true;
    if (arrived(row)) return false;
    if (row.item.inset && (row.inset === undefined || Math.abs(row.inset - row.item.inset.value) > SETTLED * span)) return true;
    return row.item.segments.some(segment => {
      const drawn = row.widths.get(segment.id);
      return !drawn || Math.abs(drawn.solid - segment.value) > SETTLED * span
        || Math.abs(drawn.hatched - Math.max(0, segment.hatched ?? 0)) > SETTLED * span;
    });
  });
}

/** Where a race draws, in layout pixels. */
export interface RaceLayout {
  /** The bars' baseline (after the label column) and the plot's right edge. */
  left: number; right: number;
  /** Where the first row starts and the last row ends, scrolled with the rows: past
   * `view` when the rows need more room than the chart has. */
  top: number; bottom: number;
  /** One row's height, and a bar's thickness in it (at most 20 px, style A). */
  slot: number; thickness: number;
  /** The plot as seen, which does not scroll: the grid and value axis are drawn on it,
   * and only rows inside it are drawn or hit. */
  view: { top: number; bottom: number };
}

/** Room under the bars for the value axis' labels. */
export const RACE_AXIS = 22;
const TOP = 8;
/** Style A: bars are at most 20 px thick and take about half their row. */
export const MAX_THICKNESS = 20;
/** A bar's thickness in a row `slot` px tall: about half the row, at most [`MAX_THICKNESS`]. */
export const barThickness = (slot: number) => Math.max(2, Math.min(MAX_THICKNESS, slot * 0.52));
/** The least height of a row: its 14 px flag and 12 px name with room between rows,
 * so rows never overlap however many are asked for. */
export const MIN_SLOT = 20;

/** The height `rows` rows need at their least height, with the room above them and
 * the value axis under them. A chart shorter than this scrolls its rows. */
export const raceHeight = (rows: number) => TOP + Math.max(1, Math.floor(rows)) * MIN_SLOT + RACE_AXIS;

/** The layout for `slots` rows in a chart `height` px tall, scrolled down `scroll` px:
 * the label column (flag and name), the room kept at the right for the value label at
 * a bar's tip, and the slots sharing the plot's height. Rows are never less than
 * [`MIN_SLOT`]: when the slots need more room they scroll under the fixed `view`.
 * The scroll ends at the last of the `rows` drawn, so slots kept for entries that are
 * not there leave no empty rows to scroll into; `scroll` is kept within them. */
export function raceLayout(width: number, height: number, slots: number, labelWidth: number, valueWidth: number, scroll = 0, rows = slots): RaceLayout {
  const count = Math.max(1, Math.floor(slots));
  const view = { top: TOP, bottom: Math.max(TOP + 1, height - RACE_AXIS) };
  const slot = Math.max(MIN_SLOT, (view.bottom - view.top) / count);
  const drawn = Math.max(1, Math.floor(rows));
  const top = view.top - Math.max(0, Math.min(scroll, TOP + drawn * slot + RACE_AXIS - height));
  return { left: labelWidth, right: Math.max(labelWidth + 1, width - valueWidth), top, bottom: top + slot * drawn, slot,
    thickness: barThickness(slot), view };
}

/** Whether a row centred at `centre` shows in the view at all. */
export const rowInView = (layout: RaceLayout, centre: number) =>
  centre + layout.slot / 2 >= layout.view.top && centre - layout.slot / 2 <= layout.view.bottom;

/** Whether a row is seen where it is or where it is gliding to, so one passing the
 * view's edge while re-ranking still counts. */
export const rowSeen = (layout: RaceLayout, row: Pick<RaceRow, 'y' | 'target'>) =>
  rowInView(layout, rowCentre(layout, row.y)) || rowInView(layout, rowCentre(layout, row.target));

/** A row's vertical centre. */
export const rowCentre = (layout: RaceLayout, y: number) => layout.top + y * layout.slot + layout.slot / 2;

/** The value axis' ticks: about one per 90 px of plot. */
export function raceTicks(layout: RaceLayout, min: number, max: number): number[] {
  return linearTicks(min, max, Math.max(2, Math.round((layout.right - layout.left) / 90)));
}

/** A value's x on the bars' scale, which runs from `min` (zero unless a bar reaches
 * below it) to `max`. */
export const valueX = (layout: RaceLayout, min: number, max: number, value: number) =>
  layout.left + ((value - min) / Math.max(1e-9, max - min)) * (layout.right - layout.left);

/** The order rows are drawn in: lower rows first, so a row moving up passes over
 * the others (stable, so rows at one place keep their order). Hit testing reads it
 * backwards, so what is under the pointer is what is drawn on top. */
export const paintOrder = (rows: readonly RaceRow[]): RaceRow[] => [...rows].sort((a, b) => b.y - a.y);

/** A row's bar as drawn: the rectangle from its lowest to its highest value, at its
 * place, on the axis from `min` to `max`. */
export function barRectangle(row: RaceRow, layout: RaceLayout, min: number, max: number): { left: number; right: number; top: number; bottom: number } {
  const extent = rowExtent(row);
  const centre = rowCentre(layout, row.y);
  return { left: valueX(layout, min, max, extent.low), right: valueX(layout, min, max, extent.high),
    top: centre - layout.thickness / 2, bottom: centre + layout.thickness / 2 };
}

/** The row under a point, among rows still in the race: the topmost drawn whose bar
 * rectangle holds the point (two bars overlap while they swap places, and a longer
 * one may show past the tip of a shorter one drawn over it), else the one whose
 * slot's band covers it, nearest first; null over none, and outside the view, where
 * rows scrolled away are not drawn. `min` and `max` are the axis the bars are drawn on. */
export function rowAt(rows: readonly RaceRow[], layout: RaceLayout, point: { x: number; y: number }, min: number, max: number): RaceRow | null {
  if (point.y < layout.view.top || point.y > layout.view.bottom) return null;
  const live = paintOrder(rows).filter(row => row.phase !== 'leaving').reverse();
  const onBar = live.find(row => {
    const bar = barRectangle(row, layout, min, max);
    return point.x >= bar.left && point.x <= bar.right && point.y >= bar.top && point.y <= bar.bottom;
  });
  if (onBar) return onBar;
  const distance = (row: RaceRow) => Math.abs(point.y - rowCentre(layout, row.y));
  let found: RaceRow | null = null;
  for (const row of live) if (distance(row) <= layout.slot / 2 && (!found || distance(row) < distance(found))) found = row;
  return found;
}

/** The entry a key moves the keyboard to, in rank order: Home and End the first and
 * last, the first press the first; an arrow the next or previous. A key no longer in
 * the race steps from the top. Null when there are none. */
export function keyedEntry(key: 'ArrowUp' | 'ArrowDown' | 'Home' | 'End', current: string | null, items: readonly RaceItem[]): string | null {
  if (!items.length) return null;
  if (key === 'Home') return items[0].key;
  if (key === 'End') return items.at(-1)!.key;
  const index = current == null ? -1 : items.findIndex(item => item.key === current);
  if (index < 0) return items[0].key;
  return items[key === 'ArrowDown' ? Math.min(items.length - 1, index + 1) : Math.max(0, index - 1)].key;
}

/** The ranking as one line for assistive technology: "1. Germany 45; 2. …". */
export function raceText(items: readonly RaceItem[], name: (key: string, item?: RaceItem) => string, offset = 0): string {
  return items.map((item, index) => `${offset + index + 1}. ${name(item.key, item)} ${item.label}`).join('; ');
}
