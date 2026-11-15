/** A rows-at-an-hour product's live cells (`crates/dashboard-follow/src/live.rs`): the numbers it shows that move with
 * the playhead, never in its reply. The reply's `live` names each cell by the row and column its structure shows it in,
 * and a small formula over recorded cells (a value, a sum, a difference or a ratio, times a scale), the cells grouped
 * as followed queries. A request's reply carries each group's values at its hour; a panel's names the follow id each
 * group is pushed under, its answers landing in a `CellGroup` the store holds. Either way the widget reads one
 * `Live<LiveValues<number>>` keyed `liveKey(row, column)`. */
import { liveKey, noValues } from '../../charts/live.ts';
import type { Live, LiveValues } from '../../charts/live';
import { CELL, cellNumber, cellState, cellValue, rowKey, rowReason } from './message.ts';
import type { FollowedAnswer } from './message';

/** The bit of every follow id a panel's host takes for a group of its cells. */
export const CELLS = 0x8000_0000;
export const isCellsId = (id: number) => id >= CELLS;

/** Why a cell is missing; the service's request path says the same. */
export const NOT_READ = 'Not read yet';
export const NO_VALUE = 'No value';
export const NOT_RECORDED = 'Not recorded at this hour';
export const ZERO_DIVISOR = 'Its divisor is zero at this hour';

export type LiveOp = 'value' | 'sum' | 'difference' | 'ratio';
export interface LiveKind { id: string; op: LiveOp; scale: number }
/** One followed query of the cells: these keys of a table, each read in these columns; its values at the asked hour,
 * the id it is followed under, or why it is not. */
export interface LiveGroupSpec { table: number; columns: number[]; keys: string[]; values?: (number | string)[]; follow?: number; unavailable?: string }
/** Per cell, flat: its row, its kind, its sources' count, then each source's group, row and column. */
export interface LiveSection { groups: LiveGroupSpec[]; rows: string[]; kinds: LiveKind[]; cells: number[] }

const OPS = new Set<LiveOp>(['value', 'sum', 'difference', 'ratio']);
const integer = (value: unknown): value is number => Number.isSafeInteger(value) && (value as number) >= 0;
const texts = (value: unknown): value is string[] => Array.isArray(value) && value.every(item => typeof item === 'string');

/** A reply's `live`, checked whole; null when the reply names no live cells. */
export function liveSection(reply: unknown): LiveSection | null {
  const live = reply && typeof reply === 'object' ? (reply as { live?: unknown }).live : undefined;
  if (live === undefined || live === null) return null;
  const section = live as LiveSection;
  if (typeof live !== 'object' || !Array.isArray(section.groups) || !texts(section.rows) || !Array.isArray(section.kinds) || !Array.isArray(section.cells))
    throw new Error('A reply\'s live cells are malformed.');
  for (const group of section.groups) {
    const size = Array.isArray(group?.keys) && Array.isArray(group.columns) ? group.keys.length * group.columns.length : -1;
    if (!group || !integer(group.table) || !group.columns.every(integer) || !texts(group.keys)
      || [group.values !== undefined, group.follow !== undefined, group.unavailable !== undefined].filter(Boolean).length !== 1
      || group.values !== undefined && (!Array.isArray(group.values) || group.values.length !== size || !group.values.every(value => typeof value === 'string' || Number.isFinite(value)))
      || group.follow !== undefined && !(integer(group.follow) && isCellsId(group.follow))
      || group.unavailable !== undefined && typeof group.unavailable !== 'string')
      throw new Error('A live cells group is malformed.');
  }
  if (!section.kinds.every(kind => kind && typeof kind.id === 'string' && OPS.has(kind.op) && Number.isFinite(kind.scale)))
    throw new Error('A live cell kind is malformed.');
  for (let at = 0; at < section.cells.length;) {
    const [row, kind, count] = [section.cells[at], section.cells[at + 1], section.cells[at + 2]];
    if (!integer(row) || row >= section.rows.length || !integer(kind) || kind >= section.kinds.length || !integer(count) || count < 1) throw new Error('A live cell is malformed.');
    for (let source = 0; source < count; source++) {
      const [group, key, column] = section.cells.slice(at + 3 + source * 3, at + 6 + source * 3);
      const held = integer(group) ? section.groups[group] : undefined;
      if (!held || !integer(key) || key >= held.keys.length || !integer(column) || column >= held.columns.length) throw new Error('A live cell names a source no group holds.');
    }
    at += 3 + count * 3;
  }
  return section;
}

const sections = new WeakMap<object, LiveSection | null>();

/** [`liveSection`] of a reply, checked once per reply object. */
export function sectionOf(reply: unknown): LiveSection | null {
  if (!reply || typeof reply !== 'object') return null;
  if (!sections.has(reply)) sections.set(reply, liveSection(reply));
  return sections.get(reply)!;
}

/** Where a cell's source is read: a number, or undefined with `missing` saying why. */
export interface GroupValues {
  readonly version: number;
  get(row: number, column: number): number | undefined;
  missing(row: number, column: number): string;
}

/** A group whose every cell is missing for one reason. */
const missingGroup = (reason: string): GroupValues => ({ version: 0, get: () => undefined, missing: () => reason });

/** A group's values as a request's reply carries them, row by row. */
function valuedGroup(spec: LiveGroupSpec, values: readonly (number | string)[]): GroupValues {
  const width = spec.columns.length;
  return {
    version: 0,
    get: (row, column) => { const value = values[row * width + column]; return typeof value === 'number' ? value : undefined; },
    missing: (row, column) => { const value = values[row * width + column]; return typeof value === 'string' ? value : NOT_READ; },
  };
}

/** A followed group's cells as its answers left them: a whole answer replaces them, an answer of rows changed lays its
 * rows over them by key, "not recorded" makes every one missing for its reason. */
export class CellGroup implements GroupValues {
  version = 0;
  /** The hour its cells are current through: the newest message of its timeline; null before its first answer. */
  through: number | null = null;
  /** The follow its answers come from, kept for the panel that names it after its first answer. */
  route: number | null = null;
  private byKey = new Map<string, number>();
  private width = 0;
  private values = new Float64Array(0);
  private states = new Uint8Array(0);
  private reasons: (string | null)[] = [];
  private whole: string | null = NOT_READ;

  land(answer: FollowedAnswer): void {
    this.version++;
    if (answer.kind === 'notRecorded') { this.whole = answer.reason; return; }
    if (answer.kind === 'changed') {
      if (this.whole !== null) return;
      for (let row = 0; row < answer.rows; row++) {
        const at = this.byKey.get(String(rowKey(answer, row)));
        if (at !== undefined) this.copy(answer, row, at);
      }
      return;
    }
    this.whole = null;
    this.width = answer.columns;
    this.byKey = new Map();
    this.values = new Float64Array(answer.rows * answer.columns);
    this.states = new Uint8Array(answer.rows * answer.columns);
    this.reasons = new Array<string | null>(answer.rows).fill(null);
    for (let row = 0; row < answer.rows; row++) {
      this.byKey.set(String(rowKey(answer, row)), row);
      this.copy(answer, row, row);
    }
  }

  private copy(answer: Exclude<FollowedAnswer, { kind: 'notRecorded' }>, from: number, to: number): void {
    for (let column = 0; column < this.width; column++) {
      const cell = from * answer.columns + column, at = to * this.width + column;
      this.states[at] = cellState(answer, cell);
      this.values[at] = cellNumber(answer, cell) ?? Number(cellValue(answer, cell));
    }
    this.reasons[to] = rowReason(answer, from);
  }

  get(row: number, column: number): number | undefined {
    if (this.whole !== null || row >= this.reasons.length) return undefined;
    const at = row * this.width + column;
    return this.states[at] === CELL.value ? this.values[at] : undefined;
  }

  missing(row: number, column: number): string {
    if (this.whole !== null) return this.whole;
    if (row >= this.reasons.length) return NOT_READ;
    const state = this.states[row * this.width + column];
    return this.reasons[row] ?? (state === CELL.missing ? NO_VALUE : NOT_RECORDED);
  }
}

interface Index { byKey: Map<string, number> }
const indexes = new WeakMap<LiveSection, Index>();

/** Each cell's start in `cells` by its key. */
function indexOf(section: LiveSection): Index {
  let index = indexes.get(section);
  if (!index) {
    const byKey = new Map<string, number>();
    for (let at = 0; at < section.cells.length; at += 3 + section.cells[at + 2]! * 3)
      byKey.set(liveKey(section.rows[section.cells[at]!]!, section.kinds[section.cells[at + 1]!]!.id), at);
    indexes.set(section, index = { byKey });
  }
  return index;
}

/** The cells of `section` read from `groups` (one per section group, in order). */
export function sectionValues(section: LiveSection, groups: readonly GroupValues[]): LiveValues<number> {
  const { byKey } = indexOf(section);
  const { cells, kinds } = section;
  const operand = (at: number, source: number) => {
    const base = at + 3 + source * 3;
    return { group: groups[cells[base]!]!, row: cells[base + 1]!, column: cells[base + 2]! };
  };
  const evaluate = (at: number): number | string => {
    const kind = kinds[cells[at + 1]!]!, count = cells[at + 2]!;
    let first = 0, second = 0, sum = 0;
    for (let source = 0; source < count; source++) {
      const { group, row, column } = operand(at, source);
      const value = group.get(row, column);
      if (value === undefined) return group.missing(row, column);
      if (source === 0) first = value;
      if (source === 1) second = value;
      sum += value;
    }
    switch (kind.op) {
      case 'value': return first * kind.scale;
      case 'sum': return sum * kind.scale;
      case 'difference': return (first - second) * kind.scale;
      case 'ratio': return second === 0 ? ZERO_DIVISOR : first / second * kind.scale;
    }
  };
  return {
    get: key => { const at = byKey.get(key); if (at === undefined) return undefined; const value = evaluate(at); return typeof value === 'number' ? value : undefined; },
    missing: key => { const at = byKey.get(key); if (at === undefined) return 'Not a live cell of this reply'; const value = evaluate(at); return typeof value === 'string' ? value : ''; },
  };
}

/** The groups of `section`: its values, the store's group for each followed id (`group`), or missing for its reason. */
export function sectionGroups(section: LiveSection, group: (id: number) => GroupValues | undefined): GroupValues[] {
  return section.groups.map(spec => spec.values ? valuedGroup(spec, spec.values)
    : spec.follow !== undefined ? group(spec.follow) ?? missingGroup(NOT_READ)
    : missingGroup(spec.unavailable ?? NOT_READ));
}

const NONE = 'This reply names no live cells';
const STILL: Live<LiveValues<number>> = { read: () => noValues(NONE), subscribe: () => () => {} };

const still = (values: LiveValues<number>): Live<LiveValues<number>> => ({ read: () => values, subscribe: () => () => {} });

/** A request's reply's cells, which never move. */
export function repliedCells(reply: unknown): Live<LiveValues<number>> {
  try {
    const section = sectionOf(reply);
    return section ? still(sectionValues(section, sectionGroups(section, () => undefined))) : STILL;
  } catch (error) {
    return still(noValues(error instanceof Error ? error.message : String(error)));
  }
}

/** Where a panel's cells come from: its newest section and the store's groups, and what wakes it. */
export interface CellsSource {
  section(): LiveSection | null;
  group(id: number): GroupValues | undefined;
  subscribe(changed: () => void): () => void;
}

/** A panel's cells read as a chart paints: the same values until its section or one of its groups moves. */
export function followedCells(source: CellsSource): Live<LiveValues<number>> {
  let made: { section: LiveSection | null; groups: GroupValues[]; stamp: number; values: LiveValues<number> } | null = null;
  return {
    read: () => {
      let section: LiveSection | null;
      try { section = source.section(); } catch (error) { return noValues(error instanceof Error ? error.message : String(error)); }
      if (!section) return noValues(NONE);
      const groups = made?.section === section ? made.groups : sectionGroups(section, source.group);
      // A followed group that had not landed yet when the section was resolved is looked up again.
      const fresh = section.groups.some((spec, at) => spec.follow !== undefined && groups[at]!.version === 0 && source.group(spec.follow) !== groups[at])
        ? sectionGroups(section, source.group) : groups;
      const stamp = fresh.reduce((sum, group) => sum + group.version, 0);
      if (!made || made.section !== section || made.stamp !== stamp || made.groups !== fresh)
        made = { section, groups: fresh, stamp, values: sectionValues(section, fresh) };
      return made.values;
    },
    subscribe: changed => source.subscribe(changed),
  };
}

/** `live` read as before, `report` told as each read paints: how a widget reports the hour it shows. */
export function reporting<T>(live: Live<T>, report: () => void): Live<T> {
  return { read: () => { report(); return live.read(); }, subscribe: live.subscribe };
}

/** The follow ids a panel's reply names for its cells. */
export function cellIds(reply: unknown): number[] {
  try {
    return liveSection(reply)?.groups.flatMap(group => group.follow === undefined ? [] : [group.follow]) ?? [];
  } catch {
    return [];
  }
}

/** The reason a live cell has when its table was read but holds no row of its key (the follow layer's `NO_ROW`): for a
 * sparse table whose missing row means zero, the one missing reason that is a value. */
export const NO_ROW = 'No live row has this key';
