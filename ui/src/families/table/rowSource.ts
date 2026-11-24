import type { BoundFacetFilter, LogReplyRow } from '../../charts/log/logSource';
import type { BoundHour } from '../../dashboard/regionTime';
import type { LogCell, LogFacetCountAsk, MissingSpan } from '../../generated/dashboard';

export type TailProduct = 'log-tail' | 'market-ledger-tail';

export interface LogTailQuery {
  readonly product: TailProduct;
  readonly branchId: string;
  readonly log?: string;
  readonly facets: readonly BoundFacetFilter[];
  readonly from: BoundHour | null;
  readonly limit: number;
  readonly columns?: readonly string[];
  readonly countFacets?: readonly LogFacetCountAsk[];
}

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
  readonly missing: readonly MissingSpan[];
}

export type LogRowSlot<TRow extends DatedLogRow> =
  | { readonly kind: 'data'; readonly ordinal: number; readonly readAt: number; readonly row: TRow }
  | { readonly kind: 'gap'; readonly key: string; readonly readAt: number; readonly firstHour: number; readonly lastHour: number; readonly reason: string; readonly beforeOrdinal: number }
  | { readonly kind: 'loading'; readonly ordinal: number; readonly readAt: number }
  | { readonly kind: 'refused'; readonly ordinal: number; readonly readAt: number; readonly reason: string };

export interface RowWindow {
  readonly first: number;
  readonly count: number;
}

export interface PageFetchAsk {
  readonly offset: number;
  readonly limit: number;
  readonly hour: number;
}

export interface WindowPlan {
  readonly fetch: readonly PageFetchAsk[];
  readonly cancel: readonly number[];
}

export interface FreezeState {
  readonly hour: number;
  readonly total: number;
  readonly firstOrdinal: number;
  readonly missing: readonly MissingSpan[];
}

export interface HeadApplyOptions {
  readonly forceFull?: boolean;
}

export interface TableRowSource<TRow extends DatedLogRow> {
  readonly identity: string;
  readonly total: number;
  readonly liveCount: number;
  readonly at: number | null;
  readonly missing: readonly MissingSpan[];
  readonly refused: string | null;
  readonly frozen: FreezeState | null;
  sliceAt(hour: number, first: number, count: number): readonly LogRowSlot<TRow>[];
}

export const DEFAULT_TAIL_PAGE_SIZE = 256;
const NO_MISSING: readonly MissingSpan[] = [];
const NO_PLAN: WindowPlan = { fetch: [], cancel: [] };

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

export function validateTailReply(value: unknown, readAt?: number): TailReply<LogReplyRow> {
  const malformed = () => new Error('The log tail reply is malformed.');
  if (!isObject(value) || !Array.isArray(value.columns) || !value.columns.every(column => typeof column === 'string') || !Array.isArray(value.rows)) {
    throw malformed();
  }
  const total = parseCount(value.total);
  const offset = parseCount(value.offset);
  const baseOrdinal = parseCount(value.baseOrdinal ?? '0');
  const firstOrdinal = parseCount(value.firstOrdinal ?? '0');
  const columns = value.columns as string[];
  if (total === null || offset === null || baseOrdinal === null || firstOrdinal === null || offset + value.rows.length > total) {
    throw malformed();
  }
  const missing = value.missing ?? [];
  if (!Array.isArray(missing) || !missing.every(isSpan)) throw malformed();
  const facetTotals = new Map<string, number>();
  const rawTotals = value.facetTotals ?? [];
  if (!Array.isArray(rawTotals)) throw malformed();
  for (const entry of rawTotals) {
    const count = isObject(entry) ? parseCount(entry.total) : null;
    if (!isObject(entry) || typeof entry.key !== 'string' || count === null) throw malformed();
    facetTotals.set(entry.key, count);
  }
  const rows = (value.rows as unknown[]).map((row): LogReplyRow => {
    if (!isObject(row) || typeof row.key !== 'string' || !Number.isSafeInteger(row.hour)
      || !Array.isArray(row.cells) || row.cells.length !== columns.length || !row.cells.every(isCell)) {
      throw malformed();
    }
    const hour = row.hour as number;
    if (readAt !== undefined && hour > readAt) throw new Error(`The log tail reply contained a row from hour ${hour} after displayed hour ${readAt}.`);
    return {
      key: row.key,
      hour,
      cells: Object.fromEntries(columns.map((column, index) => [column, (row.cells as LogCell[])[index]!])),
      fields: row,
    };
  });
  return { total, offset, baseOrdinal, firstOrdinal, rows, facetTotals, missing };
}

export function buildNavigableSlots<TRow extends DatedLogRow>(
  readAt: number,
  slots: readonly LogRowSlot<TRow>[],
  gaps: readonly MissingSpan[],
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

interface FrozenPicture<TRow extends DatedLogRow> {
  readonly state: FreezeState;
  readonly liveCount: number;
  readonly headRows: readonly TRow[];
  readonly pages: Map<number, readonly TRow[]>;
  readonly refusals: Map<number, string>;
}

export class TailWindowController<TRow extends DatedLogRow> {
  #identity: string;
  #pageSize: number;
  #headHour: number | null = null;
  #head: TailReply<TRow> | null = null;
  #headRefused: string | null = null;
  #liveCount = 0;
  #frozen: FrozenPicture<TRow> | null = null;
  #inFlight = new Set<number>();

  constructor(identity: string, pageSize = DEFAULT_TAIL_PAGE_SIZE) {
    this.#identity = identity;
    this.#pageSize = Math.max(1, pageSize);
  }

  get identity(): string { return this.#identity; }
  get frozen(): FreezeState | null { return this.#frozen?.state ?? null; }
  get at(): number | null { return this.#frozen ? this.#frozen.state.hour : this.#headHour; }

  reset(identity: string): readonly number[] {
    const cancel = [...this.#inFlight];
    this.#identity = identity;
    this.#headHour = null;
    this.#head = null;
    this.#headRefused = null;
    this.#liveCount = 0;
    this.#frozen = null;
    this.#inFlight.clear();
    return cancel;
  }

  applyHead(acknowledgedHour: number, reply: TailReply<TRow>, liveCount = 0, options?: HeadApplyOptions): readonly number[] {
    for (const row of reply.rows) {
      if (row.hour > acknowledgedHour) {
        throw new Error(`Row ${row.key} at hour ${row.hour} exceeds acknowledged hour ${acknowledgedHour}.`);
      }
    }
    this.#headHour = acknowledgedHour;
    this.#head = reply;
    this.#headRefused = null;
    this.#liveCount = Math.max(0, liveCount);
    if (this.#frozen && (options?.forceFull || acknowledgedHour < this.#frozen.state.hour)) {
      return this.releaseFreeze();
    }
    return [];
  }

  refuseHead(acknowledgedHour: number, reason: string, options?: HeadApplyOptions): readonly number[] {
    this.#headHour = acknowledgedHour;
    this.#head = null;
    this.#headRefused = reason;
    if (this.#frozen && (options?.forceFull || acknowledgedHour < this.#frozen.state.hour)) {
      return this.releaseFreeze();
    }
    return [];
  }

  releaseFreeze(): readonly number[] {
    const cancel = [...this.#inFlight];
    this.#frozen = null;
    this.#inFlight.clear();
    return cancel;
  }

  setWindow(window: RowWindow): WindowPlan {
    const first = Math.max(0, Math.floor(window.first));
    const count = Math.max(0, Math.floor(window.count));
    if (count === 0) return NO_PLAN;
    const end = first + count;

    const headLength = this.#frozen ? this.#frozen.headRows.length : (this.#head?.rows.length ?? 0);
    if (first === 0 && end <= headLength) {
      return this.#frozen ? { fetch: [], cancel: this.releaseFreeze() } : NO_PLAN;
    }

    if (!this.#frozen) {
      if (this.#head === null || this.#headHour === null) return NO_PLAN;
      if (end <= this.#head.rows.length || this.#head.rows.length >= this.#head.total) return NO_PLAN;
      this.#frozen = {
        state: {
          hour: this.#headHour,
          total: this.#head.total,
          firstOrdinal: this.#head.firstOrdinal,
          missing: this.#head.missing,
        },
        liveCount: this.#liveCount,
        headRows: this.#head.rows,
        pages: new Map(),
        refusals: new Map(),
      };
    }

    const frozen = this.#frozen;
    const baseOffset = frozen.headRows.length;
    const clampedEnd = Math.min(end, frozen.state.total);
    const wanted = new Set<number>();
    if (clampedEnd > baseOffset) {
      const startPage = Math.max(0, Math.floor((Math.max(first, baseOffset) - baseOffset) / this.#pageSize));
      const endPage = Math.floor((clampedEnd - 1 - baseOffset) / this.#pageSize);
      for (let page = startPage; page <= endPage; page++) {
        wanted.add(baseOffset + page * this.#pageSize);
      }
    }

    const cancel: number[] = [];
    for (const offset of this.#inFlight) {
      if (!wanted.has(offset)) {
        this.#inFlight.delete(offset);
        cancel.push(offset);
      }
    }

    const fetch: PageFetchAsk[] = [];
    for (const offset of wanted) {
      if (frozen.pages.has(offset) || frozen.refusals.has(offset) || this.#inFlight.has(offset)) continue;
      const limit = Math.min(this.#pageSize, frozen.state.total - offset);
      if (limit <= 0) continue;
      this.#inFlight.add(offset);
      fetch.push({ offset, limit, hour: frozen.state.hour });
    }
    return { fetch, cancel };
  }

  applyPage(hour: number, offset: number, rows: readonly TRow[]): boolean {
    if (!this.#frozen || this.#frozen.state.hour !== hour) return false;
    for (const row of rows) {
      if (row.hour > hour) {
        throw new Error(`Page row ${row.key} at hour ${row.hour} exceeds frozen hour ${hour}.`);
      }
    }
    this.#inFlight.delete(offset);
    this.#frozen.refusals.delete(offset);
    this.#frozen.pages.set(offset, rows);
    return true;
  }

  refusePage(hour: number, offset: number, reason: string): boolean {
    if (!this.#frozen || this.#frozen.state.hour !== hour) return false;
    this.#inFlight.delete(offset);
    this.#frozen.refusals.set(offset, reason);
    return true;
  }

  snapshot(): TableRowSource<TRow> {
    const frozen = this.#frozen;
    const activeHour = frozen ? frozen.state.hour : this.#headHour;
    const total = frozen ? frozen.state.total : (this.#head?.total ?? 0);
    const missing = frozen ? frozen.state.missing : (this.#head?.missing ?? NO_MISSING);
    const refused = frozen ? null : this.#headRefused;
    const headRows = frozen ? frozen.headRows : (this.#head?.rows ?? []);
    const pageSize = this.#pageSize;
    const pages = frozen ? new Map(frozen.pages) : null;
    const refusals = frozen ? new Map(frozen.refusals) : null;

    return {
      identity: this.#identity,
      total,
      liveCount: frozen ? frozen.liveCount : this.#liveCount,
      at: activeHour,
      missing,
      refused,
      frozen: frozen?.state ?? null,
      sliceAt(hour: number, first: number, count: number): readonly LogRowSlot<TRow>[] {
        const start = Math.max(0, Math.floor(first));
        const len = Math.max(0, Math.floor(count));
        if (len === 0) return [];
        if (refused !== null && activeHour === hour) {
          return [{ kind: 'refused', ordinal: start, readAt: hour, reason: refused }];
        }
        if (activeHour === null || activeHour !== hour) {
          const out: LogRowSlot<TRow>[] = [];
          const limit = activeHour === null ? len : Math.min(len, Math.max(0, total - start));
          for (let i = 0; i < limit; i++) out.push({ kind: 'loading', ordinal: start + i, readAt: hour });
          return out;
        }
        const out: LogRowSlot<TRow>[] = [];
        const end = Math.min(total, start + len);
        const baseOffset = headRows.length;
        for (let ordinal = start; ordinal < end; ordinal++) {
          if (ordinal < baseOffset) {
            const row = headRows[ordinal]!;
            out.push(row.hour <= hour
              ? { kind: 'data', ordinal, readAt: hour, row }
              : { kind: 'refused', ordinal, readAt: hour, reason: `Row ${row.key} came from after the displayed hour.` });
            continue;
          }
          const pageOffset = baseOffset + Math.floor((ordinal - baseOffset) / pageSize) * pageSize;
          const pageRefused = refusals?.get(pageOffset);
          if (pageRefused !== undefined) {
            out.push({ kind: 'refused', ordinal, readAt: hour, reason: pageRefused });
            continue;
          }
          const page = pages?.get(pageOffset);
          const row = page?.[ordinal - pageOffset];
          if (row) {
            out.push(row.hour <= hour
              ? { kind: 'data', ordinal, readAt: hour, row }
              : { kind: 'refused', ordinal, readAt: hour, reason: `Row ${row.key} came from after the displayed hour.` });
          } else {
            out.push({ kind: 'loading', ordinal, readAt: hour });
          }
        }
        return out;
      },
    };
  }
}
