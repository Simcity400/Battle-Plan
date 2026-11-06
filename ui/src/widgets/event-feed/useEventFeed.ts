import { type Failure } from '../../dashboard/queryError.ts';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { LogPage } from '../../charts/log/logPages';
import { LogPlayback } from '../../charts/log/logPlayback.ts';
import { retainPlaybackMap } from '../../charts/playbackIntervals.ts';
import type { LogRow } from '../../charts/log/logData';
import { stableLogPages } from '../../charts/log/logPages';
import { useContentRevision } from '../../dashboard/contentRevision';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { useProductWindows } from '../../dashboard/productWindows';
import { useFollowedProduct } from '../../dashboard/products';
import { feedNotes, filteredEventRows, validateEventFeedPage } from './eventFeedData';
import type { FeedEvent, FeedFilter, PreparedFeedPage } from './eventFeedData';

interface PageInfo { cursor: string | null; start: number; end: number; count: number }
const NO_WARNINGS: string[] = [];
const PAGE_SIZE = 256;
/** The page a new hour asks first: more rows than a tall log shows, so the hour answers without
 * presenting rows nobody sees yet; older rows follow in whole pages as the log is scrolled. */
const FIRST_PAGE_SIZE = 64;

/** The first page a following feed holds: a whole page asked at an hour, and the rows read
 * after it, newest first, each a reply of its own whose items and texts its rows name. */
interface HeldFeed {
  lane: string;
  base: PreparedFeedPage;
  /** The hour the whole page was asked at, which its continuations keep. */
  baseHour: number;
  added: readonly PreparedFeedPage[];
  /** The rows `added` holds. */
  count: number;
  /** The newest reply: the hour the page is read through, and its prefix. */
  latest: PreparedFeedPage;
}

/** Native dates page backward while every cursor retains the same Live read bound.
 * Only demanded payloads stay held; small cursor/count descriptors survive eviction. */
export function useEventFeed(runtime: WidgetRuntime, track: BoundTrack | undefined, active: boolean,
  hour: number | null, filter: FeedFilter, filterKey: string, prepareAll = false) {
  const content = useContentRevision(runtime.client);
  const identity = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, content]) : null;
  const request = (cursor: string | null, lastHour: number | null, pageSize = PAGE_SIZE, after?: number) => ({ sourceId: track!.source.id, revision: track!.source.revision, product: 'event-feed',
    params: { branchId: track!.branch.id, firstHour: filter.firstHour, lastHour,
      categories: [...filter.categories], cursor, pageSize, ...(after === undefined ? {} : { after }) } });
  // What the first page reads apart from its hour.
  const lane = identity ? JSON.stringify([identity, filter.firstHour, [...filter.categories]]) : null;
  const [held, setHeld] = useState<HeldFeed | null>(null);
  const holding = held && held.lane === lane ? held : null;
  const live = holding?.latest.reply.lastHour;
  // Moving forward from a held page asks only the rows read since; another hour, or a page
  // grown by a whole page's rows, asks the page whole.
  const after = holding && hour !== null && live !== undefined && hour >= live && holding.count < PAGE_SIZE ? live : undefined;
  const first = useFollowedProduct<PreparedFeedPage>(runtime.client, track && hour !== null ? request(null, hour, FIRST_PAGE_SIZE, after) : null,
    ['lastHour', 'after'], active, validateEventFeedPage, track?.source);
  useEffect(() => {
    const data = first.data;
    if (!data || !lane || !first.moment) return;
    const [asked] = JSON.parse(first.moment) as [number | null, number | null];
    setHeld(previous => {
      const kept = previous?.lane === lane ? previous : null;
      if (kept?.latest === data) return previous;
      if (data.after !== undefined) {
        // Rows read after another hour than the page's are not its.
        if (!kept || data.after !== kept.latest.reply.lastHour) return previous;
        return { ...kept, latest: data, added: data.reply.rows.length ? [data, ...kept.added] : kept.added, count: kept.count + data.reply.rows.length };
      }
      return { lane, base: data, baseHour: asked ?? data.reply.lastHour, added: [], count: 0, latest: data };
    });
  }, [first.data, first.moment, lane]);
  const base = holding?.base;
  // The page chain follows the hour the whole first page answers; rows still stop at `hour`.
  const read = holding ? holding.baseHour : hour;
  // A first page answered from the prepared prefix reads back from the prefix's
  // last hour, and its continuations keep that hour: a whole page asked again starts the
  // chain again.
  const baseLive = base?.prefix?.pending.length ? base.prefix.coverage.lastHour : null;
  const boundsKey = JSON.stringify([identity, filterKey, read, baseLive]);
  const owner = useMemo(() => ({ client: runtime.client, boundsKey, filter }), [runtime.client, boundsKey, filter]);
  const current = useRef(owner); current.current = owner;
  const [chain, setChain] = useState<{ owner: typeof owner; pages: PageInfo[]; error?: Failure }>({ owner, pages: [] });
  const initial: PageInfo = { cursor: null, start: Math.max(filter.firstHour ?? track?.branch.firstHour ?? 0, track?.branch.firstHour ?? 0), end: read ?? 0, count: FIRST_PAGE_SIZE };
  const descriptors = chain.owner === owner && chain.pages.length ? chain.pages : [initial];
  const [wanted, setWanted] = useState<{ owner: typeof owner; pages: readonly string[] }>({ owner, pages: [] });
  const completeDemand = import.meta.env.MODE === 'parity' && prepareAll;
  const chosen = completeDemand ? descriptors.map((_, index) => String(index))
    : wanted.owner === owner ? wanted.pages : ['0'];
  const indices = [...new Set(chosen.flatMap(page => Number.isInteger(Number(page)) && Number(page) > 0
    && Number(page) < descriptors.length ? [Number(page)] : []))];
  const chunks = useProductWindows<PreparedFeedPage>(runtime.client, boundsKey,
    index => request(descriptors[index].cursor, read), indices, active && !!base, validateEventFeedPage, track?.source);
  const loaded = useMemo(() => new Map<number, PreparedFeedPage>([...chunks.data, ...(base ? [[0, base] as const] : [])]), [chunks.data, base]);
  const added = holding?.added;
  // The first page's rows: those read since it, newest first, before its own.
  const parts = useCallback((index: number, page: PreparedFeedPage) => index === 0 && added ? [...added, page] : [page], [added]);
  const playbacks = useMemo(() => new Map([...loaded].map(([index, page]) => [index, new LogPlayback(parts(index, page).flatMap(part => filteredEventRows(part.reply, filter))
    // Native ledgers can repeat an occurrence in different supplying rows/pages.
    // The log's owner is internal; the event's native ID remains unchanged.
    .map(row => ({ ...row, key: JSON.stringify([index, row.key]), id: JSON.stringify([index, row.id]) })), row => row.revealedHour!)])), [loaded, parts, filter]);
  const renderedBefore = useRef<ReadonlyMap<number, ReadonlyArray<LogRow<FeedEvent>>>>(new Map());
  const rendered = useMemo(() => renderedBefore.current = retainPlaybackMap(renderedBefore.current,
    new Map([...playbacks].map(([index, playback]) => [index, playback.at(hour)]))), [playbacks, hour]);
  useEffect(() => {
    if (!active || !base) return;
    setChain(previous => {
      if (current.current !== owner) return previous;
      const pages = previous.owner === owner && previous.pages.length ? [...previous.pages] : [initial];
      let changed = previous.owner !== owner || !previous.pages.length;
      let error: Failure | undefined;
      for (const [index, page] of loaded) {
        const rows = rendered.get(index)!;
        const info = pages[index];
        if (!info) continue;
        if (page.nextCursor !== null && pages.slice(0, index + 1).some(info => info.cursor === page.nextCursor)) {
          error = new Error('The event feed continuation made no progress.');
          break;
        }
        const hours = parts(index, page).flatMap(part => part.reply.rows.map(row => row.hour));
        const start = Math.min(...hours, info.end);
        const end = Math.max(...hours, start);
        if (info.count !== rows.length || info.start !== start || info.end !== end) {
          pages[index] = { ...info, start, end, count: rows.length }; changed = true;
        }
        if (index === pages.length - 1 && page.nextCursor !== null) {
          pages.push({ cursor: page.nextCursor, start: initial.start, end: start, count: PAGE_SIZE }); changed = true;
        }
      }
      return changed || previous.error !== error ? { owner, pages, error } : previous;
    });
  }, [owner, descriptors, loaded, parts, rendered, active, base]);
  const committedPages = useRef<{ owner: typeof owner; pages: readonly LogPage<FeedEvent>[] }>({ owner, pages: [] });
  const pages = useMemo(() => stableLogPages(committedPages.current.owner === owner ? committedPages.current.pages : [], !base ? []
    : descriptors.map((info, index) => ({ key: String(index), start: info.start, end: info.end,
      count: rendered.get(index)?.length ?? info.count, rows: rendered.get(index), error: chunks.errors.get(index) }))),
    [owner, descriptors, base, rendered, chunks.errors]);
  useLayoutEffect(() => { committedPages.current = { owner, pages }; }, [owner, pages]);
  const onWantedPages = useCallback((pages: readonly string[]) => {
    if (current.current !== owner) return;
    setWanted(previous => previous.owner === owner && previous.pages.join(',') === pages.join(',') ? previous : { owner, pages });
  }, [owner]);
  const retry = useCallback(() => { setHeld(null); first.retry(); chunks.retry(); }, [first.retry, chunks.retry]);
  // The newest reply's notes, with those of the replies whose rows the first page still shows.
  const latest = holding?.latest;
  const data = useMemo(() => latest && base ? feedNotes(latest.reply, [...(added ?? []), base].filter(part => part !== latest).map(part => part.reply)) : undefined,
    [latest, base, added]);
  return { data, prefix: latest?.prefix ?? null, error: first.error ?? (chain.owner === owner ? chain.error : undefined), retry, identity, pages,
    warnings: data?.warnings ?? NO_WARNINGS, onWantedPages, errors: chunks.errors, retryPages: chunks.retry };
}
