import { approach } from '../easing.ts';
import { nextRows } from '../rowMotion.ts';
import type { MovingRow } from '../rowMotion';
import { stackEdges } from '../stack.ts';
import type { Edge } from '../stack';
import { linearTicks } from '../valueAxis.ts';

export function selectRaceRow(key: string | null, trigger: string, select?: (key: string) => void): boolean {
  if (key === null || !select || !['click', 'Enter', ' '].includes(trigger)) return false;
  select(key);
  return true;
}

export interface RaceSegment { id: string; value: number; hatched?: number; color: string }

export interface RaceItem {
  key: string;
  tag?: string; hour?: number;
  value: number;
  label: string;
  segments: readonly RaceSegment[];
  inset?: { value: number; color: string };
  stretch: number | string | null;
}

export interface RaceRow {
  key: string;
  item: RaceItem;
  phase: MovingRow<RaceItem>['phase'];
  since: number;
  target: number;
  y: number;
  alpha: number;
  widths: ReadonlyMap<string, { solid: number; hatched: number }>;
  inset?: number;
  goal?: ReadonlyMap<string, PartWidth>;
}

export interface RaceState {
  items: readonly RaceItem[];
  moving: readonly MovingRow<RaceItem>[];
  rows: readonly RaceRow[];
  min: number;
  max: number;
}

export const RACE_LEAVE_MS = 450;
const SETTLED = 1e-3;
const AXIS_ROOM = 1.06;

export interface PartWidth { solid: number; hatched: number }

export const partTotal = (width: PartWidth) => width.solid + (width.solid < 0 ? -width.hatched : width.hatched);

export const partEdges = (widths: readonly PartWidth[]): Edge[] => stackEdges(widths.map(partTotal));

export function barExtent(widths: readonly PartWidth[]): Edge {
  const edges = partEdges(widths);
  return { low: Math.min(0, ...edges.map(edge => edge.low)), high: Math.max(0, ...edges.map(edge => edge.high)) };
}

export interface BarPart { segment: number; low: number; high: number; hatched: boolean }

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
export const rowExtent = (row: Pick<RaceRow, 'item' | 'widths' | 'inset'>) => withInset(barExtent(drawnWidths(row)), row.inset);

export function raceMax(items: readonly RaceItem[]): number {
  return axisOf(items).max;
}

export function raceMin(items: readonly RaceItem[]): number {
  return axisOf(items).min;
}

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
const arrived = (row: RaceRow) => row.goal !== undefined && row.widths === row.goal && row.inset === row.item.inset?.value;
const rowsByKey = new WeakMap<RaceState, ReadonlyMap<string, RaceRow>>();
function rowsOf(state: RaceState): ReadonlyMap<string, RaceRow> {
  let rows = rowsByKey.get(state);
  if (!rows) rowsByKey.set(state, rows = new Map(state.rows.map(row => [row.key, row])));
  return rows;
}

export const drawnWidths = (row: Pick<RaceRow, 'item' | 'widths'>): PartWidth[] =>
  row.item.segments.map(segment => row.widths.get(segment.id) ?? { solid: 0, hatched: 0 });

export interface RaceView { from: number; to: number }

export function stepRace(state: RaceState | null, items: readonly RaceItem[], now: number, dt: number, tau: number, leaveMs = RACE_LEAVE_MS,
  view?: RaceView): RaceState {
  const entries = items.map(item => ({ key: item.key, item }));
  const first = state == null;
  const moving = !state ? nextRows([], entries, now, leaveMs, true)
    : state.items !== items ? nextRows(state.moving, entries, now, leaveMs)
      : state.moving.filter(row => row.phase !== 'leaving' || now - row.since < leaveMs);
  const before = state ? rowsOf(state) : new Map<string, RaceRow>();
  const below = items.length;
  const span = state ? Math.max(1e-9, state.max - state.min) : 1;
  const ease = (current: number, target: number) => approach(current, target, dt, tau, SETTLED);
  const easeAmount = (current: number, target: number) => approach(current, target, dt, tau, SETTLED * span);
  const rows: RaceRow[] = moving.map(row => {
    const previous = before.get(row.key);
    const leaving = row.phase === 'leaving';
    const target = !leaving ? row.slot
      : previous?.phase === 'leaving' ? previous.target
        : Math.max(below, (previous?.target ?? below - 1) + 1);
    const goal = previous?.goal && previous.item === row.item ? previous.goal : targetWidths(row.item);
    const unseen = view !== undefined && !leaving && previous !== undefined && (previous.y + 1 < view.from || previous.y > view.to)
      && (target + 1 < view.from || target > view.to);
    if (first || !previous || unseen) {
      if (unseen && previous.item === row.item && previous.phase === row.phase && previous.widths === goal && previous.y === target
        && previous.alpha === 1 && previous.inset === row.item.inset?.value) return previous;
      return { key: row.key, item: row.item, phase: row.phase, since: row.since, target,
        y: first || unseen ? target : below, alpha: first || unseen ? 1 : ease(0, 1), widths: goal, goal,
        ...(row.item.inset ? { inset: row.item.inset.value } : {}) };
    }
    if (!leaving && previous.item === row.item && previous.phase === row.phase && previous.widths === goal && previous.y === target
      && previous.alpha === 1 && previous.inset === row.item.inset?.value) return previous;
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

export interface RaceLayout {
  left: number; right: number;
  top: number; bottom: number;
  slot: number; thickness: number;
  view: { top: number; bottom: number };
}

export const RACE_AXIS = 22;
const TOP = 8;
export const MAX_THICKNESS = 20;
export const barThickness = (slot: number) => Math.max(2, Math.min(MAX_THICKNESS, slot * 0.52));
export const MIN_SLOT = 20;

export const raceHeight = (rows: number) => TOP + Math.max(1, Math.floor(rows)) * MIN_SLOT + RACE_AXIS;

export function raceLayout(width: number, height: number, slots: number, labelWidth: number, valueWidth: number, scroll = 0, rows = slots): RaceLayout {
  const count = Math.max(1, Math.floor(slots));
  const view = { top: TOP, bottom: Math.max(TOP + 1, height - RACE_AXIS) };
  const slot = Math.max(MIN_SLOT, (view.bottom - view.top) / count);
  const drawn = Math.max(1, Math.floor(rows));
  const top = view.top - Math.max(0, Math.min(scroll, TOP + drawn * slot + RACE_AXIS - height));
  return { left: labelWidth, right: Math.max(labelWidth + 1, width - valueWidth), top, bottom: top + slot * drawn, slot,
    thickness: barThickness(slot), view };
}

export const rowInView = (layout: RaceLayout, centre: number) =>
  centre + layout.slot / 2 >= layout.view.top && centre - layout.slot / 2 <= layout.view.bottom;

export const rowSeen = (layout: RaceLayout, row: Pick<RaceRow, 'y' | 'target'>) =>
  rowInView(layout, rowCentre(layout, row.y)) || rowInView(layout, rowCentre(layout, row.target));

export const rowCentre = (layout: RaceLayout, y: number) => layout.top + y * layout.slot + layout.slot / 2;

export function raceTicks(layout: RaceLayout, min: number, max: number): number[] {
  return linearTicks(min, max, Math.max(2, Math.round((layout.right - layout.left) / 90)));
}

export const valueX = (layout: RaceLayout, min: number, max: number, value: number) =>
  layout.left + ((value - min) / Math.max(1e-9, max - min)) * (layout.right - layout.left);

export const paintOrder = (rows: readonly RaceRow[]): RaceRow[] => [...rows].sort((a, b) => b.y - a.y);

export function barRectangle(row: RaceRow, layout: RaceLayout, min: number, max: number): { left: number; right: number; top: number; bottom: number } {
  const extent = rowExtent(row);
  const centre = rowCentre(layout, row.y);
  return { left: valueX(layout, min, max, extent.low), right: valueX(layout, min, max, extent.high),
    top: centre - layout.thickness / 2, bottom: centre + layout.thickness / 2 };
}

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

export function keyedEntry(key: 'ArrowUp' | 'ArrowDown' | 'Home' | 'End', current: string | null, items: readonly RaceItem[]): string | null {
  if (!items.length) return null;
  if (key === 'Home') return items[0].key;
  if (key === 'End') return items.at(-1)!.key;
  const index = current == null ? -1 : items.findIndex(item => item.key === current);
  if (index < 0) return items[0].key;
  return items[key === 'ArrowDown' ? Math.min(items.length - 1, index + 1) : Math.max(0, index - 1)].key;
}

export function raceText(items: readonly RaceItem[], name: (key: string, item?: RaceItem) => string, offset = 0): string {
  return items.map((item, index) => `${offset + index + 1}. ${name(item.key, item)} ${item.label}`).join('; ');
}
