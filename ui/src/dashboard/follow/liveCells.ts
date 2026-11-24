import { liveKey, noValues } from '../../charts/live.ts';
import type { Live, LiveValues } from '../../charts/live';
import { CELL, cellNumber, cellState, cellValue, rowReason } from './message.ts';
import type { FollowedAnswer } from './message';

export const CELLS = 0x8000_0000;
export const isCellsId = (id: number) => id >= CELLS;

export const NOT_READ = 'Not read yet';
export const NO_VALUE = 'No value';
export const NOT_RECORDED = 'Not recorded at this hour';
export const ZERO_DIVISOR = 'Its divisor is zero at this hour';
export const NOT_LIVE_CELL = 'Not a live cell of this reply';

export type LiveOp = 'value' | 'sum' | 'difference' | 'ratio';
export interface LiveKind { id: string; op: LiveOp; scale: number }
export interface LiveGroupSpec { table: number; columns: number[]; keys: string[]; values?: (number | string)[]; rawValues?: (string | null)[]; follow?: number; seedHour?: number; unavailable?: string }
export interface LiveSection { groups: LiveGroupSpec[]; rows: string[]; kinds: LiveKind[]; cells: number[] }

const OPS = new Set<LiveOp>(['value', 'sum', 'difference', 'ratio']);
const RAW_INT = /^-?\d+$/;
const integer = (value: unknown): value is number => Number.isSafeInteger(value) && (value as number) >= 0;
const texts = (value: unknown): value is string[] => Array.isArray(value) && value.every(item => typeof item === 'string');

export function liveSection(reply: unknown): LiveSection | null {
  const live = reply && typeof reply === 'object' ? (reply as { live?: unknown }).live : undefined;
  if (live === undefined || live === null) return null;
  const section = live as LiveSection;
  if (typeof live !== 'object' || !Array.isArray(section.groups) || !texts(section.rows) || !Array.isArray(section.kinds) || !Array.isArray(section.cells))
    throw new Error('A reply\'s live cells are malformed.');
  for (const group of section.groups) {
    const size = Array.isArray(group?.keys) && Array.isArray(group.columns) ? group.keys.length * group.columns.length : -1;
    if (!group || !integer(group.table) || !group.columns.every(integer) || !texts(group.keys)
      || (group.values !== undefined || group.follow !== undefined) === (group.unavailable !== undefined)
      || (group.values !== undefined && group.follow !== undefined) !== (group.seedHour !== undefined)
      || group.values !== undefined && (!Array.isArray(group.values) || group.values.length !== size || !group.values.every(value => typeof value === 'string' || Number.isFinite(value)))
      || group.rawValues !== undefined && (group.values === undefined || !Array.isArray(group.rawValues) || group.rawValues.length !== size
        || !group.rawValues.every((value, at) => value === null || typeof value === 'string' && RAW_INT.test(value) && typeof group.values![at] === 'number'))
      || group.seedHour !== undefined && (!Number.isSafeInteger(group.seedHour) || !group.values?.every(value => typeof value === 'string' || Number.isSafeInteger(value)))
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

export function sectionOf(reply: unknown): LiveSection | null {
  if (!reply || typeof reply !== 'object') return null;
  if (!sections.has(reply)) sections.set(reply, liveSection(reply));
  return sections.get(reply)!;
}

export interface GroupValues {
  get(row: number, column: number): number | undefined;
  raw?(row: number, column: number): bigint | undefined;
  missing(row: number, column: number): string;
}

const missingGroup = (reason: string): GroupValues => ({ get: () => undefined, missing: () => reason });

export function valuedGroup(spec: LiveGroupSpec, values: readonly (number | string)[]): GroupValues {
  const width = spec.columns.length;
  const rawValues = spec.rawValues;
  return {
    get: (row, column) => { const value = values[row * width + column]; return typeof value === 'number' ? value : undefined; },
    raw: (row, column) => {
      const at = row * width + column;
      const value = values[at];
      if (typeof value !== 'number') return undefined;
      const exact = rawValues?.[at];
      if (typeof exact === 'string') return BigInt(exact);
      return Number.isSafeInteger(value) ? BigInt(value) : undefined;
    },
    missing: (row, column) => { const value = values[row * width + column]; return typeof value === 'string' ? value : NOT_READ; },
  };
}

export function answerGroup(answer: FollowedAnswer): GroupValues {
  if (answer.kind === 'notRecorded') return missingGroup(answer.reason);
  const rows = answer.rows;
  const width = answer.columns;
  return {
    get(row, column) {
      if (row < 0 || row >= rows || column < 0 || column >= width) return undefined;
      const at = row * width + column;
      if (cellState(answer, at) !== CELL.value) return undefined;
      const safe = cellNumber(answer, at);
      return safe !== null ? safe : Number(cellValue(answer, at));
    },
    raw(row, column) {
      if (row < 0 || row >= rows || column < 0 || column >= width) return undefined;
      const at = row * width + column;
      if (cellState(answer, at) !== CELL.value) return undefined;
      return cellValue(answer, at);
    },
    missing(row, column) {
      if (row < 0 || row >= rows || column < 0 || column >= width) return NOT_READ;
      const state = cellState(answer, row * width + column);
      return rowReason(answer, row) ?? (state === CELL.missing ? NO_VALUE : NOT_RECORDED);
    },
  };
}

export class CellGroup implements GroupValues {
  seek?: number;
  start: number | null = null;
  private seeded?: GroupValues;

  get landedHour() { return this.seeded ? null : this.start; }

  seed(spec: LiveGroupSpec) {
    if (spec.seedHour === undefined || !spec.values) return;
    this.start = this.through = spec.seedHour;
    this.seeded = valuedGroup(spec, spec.values);
  }

  private confirmed: number | null = null;
  private lane?: { hour: number };
  get through(): number | null { return this.confirmed === null ? null : Math.max(this.confirmed, this.lane?.hour ?? -Infinity); }
  set through(hour: number | null) { this.confirmed = hour; }
  follow(lane?: { hour: number }) { this.confirmed = this.through; this.lane = lane; }
  route: number | null = null;
  private width = 0;
  private values = new Float64Array(0);
  private exact: Map<number, bigint> | null = null;
  private states = new Uint8Array(0);
  private reasons: (string | null)[] = [];
  private whole: string | null = NOT_READ;

  land(answer: FollowedAnswer): void {
    if (answer.kind === 'notRecorded') { this.whole = answer.reason; this.exact = null; return; }
    this.whole = null;
    this.width = answer.columns;
    this.values = new Float64Array(answer.rows * answer.columns);
    this.exact = null;
    this.states = new Uint8Array(answer.rows * answer.columns);
    this.reasons = new Array<string | null>(answer.rows).fill(null);
    for (let row = 0; row < answer.rows; row++) {
      for (let column = 0; column < this.width; column++) {
        const at = row * this.width + column;
        const state = cellState(answer, at);
        this.states[at] = state;
        const safe = cellNumber(answer, at);
        if (safe !== null) {
          this.values[at] = safe;
        } else {
          const big = cellValue(answer, at);
          this.values[at] = Number(big);
          if (state === CELL.value) {
            (this.exact ??= new Map()).set(at, big);
          }
        }
      }
      this.reasons[row] = rowReason(answer, row);
    }
  }

  get(row: number, column: number): number | undefined {
    if (this.seeded) return this.seeded.get(row, column);
    if (this.whole !== null || row >= this.reasons.length) return undefined;
    const at = row * this.width + column;
    return this.states[at] === CELL.value ? this.values[at] : undefined;
  }

  raw(row: number, column: number): bigint | undefined {
    if (this.seeded) return this.seeded.raw?.(row, column);
    if (this.whole !== null || row >= this.reasons.length) return undefined;
    const at = row * this.width + column;
    if (this.states[at] !== CELL.value) return undefined;
    return this.exact?.get(at) ?? BigInt(Math.round(this.values[at]!));
  }

  missing(row: number, column: number): string {
    if (this.seeded) return this.seeded.missing(row, column);
    if (this.whole !== null) return this.whole;
    if (row >= this.reasons.length) return NOT_READ;
    const state = this.states[row * this.width + column];
    return this.reasons[row] ?? (state === CELL.missing ? NO_VALUE : NOT_RECORDED);
  }
}

interface Index { byKey: Map<string, number> }
const indexes = new WeakMap<LiveSection, Index>();

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

export function sectionValues(section: LiveSection, groups: readonly GroupValues[]): LiveValues<number> {
  const { byKey } = indexOf(section);
  const { cells, kinds } = section;
  const evaluate = (at: number): number | string => {
    const kind = kinds[cells[at + 1]!]!, count = cells[at + 2]!;
    let first = 0, second = 0, sum = 0;
    for (let source = 0; source < count; source++) {
      const base = at + 3 + source * 3;
      const group = groups[cells[base]!]!;
      const row = cells[base + 1]!;
      const column = cells[base + 2]!;
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
    raw: key => {
      const at = byKey.get(key);
      if (at === undefined) return undefined;
      const kind = kinds[cells[at + 1]!]!, count = cells[at + 2]!;
      if (kind.op !== 'value' || kind.scale !== 1 || count !== 1) return undefined;
      const base = at + 3;
      return groups[cells[base]!]!.raw?.(cells[base + 1]!, cells[base + 2]!);
    },
    missing: key => { const at = byKey.get(key); if (at === undefined) return NOT_LIVE_CELL; const value = evaluate(at); return typeof value === 'string' ? value : ''; },
  };
}

export function sectionGroups(section: LiveSection, group: (id: number) => GroupValues | undefined): GroupValues[] {
  return section.groups.map(spec => spec.values && spec.follow === undefined ? valuedGroup(spec, spec.values)
    : spec.follow !== undefined ? group(spec.follow) ?? missingGroup(NOT_READ)
    : missingGroup(spec.unavailable ?? NOT_READ));
}

const NONE = 'This reply names no live cells';
const NO_CELLS = noValues(NONE);

const still = (values: LiveValues<number>): Live<LiveValues<number>> => ({ read: () => values, subscribe: () => () => {} });
const STILL = still(NO_CELLS);

export function repliedCells(reply: unknown): Live<LiveValues<number>> {
  try {
    const section = sectionOf(reply);
    return section ? still(sectionValues(section, sectionGroups(section, () => undefined))) : STILL;
  } catch (error) {
    return still(noValues(error instanceof Error ? error.message : String(error)));
  }
}

export function reporting<T>(live: Live<T>, report: () => void): Live<T> {
  return { read: () => { report(); return live.read(); }, subscribe: live.subscribe };
}
