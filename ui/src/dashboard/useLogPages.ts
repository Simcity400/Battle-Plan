import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { LogPage, LogRow } from '../charts/log/logData';
import { stableLogPages } from '../charts/log/logPages.ts';
import { aheadThrough, askedPages, binPages, cumulativeCounts, drawnAhead, failedPage, logPageRequest, noPages, pageFirstHour, pageInSpan, pagesOf, rowsAhead, shownPages, shownThrough, validateLogCounts, validateLogPage } from '../charts/log/logSource.ts';
import type { BoundFacetFilter, HeldPages, LogCounts, LogReplyRow } from '../charts/log/logSource';
import { AHEAD_ROWS, aheadEnd } from '../charts/reveal.ts';
import { useAheadHour } from './aheadHour.ts';
import type { MembershipRun, MissingSpan } from '../generated/dashboard';
import type { DashboardClient, DashboardSource } from './client';
import { inFrame } from './frameUpdates.ts';
import type { PlaybackController } from './playback';
import { loadProduct, useProduct } from './products.ts';
import { asFailure } from './queryError.ts';
import type { BoundHour } from './regionTime';
import { productRequest } from './request.ts';
import { forwardRequest, type DeclaredProduct } from './request.ts';
import type { BoundWindow } from './regionTime';

export interface LogSpec {
  source: DashboardSource;
  branchId: string;
  log: string;
  facets?: readonly BoundFacetFilter[];
  scope?: readonly (MembershipRun & BoundWindow)[];
  firstHour: BoundHour;
  lastHour: BoundHour;
  product?: DeclaredProduct;
}

export type LogCut = { follow: true; offset: number } | { follow: false; hour: number | null };

export const LOG_PAGE_SIZE = 500;
const NO_MISSING: readonly MissingSpan[] = [];
const COUNT_BINS = 4096;
export const countBinHours = (firstHour: number, lastHour: number) => 6 * Math.max(1, Math.ceil((lastHour - firstHour + 1) / (6 * COUNT_BINS)));

export function useLogPages<T, N = never>(client: DashboardClient, clock: PlaybackController, spec: LogSpec | null, cut: LogCut, active: boolean,
  row: (reply: LogReplyRow) => LogRow<T>, pageSize = LOG_PAGE_SIZE, notes?: (page: unknown) => readonly N[]) {
  const lane = spec ? JSON.stringify([spec.source.id, spec.source.revision, spec.branchId, spec.log, spec.product ?? null, spec.facets ?? null, spec.scope ?? null, spec.firstHour, spec.lastHour, pageSize]) : null;
  const validateCounts = useCallback((value: unknown) => validateLogCounts(value, spec?.firstHour ?? 0, spec?.lastHour ?? -1), [spec?.firstHour, spec?.lastHour]);
  const counted = useProduct<LogCounts>(client, spec ? productRequest({ sourceId: spec.source.id, revision: spec.source.revision, product: 'log-counts',
    params: { branchId: spec.branchId, log: spec.log, ...(spec.facets ? { facets: spec.facets } : {}), ...(spec.scope ? { scope: spec.scope } : {}), from: spec.firstHour,
      to: spec.lastHour, binHours: countBinHours(spec.firstHour, spec.lastHour) } }) : null, active, validateCounts, spec?.source);
  const counts = counted.data;
  const before = useMemo(() => counts ? cumulativeCounts(counts) : null, [counts]);
  const [held, setHeld] = useState<Held<T, N>>(() => noPages(lane));
  const stash = useRef<Stash<T, N>>({ lane, rows: new Map(), notes: new Map(), version: 0, failed: new Set() });
  if (stash.current.lane !== lane) stash.current = { lane, rows: new Map(), notes: new Map(), version: 0, failed: new Set() };
  const merged = useRef<{ held: Held<T, N>; version: number; lane: string | null; loaded: Held<T, N> } | null>(null);
  if (!merged.current || merged.current.held !== held || merged.current.version !== stash.current.version || merged.current.lane !== lane) {
    const base = pagesOf(held, lane), waiting = stash.current;
    merged.current = { held, version: waiting.version, lane, loaded: waiting.rows.size ? withStash(base, waiting) : base };
  }
  const loaded = merged.current.loaded;
  const latest = useRef({ spec, row, notes, loaded, counts, before, cut, active, drawn: 0, shown: 0, playing: false, at: null as number | null });
  latest.current = { ...latest.current, spec, row, notes, loaded, counts, before, cut, active };
  const flights = useRef(new Map<number, AbortController>());
  const [, setLanded] = useState(0);
  const gone = useRef(false);
  useEffect(() => { gone.current = false; return () => { gone.current = true; }; }, []);
  const wanted = useRef<ReadonlySet<number>>(new Set());

  const shownAt = useCallback((hour: number | null) => {
    const { counts: c, before: b, loaded: l } = latest.current;
    if (!c || !b) return { shown: 0, known: false };
    return shownThrough(c, b, hour, pageSize, index => l.rows.get(index));
  }, [pageSize]);
  const following = active && cut.follow && !!counts;
  const loadedRows = loaded.rows;
  const offset = cut.follow ? cut.offset : 0;
  const end = useCallback((clockHour: number) => {
    const at = Math.floor(clockHour) + offset;
    if (!counts || !before) return clockHour + 1;
    const page = (index: number) => loadedRows.get(index);
    const head = shownThrough(counts, before, at, pageSize, page);
    if (!head.known) return clockHour + 1;
    const ahead = rowsAhead(head.shown, counts.total, pageSize, AHEAD_ROWS + 1, page);
    const { hoursPerSecond, playing } = clock.getSnapshot();
    return aheadEnd(ahead.hours, at, hoursPerSecond, playing, ahead.hours.length, ahead.complete) - offset;
  }, [clock, offset, pageSize, counts, before, loadedRows]);
  const block = useAheadHour(clock, following, end);
  const head = shownAt(cut.follow ? Math.floor(block.hour) + offset : cut.hour);
  const known = String(head.known);
  const drawnRows = following && head.known && counts
    ? head.shown + rowsAhead(head.shown, counts.total, pageSize, AHEAD_ROWS, index => loaded.rows.get(index)).hours
      .filter(hour => hour < block.until + offset).length
    : head.shown;
  const shownRows = head.shown;
  latest.current.drawn = drawnRows;
  latest.current.shown = shownRows;
  latest.current.playing = following && head.known && clock.getSnapshot().playing;
  latest.current.at = cut.follow ? Math.floor(block.hour) + offset : cut.hour;

  const load = useCallback((index: number) => {
    const { spec: held, counts: base, active: shown } = latest.current;
    if (gone.current || !shown || !held || !base || !pageInSpan(index, pageSize, base.total) || flights.current.has(index) || latest.current.loaded.rows.has(index)
      || stash.current.rows.has(index) || stash.current.failed.has(index) || latest.current.loaded.errors.has(index)) return;
    const controller = new AbortController();
    flights.current.set(index, controller);
    const at = index * pageSize, asked = lane;
    const cutHour = latest.current.at, first = pageFirstHour(base, latest.current.before ?? cumulativeCounts(base), at);
    const asksFrom = cutHour !== null && at >= latest.current.shown ? Math.max(first, cutHour + 1) : first;
    void loadProduct<unknown>(client, (({ product, params }) => forwardRequest(held.source, product, params, asksFrom))(logPageRequest(held, base.before + at, pageSize)),
      controller.signal, undefined, undefined, false, undefined, undefined, true).then(value => {
      const page = validateLogPage(value, at, pageSize, base.before);
      const rows = page.rows.map(latest.current.row);
      const said = latest.current.notes?.(value);
      const now = latest.current, waiting = stash.current;
      if (waiting.lane !== asked || now.loaded.lane !== asked) return;
      waiting.rows.set(index, rows);
      if (said) waiting.notes.set(index, said);
      waiting.version++;
      if (!(now.playing && at >= now.drawn)) inFrame(() => setLanded(count => count + 1));
    }).catch(error => {
      if (controller.signal.aborted) return;
      if (stash.current.lane === asked) stash.current.failed.add(index);
      inFrame(() => setHeld(before => failedPage(before, latest.current.loaded.lane, asked, index, asFailure(error))));
    }).finally(() => { if (flights.current.get(index) === controller) flights.current.delete(index); });
  }, [client, lane, pageSize]);

  const keep = useCallback((asked: ReadonlySet<number>) => {
    wanted.current = asked;
    const { cut: at, shown } = latest.current, head = Math.floor(shown / pageSize), limit = shown + AHEAD_ROWS + 1;
    const kept = (index: number) => asked.has(index) || (at.follow && index >= head && index * pageSize < limit);
    for (const [index, controller] of flights.current) if (!kept(index)) { controller.abort(); flights.current.delete(index); }
    for (const index of asked) load(index);
    const waiting = stash.current;
    for (const index of [...waiting.rows.keys()]) if (!kept(index)) { waiting.rows.delete(index); waiting.notes.delete(index); waiting.version++; }
    const held = pagesOf(merged.current?.held ?? latest.current.loaded, lane);
    if ([...held.rows.keys()].some(index => !kept(index))) {
      setHeld(before => before.lane !== lane ? before : { lane, rows: new Map([...before.rows].filter(([index]) => kept(index))),
        notes: new Map([...before.notes].filter(([index]) => kept(index))), errors: before.errors });
    }
  }, [load, lane, pageSize]);
  const waiting = useCallback(() => {
    const { counts: c, before: b, cut: at } = latest.current;
    const hour = at.follow ? Math.floor(clock.getHour()) + at.offset : at.hour;
    return !c || !b || shownAt(hour).known ? [] : binPages(c, b, hour, pageSize);
  }, [clock, shownAt, pageSize]);
  const onWantedPages = useCallback((keys: readonly string[]) => {
    const { counts: c, before: b, cut: at, at: hour } = latest.current;
    const { hoursPerSecond, playing } = clock.getSnapshot();
    const through = c && b && at.follow && hour !== null ? aheadThrough(c, b, shownRows, hour, hoursPerSecond, playing) : shownRows + 1;
    keep(askedPages(keys, shownRows, pageSize, at.follow, waiting(), through));
  }, [keep, shownRows, pageSize, waiting, clock]);
  useEffect(() => {
    if (!active || known === 'true') return;
    for (const index of waiting()) load(index);
  });
  useEffect(() => () => { for (const controller of flights.current.values()) controller.abort(); flights.current.clear(); }, [lane]);
  const [retried, setRetried] = useState(0);
  useEffect(() => { if (counts && active) keep(wanted.current); }, [counts, active, keep, retried]);

  const previous = useRef<readonly LogPage<T>[]>([]);
  const pages = useMemo(() => previous.current = stableLogPages(previous.current, shownPages(drawnRows, pageSize, loaded.rows, loaded.errors)),
    [drawnRows, pageSize, loaded.rows, loaded.errors]);
  const pageNotes = useMemo(() => [...loaded.notes].sort(([a], [b]) => a - b).flatMap(([, said]) => said), [loaded.notes]);
  const ahead = useMemo(() => drawnAhead(pages, shownRows), [pages, shownRows]);
  const retry = () => {
    counted.retry();
    stash.current.failed.clear();
    setHeld(before => ({ ...before, errors: new Map() }));
    setRetried(count => count + 1);
  };
  const missing = counts?.missing ?? NO_MISSING;
  const through = cut.follow ? (following ? block.until - 1 : Math.floor(block.hour)) + offset : cut.hour;
  const at = cut.follow ? Math.floor(block.hour) + offset : cut.hour;
  const cutFailure = !head.known && counts && before ? binPages(counts, before, at, pageSize).map(index => loaded.errors.get(index)).find(Boolean) : undefined;
  const pageFailure = pages.find(page => page.error)?.error;
  return { pages, onWantedPages, counts, shown: shownRows, at, ahead, through, total: counts?.total, missing, notes: pageNotes, error: counted.error ?? cutFailure ?? pageFailure, retry };
}

type Held<T, N> = HeldPages<readonly LogRow<T>[], N>;
interface Stash<T, N> { lane: string | null; rows: Map<number, readonly LogRow<T>[]>; notes: Map<number, readonly N[]>; version: number; failed: Set<number> }

function withStash<T, N>(held: Held<T, N>, stash: Stash<T, N>): Held<T, N> {
  const rows = new Map(held.rows), notes = new Map(held.notes);
  for (const [index, page] of stash.rows) if (!rows.has(index)) rows.set(index, page);
  for (const [index, said] of stash.notes) if (!notes.has(index)) notes.set(index, said);
  return { ...held, rows, notes };
}
