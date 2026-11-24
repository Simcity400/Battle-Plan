import type { Failure } from '../../dashboard/queryError.ts';
import type { ExactPicture } from '../../shell/perf.ts';
import type { LogExactReader, LogProjection } from './declaration.ts';
import { buildLogLayout, LOG_MONTH_HEIGHT, stickyMonthAt, visibleItemRange } from './layout.ts';
import { deriveLogAuditRows, type LogDensity } from './model.ts';
import {
  assembleFrozenPageWindow,
  isLogTailReply,
  mergeFrozenPageIntoTail,
  parseOrdinal,
  type LogTailReplyWire,
} from './tailReply.ts';

export interface LoadedPageRef {
  readonly pageIndex: number;
  readonly offset: number;
  readonly limit: number;
}

export interface FrozenViewState {
  readonly gen: number;
  readonly queryKey: string;
  readonly hour: number;
  readonly liveSeq: number;
  readonly baseOrdinal: number;
  readonly total: number;
  readonly tailLoadedCount: number;
  readonly pageSize: number;
  readonly pages: Map<number, LogTailReplyWire>;
  readonly inFlight: Map<number, AbortController>;
  readonly errors: Map<number, Failure>;
  visibleStart: number;
  visibleEnd: number;
  offset: number;
  loadedPages: readonly LoadedPageRef[];
  panels: Record<string, unknown> | null;
}

export function visiblePageIndices(visibleStart: number, visibleEnd: number, pageSize: number, total: number): number[] {
  if (total <= 0) return [];
  const start = Math.max(0, Math.min(total - 1, visibleStart));
  const end = Math.max(start + 1, Math.min(total, visibleEnd));
  const firstPage = Math.floor(start / pageSize);
  const lastPage = Math.floor((end - 1) / pageSize);
  const out: number[] = [];
  for (let p = firstPage; p <= lastPage; p++) out.push(p);
  return out;
}

export function reconcileFrozenView(frozen: FrozenViewState): { readonly activeError: { readonly failure: Failure; readonly pageIndex: number } | null } {
  const needed = visiblePageIndices(frozen.visibleStart, frozen.visibleEnd, frozen.pageSize, frozen.total);
  const tailCoversVisible = frozen.visibleEnd <= frozen.tailLoadedCount && !frozen.pages.has(0);
  if (tailCoversVisible) {
    frozen.offset = 0;
    frozen.loadedPages = [];
    frozen.panels = null;
  } else {
    const assembled = assembleFrozenPageWindow(null, frozen.pages, frozen.pageSize, frozen.visibleStart, frozen.visibleEnd);
    if (assembled) {
      frozen.offset = assembled.offset;
      frozen.loadedPages = assembled.loadedPages;
      frozen.panels = { tail: assembled.tail, __offset: assembled.offset };
    } else {
      frozen.offset = 0;
      frozen.loadedPages = [];
      frozen.panels = null;
    }
  }
  let activeError: { readonly failure: Failure; readonly pageIndex: number } | null = null;
  for (const pageIndex of needed) {
    if (frozen.pages.has(pageIndex)) continue;
    if (pageIndex === 0 && frozen.visibleEnd <= frozen.tailLoadedCount) continue;
    const err = frozen.errors.get(pageIndex);
    if (err) {
      activeError = { failure: err, pageIndex };
      break;
    }
  }
  return { activeError };
}

export interface LogAuditSnapshot<TOptions> {
  readonly queryKey: string;
  readonly projection: LogProjection<TOptions>;
  readonly options: TOptions;
  readonly density: LogDensity;
  readonly months: boolean;
  readonly scrollTop: number;
  readonly height: number;
  readonly limit: number;
  readonly isStatic: boolean;
  readonly exactSources: LogExactReader;
  readonly onLoadScrollPage?: (frozenHour: number, offset: number, pageSize: number, baseOrdinal: number, total: number, signal?: AbortSignal) => Promise<unknown>;
  readonly frozen: {
    readonly hour: number;
    readonly baseOrdinal: number;
    readonly total: number;
    readonly pageSize: number;
    readonly offset: number;
    readonly visibleStart: number;
    readonly visibleEnd: number;
    readonly loadedPages: readonly LoadedPageRef[];
  } | null;
}

export async function verifyLogAuditSnapshot<TOptions>(hour: number, snap: LogAuditSnapshot<TOptions>): ReturnType<ExactPicture> {
  if (snap.frozen) {
    const frozenExact = await snap.exactSources(snap.frozen.hour, snap.limit);
    if (!frozenExact) return null;
    let effectiveSources = frozenExact;
    if (snap.frozen.loadedPages.length > 0 && snap.onLoadScrollPage) {
      const fetched = new Map<number, LogTailReplyWire>();
      for (const pageRef of snap.frozen.loadedPages) {
        const pageTail = await snap.onLoadScrollPage(
          snap.frozen.hour,
          pageRef.offset,
          pageRef.limit,
          snap.frozen.baseOrdinal,
          snap.frozen.total,
        );
        if (!isLogTailReply(pageTail)) return null;
        fetched.set(pageRef.pageIndex, pageTail);
      }
      const baseTail = frozenExact.panels.get('tail');
      const baseWire = isLogTailReply(baseTail) ? baseTail : null;
      const assembled = assembleFrozenPageWindow(baseWire, fetched, snap.frozen.pageSize, snap.frozen.visibleStart, snap.frozen.visibleEnd);
      if (!assembled) return null;
      const mergedTail = baseWire ? mergeFrozenPageIntoTail(baseWire, assembled.tail) : assembled.tail;
      const panels = new Map(frozenExact.panels);
      panels.set('tail', mergedTail);
      panels.set('__offset', assembled.offset);
      effectiveSources = { ...frozenExact, panels };
    }
    let newRowCount = 0;
    if (!snap.isStatic) {
      const liveExact = await snap.exactSources(hour, 1);
      const liveTail = liveExact?.panels.get('tail');
      if (isLogTailReply(liveTail)) {
        newRowCount = Math.max(0, parseOrdinal(liveTail.total) - snap.frozen.total);
      }
    }
    const model = snap.projection.project(effectiveSources, snap.frozen.hour, snap.options);
    const layout = buildLogLayout(model, snap.density, snap.months);
    const [start, end] = visibleItemRange(layout.items, snap.scrollTop, snap.height);
    const sticky = snap.months ? stickyMonthAt(layout, snap.scrollTop) : null;
    const stickyH = sticky ? LOG_MONTH_HEIGHT[snap.density] : 0;
    const rows = deriveLogAuditRows(model, layout.items, start, end, snap.scrollTop, stickyH, snap.height, newRowCount);
    return { values: new Map(rows.map(row => [row.key, row.value ?? ''])), ordered: true };
  }

  const exact = await snap.exactSources(hour, snap.limit);
  if (!exact) return null;
  const model = snap.projection.project(exact, hour, snap.options);
  const layout = buildLogLayout(model, snap.density, snap.months);
  const [start, end] = visibleItemRange(layout.items, snap.scrollTop, snap.height);
  const sticky = snap.months ? stickyMonthAt(layout, snap.scrollTop) : null;
  const stickyH = sticky ? LOG_MONTH_HEIGHT[snap.density] : 0;
  const rows = deriveLogAuditRows(model, layout.items, start, end, snap.scrollTop, stickyH, snap.height, 0);
  return { values: new Map(rows.map(row => [row.key, row.value ?? ''])), ordered: true };
}
