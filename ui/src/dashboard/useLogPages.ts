import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { LogPage, LogRow } from '../charts/log/logData';
import { stableLogPages } from '../charts/log/logPages.ts';
import { aheadLimit, aheadThrough, askedPages, binPages, cumulativeCounts, drawnAhead, failedPage, logPageRequest, noPages, pageFirstHour, pageInSpan, pagesOf, rowsAhead, shownPages, shownThrough, validateLogCounts, validateLogPage } from '../charts/log/logSource.ts';
import type { HeldPages, LogCounts, LogReplyRow } from '../charts/log/logSource';
import { AHEAD_ROWS, aheadEnd } from '../charts/reveal.ts';
import { useAheadHour } from './aheadHour.ts';
import type { FacetFilter, MembershipRun, MissingSpan } from '../generated/dashboard';
import type { DashboardClient, DashboardSource } from './client';
import { inFrame } from './frameUpdates.ts';
import type { PlaybackController } from './playback';
import { loadProduct, useProduct } from './products.ts';
import { asFailure } from './queryError.ts';
import { StructurePace } from './structurePace.ts';

/** A declared log over a span: the recording, the history, the log as `LOGS` names it, its facet filters (by values,
 * or a country facet by tags) and the span's hours. */
export interface LogSpec {
  source: DashboardSource;
  branchId: string;
  log: string;
  facets?: readonly FacetFilter[];
  /** The countries a row must name, each tag over the hours it counts (`countryScope`), kept by the server so the pages
   * and counts hold only those rows; every row when absent. Only a log the server can scope takes it (the feed's). */
  scope?: readonly MembershipRun[];
  firstHour: number;
  lastHour: number;
  /** The product serving its pages: `log-page`, or a family's own page of the same shape that names its rows. */
  product?: string;
}

/** Where the list is cut: at the clock's head while following (`offset` maps the clock's hour to the track's), or at
 * a fixed hour (a pinned or ranged widget; null shows every row of the span). */
export type LogCut = { follow: true; offset: number } | { follow: false; hour: number | null };

/** Rows per page, the most a page asks; a list asks only the pages in view, and following, the page at the head and
 * the next. */
export const LOG_PAGE_SIZE = 500;
const NO_MISSING: readonly MissingSpan[] = [];
/** The counts the cut is read from come in the store's six-hour buckets, grouped so a span has at most this many
 * bins: the reply is the same size however long the span, and the cut inside a bin reads that bin's rows. */
const COUNT_BINS = 4096;
export const countBinHours = (firstHour: number, lastHour: number) => 6 * Math.max(1, Math.ceil((lastHour - firstHour + 1) / (6 * COUNT_BINS)));

/** A paged declared log for `LogList`: counted once per span (`log-counts`), its pages asked as the list wants them
 * (`log-page` from a cursor, `pageSize` rows), each a fixed window of ordinals counted from the oldest row so a page
 * keeps its key and rows as the head moves. The head is a cut over rows already held: while playing it asks no
 * page but the next one ahead, one request per `pageSize` rows crossed. Following, its pages hold a block of rows past
 * the head (`useAheadHour`), which the list hides until its reveal clock reaches them (`LogList`'s `reveal`, which
 * every list of these pages takes): it renders once a block, not when a row is crossed. `row` maps a reply row (typed cells by
 * column) to the list's row; `notes` takes what a family's page says beside its rows (its notices), returned for
 * every page held, in page order. */
export function useLogPages<T, N = never>(client: DashboardClient, clock: PlaybackController, spec: LogSpec | null, cut: LogCut, active: boolean,
  row: (reply: LogReplyRow) => LogRow<T>, pageSize = LOG_PAGE_SIZE, notes?: (page: unknown) => readonly N[]) {
  const lane = spec ? JSON.stringify([spec.source.id, spec.source.revision, spec.branchId, spec.log, spec.product ?? null, spec.facets ?? null, spec.scope ?? null, spec.firstHour, spec.lastHour, pageSize]) : null;
  const validateCounts = useCallback((value: unknown) => validateLogCounts(value, spec?.firstHour ?? 0, spec?.lastHour ?? -1), [spec?.firstHour, spec?.lastHour]);
  const counted = useProduct<LogCounts>(client, spec ? { sourceId: spec.source.id, revision: spec.source.revision, product: 'log-counts',
    params: { branchId: spec.branchId, log: spec.log, ...(spec.facets ? { facets: spec.facets } : {}), ...(spec.scope ? { scope: spec.scope } : {}), from: spec.firstHour,
      to: spec.lastHour, binHours: countBinHours(spec.firstHour, spec.lastHour) } } : null, active, validateCounts, spec?.source);
  const counts = counted.data;
  const before = useMemo(() => counts ? cumulativeCounts(counts) : null, [counts]);
  const [held, setHeld] = useState<Held<T, N>>(() => noPages(lane));
  // Every page that lands, held here at once; the next render takes them. One past the rows drawn while playing changes
  // nothing shown and asks no render: the block that draws it takes it (`held` keeps failures and what was let go).
  const stash = useRef<Stash<T, N>>({ lane, rows: new Map(), notes: new Map(), version: 0, failed: new Set() });
  if (stash.current.lane !== lane) stash.current = { lane, rows: new Map(), notes: new Map(), version: 0, failed: new Set() };
  const merged = useRef<{ held: Held<T, N>; version: number; lane: string | null; loaded: Held<T, N> } | null>(null);
  const base = pagesOf(held, lane);
  if (!merged.current || merged.current.held !== base || merged.current.version !== stash.current.version || merged.current.lane !== lane) {
    const waiting = stash.current;
    merged.current = { held: base, version: waiting.version, lane, loaded: waiting.rows.size ? withStash(base, waiting) : base };
  }
  const loaded = merged.current.loaded;
  const latest = useRef({ spec, row, notes, loaded, counts, before, cut, active, drawn: 0, shown: 0, playing: false, at: null as number | null });
  latest.current = { ...latest.current, spec, row, notes, loaded, counts, before, cut, active };
  const flights = useRef(new Map<number, AbortController>());
  // Following and playing, what changes the pages held (one landing at the head, one failing, pages let go) commits in the
  // widget's structure slot with the rest of its structure (`StructurePace`), every change held for the slot in one
  // commit; paused, in the frame (`inFrame`). A page landing past the rows drawn commits nothing (`stash`).
  const [, setSlot] = useState(0);
  const [, setLanded] = useState(0);
  const queued = useRef<(() => void)[]>([]);
  const pace = useRef<StructurePace<number> | null>(null);
  pace.current ??= new StructurePace<number>(0, Object.is, value => { for (const run of queued.current.splice(0)) run(); setSlot(value); });
  // Gone, it asks and takes nothing: its list lets its pages go as it leaves, after its flights were cancelled here.
  const gone = useRef(false);
  useEffect(() => { gone.current = false; return () => { gone.current = true; }; }, []);
  const paced = useCallback((update: () => void) => {
    if (gone.current) return;
    if (!clock.getSnapshot().playing) { inFrame(update); return; }
    queued.current.push(update);
    pace.current!.offer(pace.current!.held + 1, true);
  }, [clock]);
  useEffect(() => {
    const held = pace.current!;
    const stop = clock.subscribe(() => { if (!clock.getSnapshot().playing) held.settle(); });
    return () => { stop(); held.drop(); };
  }, [clock]);
  const wanted = useRef<ReadonlySet<number>>(new Set());

  const shownAt = useCallback((hour: number | null) => {
    const { counts: c, before: b, loaded: l } = latest.current;
    if (!c || !b) return { shown: 0, known: false };
    return shownThrough(c, b, hour, pageSize, index => l.rows.get(index));
  }, [pageSize]);
  const following = active && cut.follow && !!counts;
  // Following, the list renders a block of rows at once: those at the head and, playing, the next rows the loaded pages
  // hold (`aheadEnd`), which it reveals each frame by the clock (`LogList`'s `reveal`). It renders again only when the
  // clock leaves the block, never because a row inside it was reached; paused, when the head crosses a row.
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
  // The rows drawn: following, through the block's end (rows the pages hold without a gap); else those at the cut.
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
    // A page's cursor counts from the timeline's first row: it waits for the rows before the span. Off screen, nothing is
    // asked, though the counts the prepare lane holds show. A page that failed waits for a retry.
    if (gone.current || !shown || !held || !base || !pageInSpan(index, pageSize, base.total) || flights.current.has(index) || latest.current.loaded.rows.has(index)
      || stash.current.rows.has(index) || stash.current.failed.has(index) || latest.current.loaded.errors.has(index)) return;
    const controller = new AbortController();
    flights.current.set(index, controller);
    const at = index * pageSize, asked = lane;
    // A page asks from the first hour its first row can hold; one wholly past the rows through the cut, from after the
    // cut's hour too: a block ahead of the head, each page from its own hour.
    const cutHour = latest.current.at, first = pageFirstHour(base, latest.current.before ?? cumulativeCounts(base), at);
    const asksFrom = cutHour !== null && at >= latest.current.shown ? Math.max(first, cutHour + 1) : first;
    // A page is its rows from a cursor, the same at every hour: held with the hours' replies, so a remount, a setting put
    // back or the prepare lane reads it once.
    void loadProduct<unknown>(client, { sourceId: held.source.id, revision: held.source.revision, ...logPageRequest(held, base.before + at, pageSize), asksFrom },
      controller.signal, undefined, undefined, false, undefined, undefined, true).then(value => {
      const page = validateLogPage(value, at, pageSize, base.before);
      const rows = page.rows.map(latest.current.row);
      const said = latest.current.notes?.(value);
      const now = latest.current, waiting = stash.current;
      if (waiting.lane !== asked || now.loaded.lane !== asked) return;
      // Held at once, so no want between its landing and its render asks it again; a page past the rows drawn waits there
      // for the block that draws them, one the list shows renders in the slot (`paced`).
      waiting.rows.set(index, rows);
      if (said) waiting.notes.set(index, said);
      waiting.version++;
      if (!(now.playing && at >= now.drawn)) paced(() => setLanded(count => count + 1));
    }).catch(error => {
      if (controller.signal.aborted) return;
      if (stash.current.lane === asked) stash.current.failed.add(index);
      paced(() => setHeld(before => failedPage(before, latest.current.loaded.lane, asked, index, asFailure(error))));
    }).finally(() => { if (flights.current.get(index) === controller) flights.current.delete(index); });
  }, [client, lane, pageSize, paced]);

  // The pages wanted: those the list holds in view and, following, the page at the head and those ahead; others go. A
  // page from the head's on that a list following may still draw is never cancelled nor let go by a want that leaves it
  // out (one told with an older block's rows): it would be asked again at the head.
  const keep = useCallback((asked: ReadonlySet<number>) => {
    wanted.current = asked;
    const { cut: at, shown } = latest.current, head = Math.floor(shown / pageSize), limit = aheadLimit(shown);
    const kept = (index: number) => asked.has(index) || (at.follow && index >= head && index * pageSize < limit);
    for (const [index, controller] of flights.current) if (!kept(index)) { controller.abort(); flights.current.delete(index); }
    for (const index of asked) load(index);
    const waiting = stash.current;
    for (const index of [...waiting.rows.keys()]) if (!kept(index)) { waiting.rows.delete(index); waiting.notes.delete(index); waiting.version++; }
    const held = merged.current?.held ?? latest.current.loaded;
    if ([...held.rows.keys()].some(index => !kept(index))) {
      const release = () => setHeld(before => before.lane !== lane ? before : { lane, rows: new Map([...before.rows].filter(([index]) => kept(index))),
        notes: new Map([...before.notes].filter(([index]) => kept(index))), errors: before.errors });
      if (clock.getSnapshot().playing) paced(release); else release();
    }
  }, [load, lane, pageSize, paced, clock]);
  // The pages the cut waits on (its bin's, while a page of it is not held), asked at once and never released by the list.
  const waiting = useCallback(() => {
    const { counts: c, before: b, cut: at } = latest.current;
    const hour = at.follow ? Math.floor(clock.getHour()) + at.offset : at.hour;
    return !c || !b || shownAt(hour).known ? [] : binPages(c, b, hour, pageSize);
  }, [clock, shownAt, pageSize]);
  // Following, the pages of the rows the next blocks draw are asked with this block, never as the clock reaches them.
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
  // Pages wanted before the counts landed, or while off screen, are asked once they have and it is shown; after a retry,
  // once the failures it cleared are gone.
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
  // The hours the log was not recorded whole, with their reasons: never shown as hours without rows.
  const missing = counts?.missing ?? NO_MISSING;
  // The last hour of the track whose rows are drawn: following, the block's last (rows past the clock wait hidden for the
  // list's reveal clock); else the cut's.
  const through = cut.follow ? (following ? block.until - 1 : Math.floor(block.hour)) + offset : cut.hour;
  // `shown` counts the rows through `at`; following, `ahead` are the rows drawn past it, oldest first, which a count
  // painted each frame adds as the clock reaches them (`countsAhead`).
  const at = cut.follow ? Math.floor(block.hour) + offset : cut.hour;
  // A page of the cut's bin that failed leaves the rows through the cut unknown: said, with a retry, never as no rows.
  const cutFailure = !head.known && counts && before ? binPages(counts, before, at, pageSize).map(index => loaded.errors.get(index)).find(Boolean) : undefined;
  // A drawn page that failed is asked again only by a retry: said with one.
  const pageFailure = pages.find(page => page.error)?.error;
  return { pages, onWantedPages, counts, shown: shownRows, at, ahead, through, total: counts?.total, missing, notes: pageNotes, error: counted.error ?? cutFailure ?? pageFailure, retry };
}

type Held<T, N> = HeldPages<readonly LogRow<T>[], N>;
interface Stash<T, N> { lane: string | null; rows: Map<number, readonly LogRow<T>[]>; notes: Map<number, readonly N[]>; version: number; failed: Set<number> }

/** `held` with the pages waiting in `stash` (each a page `held` has not). */
function withStash<T, N>(held: Held<T, N>, stash: Stash<T, N>): Held<T, N> {
  const rows = new Map(held.rows), notes = new Map(held.notes);
  for (const [index, page] of stash.rows) if (!rows.has(index)) rows.set(index, page);
  for (const [index, said] of stash.notes) if (!notes.has(index)) notes.set(index, said);
  return { ...held, rows, notes };
}
