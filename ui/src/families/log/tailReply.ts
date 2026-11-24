import type { LogCell, LogPageReply, LogPageRow, MissingSpan } from '../../generated/dashboard';

export interface LogFacetTotalWire {
  readonly key: string;
  readonly total: string;
}

export interface LogTailReplyWire {
  readonly branchId: string;
  readonly log: string;
  readonly columns: readonly string[];
  readonly baseOrdinal: string;
  readonly firstOrdinal: string;
  readonly offset?: string;
  readonly total: string;
  readonly rows: readonly LogPageRow[];
  readonly facetTotals: readonly LogFacetTotalWire[];
  readonly missing: readonly MissingSpan[];
  readonly inlineMissing?: readonly MissingSpan[];
  readonly warnings: readonly string[];
}

export function isLogTailReply(value: unknown): value is LogTailReplyWire {
  if (!value || typeof value !== 'object') return false;
  const v = value as Partial<LogTailReplyWire>;
  return typeof v.branchId === 'string'
    && typeof v.log === 'string'
    && Array.isArray(v.columns)
    && Array.isArray(v.rows)
    && typeof v.total === 'string';
}

export function columnIndex(columns: readonly string[]): ReadonlyMap<string, number> {
  const index = new Map<string, number>();
  for (let i = 0; i < columns.length; i++) index.set(columns[i]!, i);
  return index;
}

export function cellAt(row: LogPageRow, index: ReadonlyMap<string, number>, name: string): LogCell | undefined {
  const at = index.get(name);
  return at === undefined ? undefined : row.cells[at];
}

export function cellNumber(cell: LogCell | undefined): number | null {
  if (cell?.kind !== 'number') return null;
  const parsed = Number(cell.value);
  return Number.isFinite(parsed) ? parsed : null;
}

export function cellRawNumber(cell: LogCell | undefined): string | null {
  return cell?.kind === 'number' ? cell.value : null;
}

export function cellText(cell: LogCell | undefined): string | null {
  return cell?.kind === 'text' ? cell.value : null;
}

export function cellIsNotRecorded(cell: LogCell | undefined): boolean {
  return !cell || cell.kind === 'notRecorded' || cell.kind === 'missing';
}

export function nonEmptyText(text: string | null | undefined): string | null {
  if (!text) return null;
  const trimmed = text.trim();
  return trimmed.length > 0 ? trimmed : null;
}

export function parseOrdinal(raw: string | number | undefined): number {
  if (typeof raw === 'number') return Number.isFinite(raw) ? raw : 0;
  if (typeof raw === 'string') {
    const parsed = Number(raw);
    return Number.isFinite(parsed) ? parsed : 0;
  }
  return 0;
}

export function scrollPageOrdinalRange(baseOrdinal: number, total: number, offset: number, pageSize: number): { readonly cursor: string; readonly pageSize: number } | null {
  if (total <= 0 || offset >= total) return null;
  const end = Math.max(0, total - offset);
  const start = Math.max(0, end - pageSize);
  const count = end - start;
  if (count <= 0) return null;
  return {
    cursor: String(baseOrdinal + start),
    pageSize: count,
  };
}

export function compareOrdinalStrings(a: string | undefined, b: string | undefined): number {
  const left = a ?? '0';
  const right = b ?? '0';
  if (left.length !== right.length) return left.length - right.length;
  return left < right ? -1 : left > right ? 1 : 0;
}

export function newestFirstWireRows(rows: readonly LogPageRow[]): readonly LogPageRow[] {
  if (rows.length <= 1) return rows;
  let alreadyOrdered = true;
  for (let i = 1; i < rows.length; i++) {
    const prev = rows[i - 1]!;
    const cur = rows[i]!;
    if (prev.hour < cur.hour || (prev.hour === cur.hour && compareOrdinalStrings(prev.ordinal, cur.ordinal) > 0)) {
      alreadyOrdered = false;
      break;
    }
  }
  if (alreadyOrdered) return rows;
  return [...rows].sort((a, b) => b.hour - a.hour || compareOrdinalStrings(a.ordinal, b.ordinal));
}

export function pageReplyToTail(
  page: LogPageReply,
  baseOrdinal: number,
  total: number,
  facetTotals: readonly LogFacetTotalWire[] = [],
  offset = 0,
): LogTailReplyWire {
  return {
    branchId: page.branchId,
    log: page.log,
    columns: page.columns,
    baseOrdinal: String(baseOrdinal),
    firstOrdinal: page.firstOrdinal,
    offset: String(offset),
    total: String(total),
    rows: newestFirstWireRows(page.rows),
    facetTotals,
    missing: page.missing,
    warnings: page.warnings,
  };
}

export function mergeMissingSpans(a: readonly MissingSpan[], b: readonly MissingSpan[]): readonly MissingSpan[] {
  if (b.length === 0) return a;
  if (a.length === 0) return b;
  const seen = new Set<string>();
  const out: MissingSpan[] = [];
  for (const list of [a, b]) {
    for (const span of list) {
      const key = `${span.firstHour}:${span.lastHour}:${span.reason}`;
      if (!seen.has(key)) {
        seen.add(key);
        out.push(span);
      }
    }
  }
  out.sort((x, y) => x.firstHour - y.firstHour || x.lastHour - y.lastHour);
  return out;
}

export function mergeFrozenPageIntoTail(
  frozenTail: LogTailReplyWire,
  pageTail: LogTailReplyWire,
): LogTailReplyWire {
  const warnings: string[] = [];
  const seen = new Set<string>();
  for (const list of [frozenTail.warnings, pageTail.warnings]) {
    for (const msg of list ?? []) {
      if (msg && !seen.has(msg)) {
        seen.add(msg);
        warnings.push(msg);
      }
    }
  }
  return {
    ...pageTail,
    baseOrdinal: frozenTail.baseOrdinal,
    total: frozenTail.total,
    facetTotals: pageTail.facetTotals.length > 0 ? pageTail.facetTotals : frozenTail.facetTotals,
    missing: frozenTail.missing,
    inlineMissing: mergeMissingSpans(frozenTail.inlineMissing ?? frozenTail.missing, pageTail.inlineMissing ?? pageTail.missing),
    warnings,
  };
}

export interface AssembledPageSlice {
  readonly offset: number;
  readonly tail: LogTailReplyWire;
  readonly loadedPages: readonly { readonly pageIndex: number; readonly offset: number; readonly limit: number }[];
}

export function assembleFrozenPageWindow(
  frozenTail: LogTailReplyWire | null,
  pages: ReadonlyMap<number, LogTailReplyWire>,
  pageSize: number,
  wantStart: number,
  wantEnd: number,
): AssembledPageSlice | null {
  const firstPage = Math.max(0, Math.floor(wantStart / pageSize));
  const lastPage = Math.max(firstPage, Math.floor(Math.max(wantStart, wantEnd - 1) / pageSize));

  const getChunk = (pageIndex: number): { readonly wire: LogTailReplyWire; readonly fetched: boolean } | null => {
    const fetched = pages.get(pageIndex);
    if (fetched) return { wire: fetched, fetched: true };
    if (pageIndex === 0 && frozenTail) return { wire: frozenTail, fetched: false };
    return null;
  };

  let startPage = firstPage;
  let chunk = getChunk(startPage);
  if (!chunk) {
    for (let p = firstPage + 1; p <= lastPage; p++) {
      const candidate = getChunk(p);
      if (candidate) {
        startPage = p;
        chunk = candidate;
        break;
      }
    }
  }
  if (!chunk) {
    if (frozenTail) {
      return { offset: 0, tail: frozenTail, loadedPages: [] };
    }
    return null;
  }

  const rows: LogPageRow[] = [...chunk.wire.rows];
  let inlineMissing: readonly MissingSpan[] = chunk.wire.inlineMissing ?? chunk.wire.missing;
  const warnings: string[] = [...chunk.wire.warnings];
  const seenWarn = new Set<string>(warnings);
  const loadedPages: { readonly pageIndex: number; readonly offset: number; readonly limit: number }[] = [];
  if (chunk.fetched) {
    loadedPages.push({ pageIndex: startPage, offset: startPage * pageSize, limit: pageSize });
  }

  const expectedChunkLen = (p: number) => p * pageSize + pageSize;
  let endIndex = startPage * pageSize + chunk.wire.rows.length;
  for (let p = startPage + 1; p <= lastPage; p++) {
    if (endIndex < expectedChunkLen(p - 1)) break;
    const next = getChunk(p);
    if (!next) break;
    rows.push(...next.wire.rows);
    inlineMissing = mergeMissingSpans(inlineMissing, next.wire.inlineMissing ?? next.wire.missing);
    for (const w of next.wire.warnings) {
      if (w && !seenWarn.has(w)) {
        seenWarn.add(w);
        warnings.push(w);
      }
    }
    if (next.fetched) {
      loadedPages.push({ pageIndex: p, offset: p * pageSize, limit: pageSize });
    }
    endIndex += next.wire.rows.length;
  }

  const base = frozenTail ?? chunk.wire;
  const offset = startPage * pageSize;
  const assembled: LogTailReplyWire = {
    branchId: base.branchId,
    log: base.log,
    columns: base.columns,
    baseOrdinal: base.baseOrdinal,
    firstOrdinal: rows[0]?.ordinal ?? base.firstOrdinal,
    offset: String(offset),
    total: base.total,
    rows,
    facetTotals: base.facetTotals,
    missing: base.missing,
    inlineMissing: mergeMissingSpans(base.inlineMissing ?? base.missing, inlineMissing),
    warnings: frozenTail
      ? [...new Set([...frozenTail.warnings, ...warnings])]
      : warnings,
  };
  return { offset, tail: assembled, loadedPages };
}

export function facetTotalMap(reply: LogTailReplyWire): ReadonlyMap<string, number> {
  const out = new Map<string, number>();
  for (const item of reply.facetTotals ?? []) {
    const parsed = Number(item.total);
    if (Number.isFinite(parsed)) out.set(item.key, parsed);
  }
  return out;
}
