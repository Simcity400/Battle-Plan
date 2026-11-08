import { type Failure } from '../../dashboard/queryError.ts';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { LogPage, LogRow } from '../../charts/log/logData';
import { stableLogPages, useEstimatedPages, withContinuation } from '../../charts/log/logPages';
import { playbackRows, retainPlaybackMap } from '../../charts/playbackIntervals.ts';
import type { PlaybackBounds } from '../../charts/playbackIntervals';
import type { RecordingBreak } from './scriptOccurrences';
import { useContentRevision } from '../../dashboard/contentRevision';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { useProductWindows } from '../../dashboard/productWindows';
import { useFollowedProduct } from '../../dashboard/products';
import type { EventFeedKind, ScriptOccurrenceReply } from '../../generated/dashboard';
import { validateScriptPage } from './scriptOccurrences';
import type { ScriptPage } from './scriptOccurrences';

/** `tally` is the page's summary count once measured; `final` marks the chain's last page;
 * `cards` counts its prepared cards before the hour's visibility cut; `valid` is the cut's span
 * over which `count` and `tally` hold, null until counted. */
interface PageInfo { cursor: string | null; start: number; end: number; count: number; tally: number | null; final: boolean; cards: number | null;
  valid: PlaybackBounds | null }
const NO_WARNINGS: string[] = [];
const NO_ROWS: ReadonlyArray<LogRow<never>> = [];
const PAGE_SIZE = 256;
/** The page a new hour asks first, as the event feed's (`FIRST_PAGE_SIZE` there). */
const FIRST_PAGE_SIZE = 64;
/** A page counted under another visibility: estimated by its cards, its tally unknown, until read again. */
const recount = (info: PageInfo): PageInfo => ({ ...info, count: info.cards ?? info.count, tally: null, valid: null });
/** Whether a page's count holds at `cut`: counted under it, at a cut its rows show the same at. */
const holds = (info: PageInfo, cut: number | null) => info.valid !== null && cut !== null && info.valid.first <= cut && cut < info.valid.end;

/** Event choices' and Decisions' occurrence pages, read as the event feed reads
 * its own: native dates page backward while every cursor keeps the same Live
 * read bound, only demanded payloads stay held, and small cursor/count
 * descriptors survive eviction. `prepare` filters and orders a page's cards
 * once; `at` cuts each prepared card at the playback hour (null hides it);
 * `tally` counts a page's shown cards for the header summary. */
export function useScriptOccurrences<T>(runtime: WidgetRuntime, track: BoundTrack | undefined, active: boolean, hour: number | null,
  product: 'event-choices' | 'decisions', kinds: readonly EventFeedKind[], firstHour: number | null, filterKey: string,
  prepare: (reply: ScriptOccurrenceReply) => ReadonlyArray<LogRow<T>>, at: (row: LogRow<T>, hour: number) => LogRow<T> | null,
  tally: (rows: readonly LogRow<T>[]) => number, breaks: ReadonlyArray<RecordingBreak>,
  bounds: (rows: ReadonlyArray<LogRow<T>>, hour: number) => PlaybackBounds) {
  const content = useContentRevision(runtime.client);
  const identity = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, content]) : null;
  const request = (cursor: string | null, lastHour: number | null, pageSize = PAGE_SIZE) => ({ sourceId: track!.source.id, revision: track!.source.revision, product,
    params: { branchId: track!.branch.id, firstHour, lastHour, cursor, pageSize, kinds: [...kinds] } });
  const first = useFollowedProduct<ScriptPage>(runtime.client, track && hour !== null ? request(null, hour, FIRST_PAGE_SIZE) : null, ['lastHour'], active, validateScriptPage, track?.source);
  // The page chain follows the hour the first page answers.
  const read = first.data && first.moment ? (JSON.parse(first.moment) as [number | null])[0] : hour;
  // A first page answered from the prepared prefix reads back from the prefix's
  // last hour, and its continuations keep that hour: a grown prefix starts the
  // chain again.
  const live = first.data?.prefix?.pending.length ? first.data.prefix.coverage.lastHour : null;
  const boundsKey = JSON.stringify([identity, product, filterKey, read, live]);
  const owner = useMemo(() => ({ client: runtime.client, boundsKey, prepare }), [runtime.client, boundsKey, prepare]);
  const current = useRef(owner); current.current = owner;
  // `shown` is the visibility cut the pages were counted under.
  const [chain, setChain] = useState<{ owner: typeof owner; pages: PageInfo[]; error?: Failure; shown?: typeof at }>({ owner, pages: [] });
  const initial: PageInfo = { cursor: null, start: Math.max(firstHour ?? track?.branch.firstHour ?? 0, track?.branch.firstHour ?? 0), end: read ?? 0,
    count: FIRST_PAGE_SIZE, tally: null, final: false, cards: null, valid: null };
  const kept = chain.owner === owner && chain.pages.length ? chain.pages : null;
  // Cards stop where the shown pages were read, which a page kept while the next hour loads bounds.
  const cut = hour === null ? null : read === null ? hour : Math.min(read, hour);
  // A page counted at a cut whose cards show otherwise is estimated until it is read again; its count is kept for a return.
  const described = useEstimatedPages(kept, info => chain.shown === at && (info.valid === null || holds(info, cut)), recount) ?? [initial];
  const [wanted, setWanted] = useState<{ owner: typeof owner; pages: readonly string[] }>({ owner, pages: [] });
  const chosen = wanted.owner === owner ? wanted.pages : ['0'];
  const indices = [...new Set(chosen.flatMap(page => Number.isInteger(Number(page)) && Number(page) > 0
    && Number(page) < described.length ? [Number(page)] : []))];
  const chunks = useProductWindows<ScriptPage>(runtime.client, boundsKey,
    index => request(described[index].cursor, read), indices, active && !!first.data, validateScriptPage, track?.source);
  const loaded = useMemo(() => new Map<number, ScriptPage>([...chunks.data, ...(first.data ? [[0, first.data] as const] : [])]), [chunks.data, first.data]);
  const prepared = useMemo(() => new Map([...loaded].map(([index, page]) => [index, prepare(page.reply)])), [loaded, prepare]);
  const descriptors = withContinuation(described, loaded, (page, index) => ({ cursor: page.nextCursor, start: initial.start,
    end: Math.min(...page.reply.occurrences.map(occurrence => occurrence.edges[0].row.hour), described[index].end),
    count: PAGE_SIZE, tally: null, final: false, cards: null, valid: null }));
  const playbacks = useMemo(() => new Map([...prepared].map(([index, rows]) => {
    const changes = [...breaks.map(span => span.firstHour), ...loaded.get(index)!.reply.occurrences.flatMap(occurrence => occurrence.edges.map(edge => edge.row.readHour))];
    return [index, playbackRows<LogRow<T>>(changes, hour => rows.flatMap(row => {
      const shown = at(row, hour);
      return shown ? [{ ...shown, key: JSON.stringify([index, row.key]), id: JSON.stringify([index, row.id ?? row.key]) }] : [];
    }), bounds)] as const;
  })), [prepared, loaded, at, breaks, bounds]);
  const renderedBefore = useRef<ReadonlyMap<number, ReadonlyArray<LogRow<T>>>>(new Map());
  const rendered = useMemo(() => renderedBefore.current = retainPlaybackMap(renderedBefore.current,
    new Map([...playbacks].map(([index, playback]) => [index, cut === null ? NO_ROWS : playback.at(cut)]))), [playbacks, cut]);
  useEffect(() => {
    if (!active || !first.data) return;
    setChain(previous => {
      if (current.current !== owner) return previous;
      const same = previous.owner === owner && previous.pages.length > 0;
      const pages = !same ? [initial] : previous.shown === at ? [...previous.pages] : previous.pages.map(recount);
      let changed = !same || previous.shown !== at;
      let error: Failure | undefined;
      for (const [index, page] of loaded) {
        const rows = rendered.get(index)!;
        const info = pages[index];
        if (!info) continue;
        if (page.nextCursor !== null && pages.slice(0, index + 1).some(info => info.cursor === page.nextCursor)) {
          error = new Error('The occurrence list continuation made no progress.');
          break;
        }
        const hours = page.reply.occurrences.map(occurrence => occurrence.edges[0].row.hour);
        const start = Math.min(...hours, info.end);
        const end = Math.max(...hours, start);
        const counted = tally(rows);
        const final = page.nextCursor === null;
        const cards = prepared.get(index)!.length;
        const valid = cut === null ? null : playbacks.get(index)!.span(cut);
        if (info.count !== rows.length || info.start !== start || info.end !== end || info.tally !== counted || info.final !== final || info.cards !== cards
          || info.valid?.first !== valid?.first || info.valid?.end !== valid?.end) {
          pages[index] = { ...info, start, end, count: rows.length, tally: counted, final, cards, valid }; changed = true;
        }
        if (index === pages.length - 1 && page.nextCursor !== null) {
          pages.push({ cursor: page.nextCursor, start: initial.start, end: start, count: PAGE_SIZE, tally: null, final: false, cards: null, valid: null }); changed = true;
        }
      }
      return changed || previous.error !== error ? { owner, pages, error, shown: at } : previous;
    });
    // `cut` is read for each page's span only when its rows change, which is where a span ends.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [owner, descriptors, loaded, prepared, rendered, at, active, first.data, tally]);
  const committedPages = useRef<{ owner: typeof owner; pages: readonly LogPage<T>[] }>({ owner, pages: [] });
  const pages = useMemo(() => stableLogPages(committedPages.current.owner === owner ? committedPages.current.pages : [], !first.data ? []
    : descriptors.map((info, index) => ({ key: String(index), start: info.start, end: info.end,
      count: rendered.get(index)?.length ?? info.count, rows: rendered.get(index), error: chunks.errors.get(index) }))),
  [owner, descriptors, first.data, rendered, chunks.errors]);
  useLayoutEffect(() => { committedPages.current = { owner, pages }; }, [owner, pages]);
  const onWantedPages = useCallback((pages: readonly string[]) => {
    if (current.current !== owner) return;
    setWanted(previous => previous.owner === owner && previous.pages.join(',') === pages.join(',') ? previous : { owner, pages });
  }, [owner]);
  const retry = useCallback(() => { first.retry(); chunks.retry(); }, [first.retry, chunks.retry]);
  // Every page discovered and counted: the shown total is known, else null.
  const counted = chain.owner === owner && descriptors.at(-1)!.final && descriptors.every(info => info.tally !== null);
  const total = counted ? descriptors.reduce((sum, info) => sum + info.tally!, 0) : null;
  const replies = useMemo(() => [...loaded.values()].map(page => page.reply), [loaded]);
  return { data: first.data?.reply, shownHour: cut, prefix: first.data?.prefix ?? null, error: first.error ?? (chain.owner === owner ? chain.error : undefined), retry,
    identity, pages, total, replies, warnings: first.data?.reply.warnings ?? NO_WARNINGS, onWantedPages, errors: chunks.errors, retryPages: chunks.retry };
}
