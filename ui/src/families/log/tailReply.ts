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
  readonly total: string;
  readonly rows: readonly LogPageRow[];
  readonly facetTotals: readonly LogFacetTotalWire[];
  readonly missing: readonly MissingSpan[];
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
): LogTailReplyWire {
  return {
    branchId: page.branchId,
    log: page.log,
    columns: page.columns,
    baseOrdinal: String(baseOrdinal),
    firstOrdinal: page.firstOrdinal,
    total: String(total),
    rows: newestFirstWireRows(page.rows),
    facetTotals,
    missing: page.missing,
    warnings: page.warnings,
  };
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
    facetTotals: pageTail.facetTotals.length > 0 ? pageTail.facetTotals : frozenTail.facetTotals,
    warnings,
  };
}

export function facetTotalMap(reply: LogTailReplyWire): ReadonlyMap<string, number> {
  const out = new Map<string, number>();
  for (const item of reply.facetTotals ?? []) {
    const parsed = Number(item.total);
    if (Number.isFinite(parsed)) out.set(item.key, parsed);
  }
  return out;
}
