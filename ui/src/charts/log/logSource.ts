import type { BoundHour, BoundWindow, ParamsFor } from '../../dashboard/regionTime';
import type { Failure } from '../../dashboard/queryError';
import type { FacetFilter, LogCell, MembershipRun, MissingSpan } from '../../generated/dashboard';
import type { LogPage, LogRow } from './logData';
import { AHEAD_SECONDS } from '../reveal.ts';
import type { DeclaredProduct } from '../../dashboard/request';

export interface LogCounts { firstHour: number; binHours: number; counts: readonly number[]; total: number; before: number; missing: readonly MissingSpan[] }

export interface LogReplyRow { key: string; hour: number; cells: Readonly<Record<string, LogCell>>; fields: Readonly<Record<string, unknown>> }
export interface LogReplyPage { firstOrdinal: number; total: number; rows: readonly LogReplyRow[] }

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null;
const count = (value: unknown): value is number => Number.isSafeInteger(value) && (value as number) >= 0;
const counted = (value: unknown): number | null => {
  if (typeof value !== 'string' || !/^\d+$/.test(value)) return null;
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) ? parsed : null;
};
const span = (value: unknown): value is MissingSpan => object(value) && Number.isSafeInteger(value.firstHour) && Number.isSafeInteger(value.lastHour)
  && (value.firstHour as number) <= (value.lastHour as number) && typeof value.reason === 'string' && value.reason.length > 0;
const cellOf = (value: unknown): value is LogCell => object(value) && (
  ((value.kind === 'number' || value.kind === 'text') && typeof value.value === 'string')
  || (value.kind === 'list' && typeof value.bytes === 'string') || value.kind === 'missing' || value.kind === 'notRecorded');

export function validateLogCounts(value: unknown, firstHour: number, lastHour: number): LogCounts {
  const malformed = () => new Error('The log counts reply is malformed.');
  if (!object(value) || !Number.isSafeInteger(value.firstHour) || !count(value.binHours) || value.binHours === 0
    || !Array.isArray(value.lines) || lastHour < firstHour) throw malformed();
  const binHours = value.binHours as number, start = value.firstHour as number;
  if (start > firstHour || firstHour - start >= binHours) throw malformed();
  const counts = new Array<number>(Math.floor((lastHour - start) / binHours) + 1).fill(0);
  let total = 0, before = 0;
  for (const line of value.lines as unknown[]) {
    const lineTotal = object(line) ? counted(line.total) : null, lineBefore = object(line) ? counted(line.before) : null;
    if (!object(line) || lineTotal === null || lineBefore === null || !Array.isArray(line.bins)) throw malformed();
    let sum = 0;
    for (const bin of line.bins as unknown[]) {
      if (!Array.isArray(bin) || bin.length !== 2 || !count(bin[0]) || !count(bin[1]) || bin[0] >= counts.length) throw malformed();
      counts[bin[0]] += bin[1]; sum += bin[1];
    }
    if (sum !== lineTotal) throw malformed();
    total += lineTotal; before += lineBefore;
  }
  if (!Number.isSafeInteger(total + before)) throw malformed();
  const missing = value.missing ?? [];
  if (!Array.isArray(missing) || !missing.every(span) || missing.some((held, at) => at > 0 && held.firstHour <= missing[at - 1].lastHour)) throw malformed();
  return { firstHour: start, binHours, counts, total, before, missing };
}

export function missingThrough(missing: readonly MissingSpan[], until: number | null): MissingSpan[] {
  return missing.filter(held => until === null || held.firstHour <= until).map(held => until !== null && held.lastHour > until ? { ...held, lastHour: until } : held);
}

export function validateLogPage(value: unknown, firstOrdinal: number, pageSize: number, before: number): LogReplyPage {
  const malformed = () => new Error('The log page reply is malformed.');
  if (!object(value) || !Array.isArray(value.columns) || !value.columns.every(column => typeof column === 'string')
    || counted(value.firstOrdinal) !== before + firstOrdinal || !Array.isArray(value.rows) || value.rows.length > pageSize) throw malformed();
  const total = counted(value.total), columns = value.columns as string[];
  if (total === null || firstOrdinal + value.rows.length > total) throw malformed();
  const rows = (value.rows as unknown[]).map((row, at): LogReplyRow => {
    if (!object(row) || typeof row.key !== 'string' || !Number.isSafeInteger(row.hour) || counted(row.ordinal) !== before + firstOrdinal + at
      || !Array.isArray(row.cells) || row.cells.length !== columns.length || !row.cells.every(cellOf)) throw malformed();
    return { key: row.key, hour: row.hour as number, cells: Object.fromEntries(columns.map((column, index) => [column, (row.cells as LogCell[])[index]])), fields: row };
  });
  if (rows.some((row, at) => at > 0 && row.hour < rows[at - 1].hour)) throw malformed();
  return { firstOrdinal, total, rows };
}

export function logPageRequest(spec: { product?: DeclaredProduct; branchId: string; log: string; facets?: readonly FacetFilter[]; scope?: readonly (MembershipRun & BoundWindow)[];
  firstHour: BoundHour; lastHour: BoundHour }, cursor: number, pageSize: number): { product: DeclaredProduct; params: ParamsFor<DeclaredProduct> } {
  return { product: spec.product ?? 'log-page', params: { branchId: spec.branchId, ...(spec.product ? {} : { log: spec.log }),
    ...(spec.facets ? { facets: spec.facets } : {}), ...(spec.scope ? { scope: spec.scope } : {}), from: spec.firstHour, to: spec.lastHour, cursor: String(cursor), pageSize } };
}

export interface HeldPages<R, N> {
  lane: string | null;
  rows: ReadonlyMap<number, R>;
  notes: ReadonlyMap<number, readonly N[]>;
  errors: ReadonlyMap<number, Failure>;
}

export const noPages = <R, N>(lane: string | null): HeldPages<R, N> => ({ lane, rows: new Map(), notes: new Map(), errors: new Map() });

export const pagesOf = <R, N>(held: HeldPages<R, N>, lane: string | null): HeldPages<R, N> => held.lane === lane ? held : noPages(lane);

export function heldPage<R, N>(held: HeldPages<R, N>, shown: string | null, asked: string | null, index: number, rows: R,
  said: readonly N[] | undefined): HeldPages<R, N> {
  if (asked !== shown) return held;
  const base = pagesOf(held, asked);
  return { lane: asked, rows: new Map(base.rows).set(index, rows), notes: said ? new Map(base.notes).set(index, said) : base.notes,
    errors: base.errors.has(index) ? new Map([...base.errors].filter(([at]) => at !== index)) : base.errors };
}

export function failedPage<R, N>(held: HeldPages<R, N>, shown: string | null, asked: string | null, index: number, failure: Failure): HeldPages<R, N> {
  if (asked !== shown) return held;
  const base = pagesOf(held, asked);
  return { ...base, errors: new Map(base.errors).set(index, failure) };
}

export function pageInSpan(index: number, pageSize: number, total: number): boolean {
  return Number.isSafeInteger(index) && index >= 0 && index * pageSize < total;
}

export function cumulativeCounts(counts: LogCounts): Uint32Array {
  const before = new Uint32Array(counts.counts.length + 1);
  for (let at = 0; at < counts.counts.length; at++) before[at + 1] = before[at] + counts.counts[at];
  return before;
}

export function shownThrough(counts: LogCounts, before: Uint32Array, until: number | null, pageSize: number,
  page: (index: number) => readonly { hour: number }[] | undefined): { shown: number; known: boolean } {
  if (until === null) return { shown: counts.total, known: true };
  const bin = Math.floor((until - counts.firstHour) / counts.binHours);
  if (bin < 0) return { shown: 0, known: true };
  if (bin >= counts.counts.length) return { shown: counts.total, known: true };
  const first = before[bin], end = before[bin + 1];
  let shown = first;
  for (let index = Math.floor(first / pageSize); index * pageSize < end; index++) {
    const rows = page(index);
    if (!rows) return { shown: first, known: false };
    for (let at = Math.max(first, index * pageSize); at < Math.min(end, (index + 1) * pageSize); at++) {
      if (rows[at - index * pageSize]!.hour <= until) shown = at + 1;
    }
  }
  return { shown, known: true };
}

export function binPages(counts: LogCounts, before: Uint32Array, hour: number | null, pageSize: number): number[] {
  if (hour === null) return [];
  const bin = Math.floor((hour - counts.firstHour) / counts.binHours);
  if (bin < 0 || bin >= counts.counts.length) return [];
  const pages: number[] = [];
  for (let index = Math.floor(before[bin] / pageSize); index * pageSize < before[bin + 1]; index++) pages.push(index);
  return pages;
}

export function askedPages(inView: readonly string[], shown: number, pageSize: number, follow: boolean, waiting: readonly number[], through = shown + 1): Set<number> {
  const asked = new Set(inView.map(Number).filter(Number.isSafeInteger));
  if (follow) {
    const head = Math.floor(shown / pageSize);
    for (let index = head, last = Math.max(head + 1, Math.floor((through - 1) / pageSize)); index <= last; index++) asked.add(index);
  }
  for (const index of waiting) asked.add(index);
  return asked;
}

export function aheadThrough(counts: LogCounts, before: Uint32Array, shown: number, hour: number, hoursPerSecond: number, playing: boolean): number {
  if (!playing || !(hoursPerSecond > 0)) return shown + 1;
  const bin = Math.floor((hour + Math.ceil(hoursPerSecond * AHEAD_SECONDS) - counts.firstHour) / counts.binHours);
  const counted = bin < 0 ? 0 : before[Math.min(bin + 1, counts.counts.length)];
  return Math.max(shown + 1, counted);
}

export function pageFirstHour(counts: LogCounts, before: Uint32Array, at: number): number {
  let low = 0, high = counts.counts.length - 1;
  while (low < high) {
    const middle = (low + high + 1) >> 1;
    if (before[middle] <= at) low = middle; else high = middle - 1;
  }
  return counts.firstHour + low * counts.binHours;
}

export function shownPages<T>(shown: number, pageSize: number, loaded: ReadonlyMap<number, readonly LogRow<T>[]>,
  errors: ReadonlyMap<number, Failure>): LogPage<T>[] {
  const pages: LogPage<T>[] = [];
  for (let index = 0; index * pageSize < shown; index++) {
    const start = index * pageSize, size = Math.min(pageSize, shown - start);
    const rows = loaded.get(index), error = errors.get(index);
    pages.push({ key: String(index), start, end: start + size - 1, count: size,
      ...(rows ? { rows: rows.length > size ? rows.slice(0, size) : rows } : {}), ...(error ? { error } : {}) });
  }
  return pages;
}

export function missingNote(missing: readonly MissingSpan[], date: (hour: number) => string, subject = 'Not recorded'):
  { spans: { text: string; detail: string }[]; counted: string } | null {
  if (!missing.length) return null;
  return {
    spans: missing.map(held => {
      const first = date(held.firstHour), last = date(held.lastHour);
      return { text: `${subject} ${first === last ? `on ${first}` : `from ${first} to ${last}`}.`, detail: held.reason };
    }),
    counted: 'Those hours are not empty: every count covers the recorded hours only.',
  };
}

export function rowsAhead(shown: number, total: number, pageSize: number, limit: number,
  page: (index: number) => readonly { hour: number }[] | undefined): { hours: number[]; complete: boolean } {
  const hours: number[] = [];
  for (let ordinal = shown; ordinal < total; ordinal++) {
    if (hours.length >= limit) return { hours, complete: false };
    const row = page(Math.floor(ordinal / pageSize))?.[ordinal % pageSize];
    if (!row) return { hours, complete: false };
    hours.push(row.hour);
  }
  return { hours, complete: true };
}

export function drawnAhead<T>(pages: readonly LogPage<T>[], shown: number): LogRow<T>[] {
  const rows: LogRow<T>[] = [];
  for (const page of pages) {
    if (page.end < shown || !page.rows) continue;
    for (let at = Math.max(shown, page.start); at <= page.end; at++) {
      const row = page.rows[at - page.start];
      if (row) rows.push(row);
    }
  }
  return rows;
}

export function countsAhead<K, T>(base: ReadonlyMap<K, number>, ahead: readonly LogRow<T>[], keyOf: (row: LogRow<T>) => K): (hour: number) => ReadonlyMap<K, number> {
  const hours = new Map<K, number[]>();
  for (const row of ahead) {
    const key = keyOf(row);
    if (base.has(key)) { const list = hours.get(key); if (list) list.push(row.hour); else hours.set(key, [row.hour]); }
  }
  const sorted = new Map([...hours].map(([key, list]) => [key, Float64Array.from(list).sort()]));
  return hour => {
    if (!sorted.size) return base;
    const out = new Map(base);
    for (const [key, list] of sorted) {
      let low = 0, high = list.length;
      while (low < high) { const middle = (low + high) >> 1; if (list[middle] <= hour) low = middle + 1; else high = middle; }
      out.set(key, base.get(key)! + low);
    }
    return out;
  };
}
