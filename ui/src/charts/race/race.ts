/** Ranked bars that race (mock-ups style-1 and style-2, "A · Clean"): one row per
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
const itemExtent = (item: RaceItem) => withInset(barExtent(itemWidths(item)), item.inset?.value);
/** Every amount actually drawn, including an inner measure exceeding the total. */
export const rowExtent = (row: Pick<RaceRow, 'item' | 'widths' | 'inset'>) => withInset(barExtent(drawnWidths(row)), row.inset);

/** The length a bar draws: the values its segments cover. */
export const barLength = (item: RaceItem) => { const extent = itemExtent(item); return extent.high - extent.low; };

/** The value axis' end for these entries: a little past the furthest bar, so its
 * label fits; at least 1. */
export function raceMax(items: readonly RaceItem[]): number {
  return Math.max(1, ...items.map(item => itemExtent(item).high)) * AXIS_ROOM;
}

/** The value axis' start: zero, or a little past the bar reaching furthest below it. */
export function raceMin(items: readonly RaceItem[]): number {
  return Math.min(0, ...items.map(item => itemExtent(item).low)) * AXIS_ROOM;
}

const targetWidths = (item: RaceItem) => new Map(item.segments.map(segment =>
  [segment.id, { solid: segment.value, hatched: Math.max(0, segment.hatched ?? 0) }]));

/** A row's drawn parts in its segments' order. */
export const drawnWidths = (row: Pick<RaceRow, 'item' | 'widths'>): PartWidth[] =>
  row.item.segments.map(segment => row.widths.get(segment.id) ?? { solid: 0, hatched: 0 });

/** The state after `dt` milliseconds at `now`, showing `items` (ranked, as drawn
 * from the top). `tau` is the easing time (0 snaps, as under reduced motion). A new
 * `items` array is a new moment: rows are re-ranked then (`nextRows`); the same
 * array only moves the animation on. The first state shows the entries in place. */
export function stepRace(state: RaceState | null, items: readonly RaceItem[], now: number, dt: number, tau: number, leaveMs = RACE_LEAVE_MS): RaceState {
  const entries = items.map(item => ({ key: item.key, item }));
  const first = state == null;
  const moving = !state ? nextRows([], entries, now, leaveMs, true)
    : state.items !== items ? nextRows(state.moving, entries, now, leaveMs)
      : state.moving.filter(row => row.phase !== 'leaving' || now - row.since < leaveMs);
  const before = new Map(state?.rows.map(row => [row.key, row]));
  // The slot just below the last row: a new row comes in from there.
  const below = items.length;
  const ease = (current: number, target: number) => approach(current, target, dt, tau);
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
    const goal = targetWidths(row.item);
    if (first || !previous) {
      // New rows appear at their values: nothing grows out of zero.
      return { key: row.key, item: row.item, phase: row.phase, since: row.since, target,
        y: first ? target : below, alpha: first ? 1 : ease(0, 1), widths: goal,
        ...(row.item.inset ? { inset: row.item.inset.value } : {}) };
    }
    // A bar glides only within one unbroken stretch; else it jumps. A segment it did
    // not have before appears at its value.
    const glide = previous.item.stretch != null && previous.item.stretch === row.item.stretch;
    const widths = leaving ? previous.widths : new Map([...goal].map(([id, width]) => {
      const drawn = previous.widths.get(id);
      return [id, glide && drawn ? { solid: ease(drawn.solid, width.solid), hatched: ease(drawn.hatched, width.hatched) } : width];
    }));
    const inset = leaving ? previous.inset : row.item.inset
      ? glide && previous.inset !== undefined ? ease(previous.inset, row.item.inset.value) : row.item.inset.value : undefined;
    return { key: row.key, item: leaving ? previous.item : row.item, phase: row.phase, since: row.since, target,
      y: ease(previous.y, target), alpha: ease(previous.alpha, leaving ? 0 : 1), widths,
      ...(inset !== undefined ? { inset } : {}) };
  });
  const goalMax = raceMax(items);
  const goalMin = raceMin(items);
  if (first) return { items, moving, rows, min: goalMin, max: goalMax };
  // The axis eases, but never leaves a bar as drawn outside it: whatever grew at
  // once (a new row, a jump across a gap or a change of country, a new segment) is
  // taken in at once, so no bar or value is cut off while the axis catches up.
  let max = ease(state!.max, goalMax);
  let min = ease(state!.min, goalMin);
  for (const row of rows) {
    if (row.phase === 'leaving') continue;
    const extent = rowExtent(row);
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
  top: number; bottom: number;
  /** One row's height, and a bar's thickness in it (at most 20 px, style A). */
  slot: number; thickness: number;
}

/** Room under the bars for the value axis' labels. */
export const RACE_AXIS = 22;
const TOP = 8;
/** Style A: bars are at most 20 px thick and take about half their row. */
export const MAX_THICKNESS = 20;
/** The least height of a row: its 14 px flag and 12 px name with room between rows,
 * so rows never overlap however many are asked for. */
export const MIN_SLOT = 20;
/** The line under the rows saying how many more there are than fit. */
export const MORE_LINE = 18;

/** How many of `wanted` rows fit a chart `height` px tall at no less than
 * [`MIN_SLOT`] each, and how many do not. When not all fit, a line under the rows
 * ([`MORE_LINE`]) says so. A race shows its leaders rather than scroll: it is read
 * at a glance while it plays, and a scroll position fighting the re-ranking would
 * hide the very rows that move. */
export function fitRows(height: number, wanted: number): { rows: number; hidden: number } {
  const room = Math.max(0, height - TOP - RACE_AXIS);
  const want = Math.max(1, Math.floor(wanted));
  if (want * MIN_SLOT <= room) return { rows: want, hidden: 0 };
  const rows = Math.max(1, Math.floor((room - MORE_LINE) / MIN_SLOT));
  return { rows, hidden: want - rows };
}

/** Where the "+N more" line's middle sits: in the room kept for it under the value
 * axis' labels (which take [`RACE_AXIS`] under the rows), so the two never overlap. */
export const moreLineY = (layout: RaceLayout) => layout.bottom + RACE_AXIS + MORE_LINE / 2;

/** The line under the rows when `hidden` entries do not fit. */
export const moreText = (hidden: number) => `+${hidden.toLocaleString('en-US')} more: make the widget taller or show fewer countries`;

/** The layout for `rows` rows: the label column (flag and name), the room kept at
 * the right for the value label at a bar's tip, and `reserve` px kept under the rows
 * (the "+N more" line). */
export function raceLayout(width: number, height: number, rows: number, labelWidth: number, valueWidth: number, reserve = 0): RaceLayout {
  const top = TOP;
  const bottom = Math.max(top + 1, height - RACE_AXIS - reserve);
  const slot = (bottom - top) / Math.max(1, rows);
  return { left: labelWidth, right: Math.max(labelWidth + 1, width - valueWidth), top, bottom, slot, thickness: Math.max(2, Math.min(MAX_THICKNESS, slot * 0.52)) };
}

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
 * slot's band covers it, nearest first; null over none. `min` and `max` are the
 * axis the bars are drawn on. */
export function rowAt(rows: readonly RaceRow[], layout: RaceLayout, point: { x: number; y: number }, min: number, max: number): RaceRow | null {
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
export function raceText(items: readonly RaceItem[], name: (key: string, item?: RaceItem) => string): string {
  return items.map((item, index) => `${index + 1}. ${name(item.key, item)} ${item.label}`).join('; ');
}
