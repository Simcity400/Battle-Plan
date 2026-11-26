import type { BoundHour, BoundWindow } from '../../dashboard/regionTime';
import type { CountrySpan, FacetFilter, LogCell, LogFacetCountAsk, MissingSpan } from '../../generated/dashboard';
import { formatDateOnly } from '../valueFormat.ts';

type BoundFacetFilter = Omit<FacetFilter, 'membership' | 'unobserved'> & {
  membership?: readonly (CountrySpan & BoundWindow)[];
  unobserved?: readonly BoundWindow[];
};

interface LogReplyRow { key: string; hour: number; cells: Readonly<Record<string, LogCell>>; fields: Readonly<Record<string, unknown>> }

export type TailProduct =
  | 'log-tail'
  | 'market-ledger-tail'
  | 'raid-log-tail'
  | 'peace-deals-tail'
  | 'land-battles-tail'
  | 'naval-battles-tail'
  | 'ships-sunk-tail'
  | 'division-log-tail'
  | 'division-service-record-tail'
  | 'espionage-log-tail'
  | 'construction-log-tail'
  | 'event-feed-tail'
  | 'decisions-tail'
  | 'event-choices-tail';

export interface LogTailQuery {
  readonly product: TailProduct;
  readonly branchId: string;
  readonly log?: string;
  readonly facets?: readonly BoundFacetFilter[];
  readonly from: BoundHour | null;
  readonly limit: number;
  readonly columns?: readonly string[];
  readonly countFacets?: readonly LogFacetCountAsk[];
  readonly ask?: Readonly<Record<string, unknown>>;
}

export type PlacedSpan = MissingSpan & { readonly before?: string };

export interface DatedLogRow {
  readonly key: string;
  readonly hour: number;
}

export interface TailReply<TRow extends DatedLogRow = LogReplyRow> {
  readonly total: number;
  readonly offset: number;
  readonly baseOrdinal: number;
  readonly firstOrdinal: number;
  readonly rows: readonly TRow[];
  readonly facetTotals: ReadonlyMap<string, number>;
  readonly facetMissing?: ReadonlyMap<string, string>;
  readonly missing: readonly PlacedSpan[];
}

export type LogRowSlot<TRow extends DatedLogRow> =
  | { readonly kind: 'data'; readonly ordinal: number; readonly readAt: number; readonly row: TRow }
  | { readonly kind: 'gap'; readonly key: string; readonly readAt: number; readonly firstHour: number; readonly lastHour: number; readonly reason: string; readonly beforeOrdinal: number }
  | { readonly kind: 'loading'; readonly ordinal: number; readonly readAt: number }
  | { readonly kind: 'refused'; readonly ordinal: number; readonly readAt: number; readonly reason: string };

export interface RowWindow {
  readonly first: number;
  readonly count: number;
  readonly dataFirst?: number;
  readonly dataCount?: number;
}

export interface PageFetchAsk {
  readonly offset: number;
  readonly limit: number;
  readonly hour: number;
}

export type SnapshotPage<TRow extends DatedLogRow = DatedLogRow> =
  | { readonly hour: number; readonly offset: number; readonly limit: number; readonly reply: TailReply<TRow>; readonly raw: unknown }
  | { readonly hour: number; readonly offset: number; readonly limit: number; readonly refused: string };

export type SnapshotPages<TRow extends DatedLogRow = DatedLogRow> = Readonly<Record<number, SnapshotPage<TRow>>>;

export type PagedRows = Readonly<Record<string, SnapshotPages>>;

export interface TableRowSource<TRow extends DatedLogRow> {
  readonly identity: string;
  readonly total: number;
  readonly at: number | null;
  readonly missing: readonly PlacedSpan[];
  readonly refused: string | null;
  readonly frozen?: true;
  readonly facetTotals?: ReadonlyMap<string, number>;
  readonly facetMissing?: ReadonlyMap<string, string>;
  readonly rawHead?: unknown;
  sliceAt(hour: number, first: number, count: number): readonly LogRowSlot<TRow>[];
}

export const DEFAULT_TAIL_PAGE_SIZE = 256;
const NO_PAGES: SnapshotPages = {};

const isObject = (value: unknown): value is Record<string, unknown> => value !== null && typeof value === 'object' && !Array.isArray(value);

const parseCount = (value: unknown): number | null => {
  if (typeof value !== 'string' || !/^\d+$/.test(value)) return null;
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) ? parsed : null;
};

const isCell = (value: unknown): value is LogCell => isObject(value) && (
  value.kind === 'missing' || value.kind === 'notRecorded'
  || ((value.kind === 'number' || value.kind === 'text') && typeof value.value === 'string')
  || (value.kind === 'list' && typeof value.bytes === 'string')
);

const isSpan = (value: unknown): value is MissingSpan => isObject(value)
  && Number.isSafeInteger(value.firstHour) && Number.isSafeInteger(value.lastHour)
  && (value.firstHour as number) <= (value.lastHour as number) && typeof value.reason === 'string' && value.reason.length > 0;

export function validateRowTailReply<TRow extends DatedLogRow>(
  value: unknown,
  mapRow: (row: Record<string, unknown>, columns: readonly string[]) => TRow,
  readAt?: number,
): TailReply<TRow> {
  const malformed = () => new Error('The log tail reply is malformed.');
  if (!isObject(value) || !Array.isArray(value.rows)) {
    throw malformed();
  }
  const columns = value.columns === undefined ? [] : value.columns;
  if (!Array.isArray(columns) || !columns.every(column => typeof column === 'string')) {
    throw malformed();
  }
  const total = parseCount(value.total);
  const offset = parseCount(value.offset);
  const baseOrdinal = parseCount(value.baseOrdinal ?? '0');
  const firstOrdinal = parseCount(value.firstOrdinal ?? '0');
  if (total === null || offset === null || baseOrdinal === null || firstOrdinal === null || offset + value.rows.length > total) {
    throw malformed();
  }
  const missing = value.missing ?? [];
  if (!Array.isArray(missing) || !missing.every(isSpan)) throw malformed();
  const facetTotals = new Map<string, number>();
  const facetMissing = new Map<string, string>();
  const rawTotals = value.counts ?? [];
  if (!Array.isArray(rawTotals)) throw malformed();
  for (const entry of rawTotals) {
    const count = isObject(entry) ? parseCount(entry.total) : null;
    if (!isObject(entry) || typeof entry.key !== 'string' || count === null) throw malformed();
    const reason = entry.missing ?? null;
    if (reason !== null && typeof reason !== 'string') throw malformed();
    if (reason !== null) facetMissing.set(entry.key, reason);
    else facetTotals.set(entry.key, count);
  }
  const rows = (value.rows as unknown[]).map((raw): TRow => {
    if (!isObject(raw) || typeof raw.key !== 'string' || !Number.isSafeInteger(raw.hour)) {
      throw malformed();
    }
    const hour = raw.hour as number;
    if (readAt !== undefined && hour > readAt) throw new Error(`The log tail reply contained a row from hour ${hour} after displayed hour ${readAt}.`);
    return mapRow(raw, columns);
  });
  return { total, offset, baseOrdinal, firstOrdinal, rows, facetTotals, ...(facetMissing.size ? { facetMissing } : {}), missing };
}

export function validateTailReply(value: unknown, readAt?: number): TailReply<LogReplyRow> {
  return validateRowTailReply(
    value,
    (row, columns): LogReplyRow => {
      if (!Array.isArray(row.cells) || row.cells.length !== columns.length || !row.cells.every(isCell)) {
        throw new Error('The log tail reply is malformed.');
      }
      return {
        key: row.key as string,
        hour: row.hour as number,
        cells: Object.fromEntries(columns.map((column, index) => [column, (row.cells as LogCell[])[index]!])),
        fields: row,
      };
    },
    readAt,
  );
}

export function validateGenericTailReply(value: unknown, readAt?: number): TailReply<DatedLogRow> {
  return validateRowTailReply(
    value,
    (row, columns): DatedLogRow => {
      if (columns.length > 0 && Array.isArray(row.cells) && row.cells.length === columns.length && row.cells.every(isCell)) {
        return {
          key: row.key as string,
          hour: row.hour as number,
          cells: Object.fromEntries(columns.map((column, index) => [column, (row.cells as LogCell[])[index]!])),
          fields: row,
        } as LogReplyRow;
      }
      return row as unknown as DatedLogRow;
    },
    readAt,
  );
}

export function isTailReplyLike(value: unknown): boolean {
  return isObject(value) && Array.isArray(value.rows) && typeof value.total === 'string' && typeof value.offset === 'string';
}

export function gapsBeforeDisplay(displayIndex: number, gapBefore: readonly number[]): number {
  let lo = 0;
  let hi = gapBefore.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (gapBefore[mid]! + mid < displayIndex) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

export function displayWindowToData(
  window: RowWindow,
  gapBefore: readonly number[],
  dataTotal?: number,
): { readonly first: number; readonly count: number } {
  const startDisp = Math.max(0, Math.floor(window.first));
  const count = Math.max(0, Math.floor(window.count));
  const totalDisplay = dataTotal === undefined ? undefined : dataTotal + gapBefore.length;
  const clampedStart = totalDisplay === undefined ? startDisp : Math.min(totalDisplay, startDisp);
  const endDisp = totalDisplay === undefined ? startDisp + count : Math.min(totalDisplay, clampedStart + count);
  const dataFirst = Math.max(0, clampedStart - gapsBeforeDisplay(clampedStart, gapBefore));
  if (endDisp <= clampedStart) return { first: dataFirst, count: 0 };
  const dataEnd = Math.max(dataFirst, endDisp - gapsBeforeDisplay(endDisp, gapBefore));
  return { first: dataFirst, count: dataEnd - dataFirst };
}

export function pageOffsets(window: RowWindow, pageSize: number): readonly number[] {
  const first = Math.max(0, Math.floor(window.first));
  const count = Math.max(0, Math.floor(window.count));
  if (count === 0) return [];
  const size = Math.max(1, Math.floor(pageSize));
  const offsets: number[] = [];
  for (let page = Math.floor(first / size); page * size < first + count; page++) offsets.push(page * size);
  return offsets;
}

function sliceSlots<TRow extends DatedLogRow>(
  reqHour: number,
  first: number,
  count: number,
  total: number,
  slot: (ordinal: number) => LogRowSlot<TRow>,
): LogRowSlot<TRow>[] {
  const start = Math.max(0, Math.floor(first));
  const end = Math.min(total, start + Math.max(0, Math.floor(count)));
  const out: LogRowSlot<TRow>[] = [];
  for (let ordinal = start; ordinal < end; ordinal++) {
    const made = slot(ordinal);
    out.push(made.kind === 'data' && made.row.hour > reqHour
      ? { kind: 'refused', ordinal, readAt: reqHour, reason: `Row ${made.row.key} came from after the displayed hour.` }
      : made);
  }
  return out;
}

function pageAt<TRow extends DatedLogRow>(pages: SnapshotPages<TRow>, ordinal: number, pageSize: number): SnapshotPage<TRow> | undefined {
  return pages[Math.floor(ordinal / pageSize) * pageSize];
}

export function frozenAnswer<TRow extends DatedLogRow>(pages: SnapshotPages<TRow> | undefined, hour: number): Extract<SnapshotPage<TRow>, { reply: unknown }> | undefined {
  const held = pages ? Object.values(pages) : [];
  if (held.length === 0 || held.some(page => page.hour > hour)) return undefined;
  return held.find((page): page is Extract<SnapshotPage<TRow>, { reply: unknown }> => 'reply' in page);
}

export function buildRowSourceFromHead(
  identity: string,
  headRaw: unknown,
  headRefused: string | undefined,
  hour: number,
  pages: SnapshotPages = NO_PAGES,
): TableRowSource<DatedLogRow> | null {
  const held = Object.values(pages);
  const shown = held.length > 0 && held.every(page => page.hour <= hour) ? pages : NO_PAGES;
  const answered = frozenAnswer(shown, hour);
  if (answered) {
    const { reply, hour: at, limit } = answered;
    return {
      identity,
      total: reply.total,
      at,
      missing: reply.missing,
      refused: null,
      frozen: true,
      facetTotals: reply.facetTotals,
      ...(reply.facetMissing ? { facetMissing: reply.facetMissing } : {}),
      rawHead: answered.raw,
      sliceAt: (reqHour, first, count) => sliceSlots(reqHour, first, count, reply.total, ordinal => {
        const page = pageAt(shown, ordinal, limit);
        if (!page || page.hour !== at) return { kind: 'loading', ordinal, readAt: reqHour };
        if ('refused' in page) return { kind: 'refused', ordinal, readAt: reqHour, reason: page.refused };
        const row = page.reply.rows[ordinal - page.offset];
        return row ? { kind: 'data', ordinal, readAt: reqHour, row } : { kind: 'loading', ordinal, readAt: reqHour };
      }),
    };
  }
  const refusals = Object.values(shown);
  const refusedAt = (ordinal: number) => refusals.find(page => 'refused' in page && ordinal >= page.offset && ordinal < page.offset + page.limit);
  if (headRefused !== undefined) {
    return {
      identity,
      total: 0,
      at: hour,
      missing: [],
      refused: headRefused,
      sliceAt: (reqHour, first, count) => (count > 0 ? [{ kind: 'refused', ordinal: Math.max(0, Math.floor(first)), readAt: reqHour, reason: headRefused }] : []),
    };
  }
  if (!isTailReplyLike(headRaw)) return null;
  const head = validateGenericTailReply(headRaw, hour);
  return {
    identity,
    total: head.total,
    at: hour,
    missing: head.missing,
    refused: null,
    facetTotals: head.facetTotals,
    ...(head.facetMissing ? { facetMissing: head.facetMissing } : {}),
    rawHead: headRaw,
    sliceAt: (reqHour, first, count) => sliceSlots(reqHour, first, count, head.total, ordinal => {
      const row = head.rows[ordinal];
      if (row) return { kind: 'data', ordinal, readAt: reqHour, row };
      const refusal = refusedAt(ordinal);
      if (!refusal || !('refused' in refusal)) return { kind: 'loading', ordinal, readAt: reqHour };
      const reason = refusal.hour === hour ? refusal.refused : `Not read at ${formatDateOnly(refusal.hour)}: ${refusal.refused}`;
      return { kind: 'refused', ordinal, readAt: reqHour, reason };
    }),
  };
}

export function buildNavigableSlots<TRow extends DatedLogRow>(
  readAt: number,
  slots: readonly LogRowSlot<TRow>[],
  gaps: readonly PlacedSpan[],
  total: number,
  previousRowHour?: number,
): LogRowSlot<TRow>[] {
  const upperHour = previousRowHour === undefined ? readAt : Math.min(readAt, previousRowHour);
  const activeGaps = gaps
    .filter(span => span.firstHour <= upperHour)
    .map(span => ({
      firstHour: span.firstHour,
      lastHour: Math.min(span.lastHour, upperHour),
      reason: span.reason,
    }))
    .sort((a, b) => b.lastHour - a.lastHour || b.firstHour - a.firstHour);

  if (!activeGaps.length) return [...slots];
  if (gaps.every(span => span.before !== undefined)) return placeByOrdinal(readAt, slots, gaps, upperHour, total);
  if (total === 0) {
    return activeGaps.map(span => ({
      kind: 'gap',
      key: `gap:${span.firstHour}:${span.lastHour}:${span.reason}`,
      readAt,
      firstHour: span.firstHour,
      lastHour: span.lastHour,
      reason: span.reason,
      beforeOrdinal: 0,
    }));
  }
  if (!slots.length) return [];

  const firstData = slots.find(slot => slot.kind === 'data');
  const out: LogRowSlot<TRow>[] = [];
  let gapIndex = 0;
  if (firstData && firstData.ordinal > 0 && previousRowHour === undefined) {
    while (gapIndex < activeGaps.length && activeGaps[gapIndex]!.firstHour > firstData.row.hour) {
      gapIndex++;
    }
  }
  for (const slot of slots) {
    if (slot.kind === 'data') {
      while (gapIndex < activeGaps.length && activeGaps[gapIndex]!.lastHour >= slot.row.hour) {
        const span = activeGaps[gapIndex++]!;
        out.push({
          kind: 'gap',
          key: `gap:${span.firstHour}:${span.lastHour}:${span.reason}`,
          readAt,
          firstHour: span.firstHour,
          lastHour: span.lastHour,
          reason: span.reason,
          beforeOrdinal: slot.ordinal,
        });
      }
    }
    out.push(slot);
  }
  const lastSlot = slots[slots.length - 1]!;
  const reachedTailEnd = lastSlot.kind !== 'gap' && lastSlot.ordinal + 1 >= total;
  if (reachedTailEnd) {
    while (gapIndex < activeGaps.length) {
      const span = activeGaps[gapIndex++]!;
      out.push({
        kind: 'gap',
        key: `gap:${span.firstHour}:${span.lastHour}:${span.reason}`,
        readAt,
        firstHour: span.firstHour,
        lastHour: span.lastHour,
        reason: span.reason,
        beforeOrdinal: total,
      });
    }
  }
  return out;
}

function placeByOrdinal<TRow extends DatedLogRow>(
  readAt: number,
  slots: readonly LogRowSlot<TRow>[],
  gaps: readonly PlacedSpan[],
  upperHour: number,
  total: number,
): LogRowSlot<TRow>[] {
  const placed = gaps
    .filter(span => span.firstHour <= upperHour)
    .map(span => ({ firstHour: span.firstHour, lastHour: Math.min(span.lastHour, upperHour), reason: span.reason, before: Number(span.before) }));
  const gap = (span: typeof placed[number]): LogRowSlot<TRow> => ({
    kind: 'gap',
    key: `gap:${span.firstHour}:${span.lastHour}:${span.reason}`,
    readAt,
    firstHour: span.firstHour,
    lastHour: span.lastHour,
    reason: span.reason,
    beforeOrdinal: span.before,
  });
  if (total === 0) return placed.map(gap);
  const out: LogRowSlot<TRow>[] = [];
  for (const slot of slots) {
    if (slot.kind !== 'gap') {
      for (const span of placed) if (span.before === slot.ordinal) out.push(gap(span));
    }
    out.push(slot);
  }
  const last = slots[slots.length - 1];
  if (last && last.kind !== 'gap' && last.ordinal + 1 >= total) {
    for (const span of placed) if (span.before >= total) out.push(gap(span));
  }
  return out;
}

export interface PagerStep {
  readonly fetch: readonly PageFetchAsk[];
  readonly cancel: readonly number[];
}

const NO_STEP: PagerStep = { fetch: [], cancel: [] };

export class SnapshotPager<TRow extends DatedLogRow = DatedLogRow> {
  readonly #headLimit: number;
  readonly #pageSize: number;
  #hour: number | null = null;
  #pages = new Map<number, SnapshotPage<TRow>>();
  #inFlight = new Set<number>();
  #keep = new Set<number>();
  #header: number | null = null;
  #published: SnapshotPages<TRow> = NO_PAGES as SnapshotPages<TRow>;
  #dirty = false;

  constructor(headLimit: number, pageSize = DEFAULT_TAIL_PAGE_SIZE) {
    this.#headLimit = Math.max(1, Math.floor(headLimit));
    this.#pageSize = Math.max(1, Math.floor(pageSize));
  }

  get hour(): number | null { return this.#hour; }

  get shownHour(): number | null {
    for (const page of this.#pages.values()) if ('reply' in page) return page.hour;
    return null;
  }

  window(window: RowWindow, installedHour: number | null): PagerStep {
    const displayCount = Math.max(0, Math.floor(window.count));
    if (displayCount === 0) return NO_STEP;
    const first = Math.max(0, Math.floor(window.dataFirst ?? window.first));
    const count = Math.max(0, Math.floor(window.dataCount ?? window.count));
    if (count === 0) return NO_STEP;
    if (first + count <= this.#headLimit) return { fetch: [], cancel: this.#release() };
    if (installedHour === null || !Number.isFinite(installedHour)) return NO_STEP;
    const hour = Math.floor(installedHour);
    const cancel: number[] = [];
    if (this.#hour === null || this.#hour > hour) {
      cancel.push(...this.#release());
      this.#hour = hour;
    }
    const wanted = pageOffsets({ first, count }, this.#pageSize);
    const keep = new Set(wanted);
    keep.add(wanted[0]! - this.#pageSize);
    keep.add(wanted[wanted.length - 1]! + this.#pageSize);
    this.#keep = keep;
    for (const offset of this.#pages.keys()) {
      if (keep.has(offset) || offset === this.#header) continue;
      this.#pages.delete(offset);
      this.#dirty = true;
    }
    for (const offset of this.#inFlight) {
      if (wanted.includes(offset)) continue;
      this.#inFlight.delete(offset);
      cancel.push(offset);
    }
    const fetch: PageFetchAsk[] = [];
    for (const offset of wanted) {
      if (this.#pages.has(offset) || this.#inFlight.has(offset)) continue;
      this.#inFlight.add(offset);
      fetch.push({ offset, limit: this.#pageSize, hour: this.#hour });
    }
    return { fetch, cancel };
  }

  answer(ask: PageFetchAsk, reply: TailReply<TRow>, raw: unknown): boolean {
    return this.#settle(ask, { hour: ask.hour, offset: ask.offset, limit: ask.limit, reply, raw });
  }

  refuse(ask: PageFetchAsk, reason: string): boolean {
    return this.#settle(ask, { hour: ask.hour, offset: ask.offset, limit: ask.limit, refused: reason });
  }

  pages(): SnapshotPages<TRow> {
    if (this.#dirty) {
      this.#published = Object.fromEntries(this.#pages) as SnapshotPages<TRow>;
      this.#dirty = false;
    }
    return this.#published;
  }

  #settle(ask: PageFetchAsk, page: SnapshotPage<TRow>): boolean {
    if (ask.hour !== this.#hour || !this.#inFlight.delete(ask.offset)) return false;
    this.#pages.set(ask.offset, page);
    if ('reply' in page) {
      if (this.#header !== null && this.#header !== ask.offset && !this.#keep.has(this.#header)) this.#pages.delete(this.#header);
      this.#header = ask.offset;
    }
    this.#dirty = true;
    return true;
  }

  #release(): number[] {
    const cancel = [...this.#inFlight];
    if (this.#hour !== null || this.#pages.size > 0) this.#dirty = true;
    this.#hour = null;
    this.#pages.clear();
    this.#inFlight.clear();
    this.#keep.clear();
    this.#header = null;
    return cancel;
  }
}
