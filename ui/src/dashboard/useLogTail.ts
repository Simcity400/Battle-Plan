import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { LogPage, LogRow } from '../charts/log/logData';
import { stableLogPages } from '../charts/log/logPages.ts';
import type { LogReplyRow } from '../charts/log/logSource';
import type { LogCell, MissingSpan } from '../generated/dashboard';
import type { WidgetRuntime } from './definitions';
import { useHeadProduct } from './headProduct.ts';
import type { DashboardSource } from './client';
import { inFrame } from './frameUpdates.ts';
import { loadProduct } from './products.ts';
import { asFailure, type Failure } from './queryError.ts';
import type { Gate } from './regionGate';
import type { BoundHour, GatedHour, ParamsFor } from './regionTime';
import { forwardRequest } from './request.ts';
import type { BoundFacetFilter, LogCut } from './useLogPages';

export interface LogTailSpec {
  source: DashboardSource;
  branchId: string;
  facets: readonly BoundFacetFilter[];
  from: BoundHour;
  lastHour: BoundHour;
}

export interface ValidatedLogTail {
  total: number;
  offset: number;
  rows: readonly LogReplyRow[];
  missing: readonly MissingSpan[];
}

interface FrozenTail<T> {
  owner: string;
  hour: number;
  total: number;
  tailRows: readonly LogRow<T>[];
  missing: readonly MissingSpan[];
  pages: ReadonlyMap<number, readonly LogRow<T>[]>;
  errors: ReadonlyMap<number, Failure>;
}

export const TAIL_LIMIT = 128;
export const TAIL_PAGE_SIZE = 256;
const NO_MISSING: readonly MissingSpan[] = [];
const NO_PAGES: ReadonlyMap<number, readonly LogRow<never>[]> = new Map();
const NO_ERRORS: ReadonlyMap<number, Failure> = new Map();

const object = (value: unknown): value is Record<string, unknown> => value !== null && typeof value === 'object' && !Array.isArray(value);
const counted = (value: unknown): number | null => {
  if (typeof value !== 'string' || !/^\d+$/.test(value)) return null;
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) ? parsed : null;
};
const cellOf = (value: unknown): value is LogCell => object(value) && (
  value.kind === 'missing' || value.kind === 'notRecorded'
  || ((value.kind === 'number' || value.kind === 'text') && typeof value.value === 'string')
  || (value.kind === 'list' && typeof value.bytes === 'string')
);
const spanOf = (value: unknown): value is MissingSpan => object(value)
  && Number.isSafeInteger(value.firstHour) && Number.isSafeInteger(value.lastHour)
  && (value.firstHour as number) <= (value.lastHour as number) && typeof value.reason === 'string';

export function validateLogTail(value: unknown): ValidatedLogTail {
  const malformed = () => new Error('The log tail reply is malformed.');
  if (!object(value) || !Array.isArray(value.columns) || !value.columns.every(column => typeof column === 'string') || !Array.isArray(value.rows)) throw malformed();
  const total = counted(value.total), offset = counted(value.offset), columns = value.columns as string[];
  if (total === null || offset === null || offset + value.rows.length > total) throw malformed();
  const missing = value.missing ?? [];
  if (!Array.isArray(missing) || !missing.every(spanOf)) throw malformed();
  const rows = (value.rows as unknown[]).map((row): LogReplyRow => {
    if (!object(row) || typeof row.key !== 'string' || !Number.isSafeInteger(row.hour)
      || !Array.isArray(row.cells) || row.cells.length !== columns.length || !row.cells.every(cellOf)) throw malformed();
    return { key: row.key, hour: row.hour as number, cells: Object.fromEntries(columns.map((column, index) => [column, (row.cells as LogCell[])[index]])), fields: row };
  });
  return { total, offset, rows, missing };
}

export function buildTailPages<T>(total: number, tailRows: readonly LogRow<T>[], pageSize: number,
  loaded: ReadonlyMap<number, readonly LogRow<T>[]>, errors: ReadonlyMap<number, Failure>): LogPage<T>[] {
  if (total <= 0) return [];
  const pages: LogPage<T>[] = [{ key: 'tail', start: total - tailRows.length, end: total, count: tailRows.length, rows: tailRows }];
  for (let offset = tailRows.length; offset < total; offset += pageSize) {
    const count = Math.min(pageSize, total - offset);
    pages.push({ key: String(offset), start: total - offset - count, end: total - offset, count, rows: loaded.get(offset), error: errors.get(offset) });
  }
  return pages;
}

export function useLogTail<T>(runtime: WidgetRuntime, gate: Gate, product: 'market-ledger-tail', spec: LogTailSpec | null, cut: LogCut, active: boolean,
  mapRow: (reply: LogReplyRow) => LogRow<T>, tailLimit = TAIL_LIMIT, pageSize = TAIL_PAGE_SIZE) {
  const owner = spec ? JSON.stringify([spec.source.id, spec.source.revision, spec.branchId, product, spec.facets, spec.from, spec.lastHour, tailLimit, pageSize]) : null;
  const wanted = useMemo<ParamsFor<'market-ledger-tail'> | null>(() => spec
    ? { branchId: spec.branchId, facets: spec.facets, from: spec.from, limit: tailLimit, countFacets: [] } as unknown as ParamsFor<'market-ledger-tail'>
    : null, [spec?.branchId, spec?.facets, spec?.from, tailLimit]);
  const head = useHeadProduct<ValidatedLogTail, 'market-ledger-tail'>(runtime, gate, product, wanted, { active: active && !!spec, validate: validateLogTail });
  const tailRows = useMemo(() => head.data ? head.data.rows.map(mapRow) : [], [head.data, mapRow]);
  const [frozen, setFrozen] = useState<FrozenTail<T> | null>(null);
  const flights = useRef(new Map<number, AbortController>());
  const activeFrozen = frozen && frozen.owner === owner && (!cut.follow || head.at === null || head.at >= frozen.hour) ? frozen : null;

  useEffect(() => {
    for (const controller of flights.current.values()) controller.abort();
    flights.current.clear();
    setFrozen(null);
  }, [owner]);

  useEffect(() => {
    if (frozen && !activeFrozen) {
      for (const controller of flights.current.values()) controller.abort();
      flights.current.clear();
      setFrozen(null);
    }
  }, [frozen, activeFrozen]);

  const latest = useRef({ spec, owner, head, tailRows, activeFrozen, mapRow, pageSize });
  latest.current = { spec, owner, head, tailRows, activeFrozen, mapRow, pageSize };

  const onWantedPages = useCallback((keys: readonly string[]) => {
    const { spec: currentSpec, owner: currentOwner, head: currentHead, tailRows: currentTail, activeFrozen: held, mapRow: map, pageSize: size } = latest.current;
    const offsets = keys.filter(key => key !== 'tail').map(Number).filter(offset => Number.isSafeInteger(offset) && offset >= 0);
    if (!offsets.length || !currentSpec || !currentOwner || !currentHead.data) {
      for (const controller of flights.current.values()) controller.abort();
      flights.current.clear();
      if (held) setFrozen(null);
      return;
    }
    const snap: FrozenTail<T> = held ?? {
      owner: currentOwner, hour: currentHead.at ?? currentSpec.lastHour, total: currentHead.data.total,
      tailRows: currentTail, missing: currentHead.data.missing, pages: new Map(), errors: new Map(),
    };
    if (!held) setFrozen(snap);
    const wantedSet = new Set(offsets);
    for (const [offset, controller] of flights.current) if (!wantedSet.has(offset)) { controller.abort(); flights.current.delete(offset); }
    for (const offset of offsets) {
      if (offset < snap.tailRows.length || offset >= snap.total || snap.pages.has(offset) || snap.errors.has(offset) || flights.current.has(offset)) continue;
      const controller = new AbortController();
      flights.current.set(offset, controller);
      const count = Math.min(size, snap.total - offset);
      const req = forwardRequest(currentSpec.source, product, {
        branchId: currentSpec.branchId, facets: currentSpec.facets, from: currentSpec.from,
        hour: snap.hour as GatedHour, offset: String(offset), limit: count, countFacets: [],
      } as unknown as ParamsFor<'market-ledger-tail'>);
      loadProduct(runtime.client, req, controller.signal, validateLogTail, currentSpec.source).then(reply => {
        if (controller.signal.aborted) return;
        const rows = reply.rows.map(map);
        inFrame(() => setFrozen(prev => prev && prev.owner === currentOwner && prev.hour === snap.hour
          ? { ...prev, pages: new Map(prev.pages).set(offset, rows) } : prev));
      }).catch(error => {
        if (controller.signal.aborted) return;
        const failure = asFailure(error);
        inFrame(() => setFrozen(prev => prev && prev.owner === currentOwner && prev.hour === snap.hour
          ? { ...prev, errors: new Map(prev.errors).set(offset, failure) } : prev));
      }).finally(() => { if (flights.current.get(offset) === controller) flights.current.delete(offset); });
    }
  }, [runtime.client, product]);

  const total = activeFrozen ? activeFrozen.total : (head.data?.total ?? 0);
  const shownTail = activeFrozen ? activeFrozen.tailRows : tailRows;
  const loadedPages = activeFrozen ? activeFrozen.pages : NO_PAGES;
  const pageErrors = activeFrozen ? activeFrozen.errors : NO_ERRORS;
  const previous = useRef<readonly LogPage<T>[]>([]);
  const pages = useMemo(() => previous.current = stableLogPages(previous.current, buildTailPages(total, shownTail, pageSize, loadedPages, pageErrors)),
    [total, shownTail, pageSize, loadedPages, pageErrors]);
  const pageError = pages.find(page => page.error)?.error;
  const retry = useCallback(() => {
    head.retry();
    setFrozen(prev => prev ? { ...prev, errors: new Map() } : null);
  }, [head.retry]);

  const frozenHour = useRef<number | null>(null);
  frozenHour.current = activeFrozen ? activeFrozen.hour : null;
  const shownAt = useCallback(() => frozenHour.current ?? head.shownAt(), [head.shownAt]);

  return {
    pages, onWantedPages, ready: !!head.data, at: activeFrozen ? activeFrozen.hour : head.at,
    shownAt, live: head.live,
    missing: activeFrozen ? activeFrozen.missing : (head.data?.missing ?? NO_MISSING),
    error: head.error ?? pageError, retry, root: head.root,
  };
}
