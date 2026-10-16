import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { LogPage } from '../../charts/log/logPages';
import { stableLogPages } from '../../charts/log/logPages';
import { useContentRevision } from '../../dashboard/contentRevision';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { useProductWindows } from '../../dashboard/productWindows';
import { useProduct } from '../../dashboard/products';
import { filteredEventRows, validateEventFeedPage } from './eventFeedData';
import type { FeedEvent, FeedFilter, PreparedFeedPage } from './eventFeedData';

interface PageInfo { cursor: string | null; start: number; end: number; count: number }
const NO_WARNINGS: string[] = [];
const PAGE_SIZE = 256;

/** Native dates page backward while every cursor retains the same Live read bound.
 * Only demanded payloads stay held; small cursor/count descriptors survive eviction. */
export function useEventFeed(runtime: WidgetRuntime, track: BoundTrack | undefined, active: boolean,
  hour: number | null, filter: FeedFilter, filterKey: string, prepareAll = false) {
  const content = useContentRevision(runtime.client);
  const identity = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, content]) : null;
  const boundsKey = JSON.stringify([identity, filterKey, hour]);
  const owner = useMemo(() => ({ client: runtime.client, boundsKey, filter }), [runtime.client, boundsKey, filter]);
  const current = useRef(owner); current.current = owner;
  const request = (cursor: string | null) => ({ sourceId: track!.source.id, revision: track!.source.revision, product: 'event-feed',
    params: { branchId: track!.branch.id, firstHour: filter.firstHour, lastHour: hour,
      categories: [...filter.categories], cursor, pageSize: PAGE_SIZE } });
  const first = useProduct<PreparedFeedPage>(runtime.client, track && hour !== null ? request(null) : null, active, validateEventFeedPage, track?.source);
  const [chain, setChain] = useState<{ owner: typeof owner; pages: PageInfo[]; error?: string }>({ owner, pages: [] });
  const initial: PageInfo = { cursor: null, start: Math.max(filter.firstHour ?? track?.branch.firstHour ?? 0, track?.branch.firstHour ?? 0), end: hour ?? 0, count: PAGE_SIZE };
  const descriptors = chain.owner === owner && chain.pages.length ? chain.pages : [initial];
  const [wanted, setWanted] = useState<{ owner: typeof owner; pages: readonly string[] }>({ owner, pages: [] });
  const completeDemand = import.meta.env.MODE === 'parity' && prepareAll;
  const chosen = completeDemand ? descriptors.map((_, index) => String(index))
    : wanted.owner === owner ? wanted.pages : ['0'];
  const indices = [...new Set(chosen.flatMap(page => Number.isInteger(Number(page)) && Number(page) > 0
    && Number(page) < descriptors.length ? [Number(page)] : []))];
  const chunks = useProductWindows<PreparedFeedPage>(runtime.client, boundsKey,
    index => request(descriptors[index].cursor), indices, active && !!first.data, validateEventFeedPage, track?.source);
  const loaded = useMemo(() => new Map<number, PreparedFeedPage>([...chunks.data, ...(first.data ? [[0, first.data] as const] : [])]), [chunks.data, first.data]);
  const rendered = useMemo(() => new Map([...loaded].map(([index, page]) => [index, filteredEventRows(page.reply, filter).filter(row => hour !== null && row.revealedHour! <= hour)
    // Native ledgers can repeat an occurrence in different supplying rows/pages.
    // The log's owner is internal; the event's native ID remains unchanged.
    .map(row => ({ ...row, key: JSON.stringify([index, row.key]), id: JSON.stringify([index, row.id]) }))])), [loaded, filter, hour]);
  useEffect(() => {
    if (!active || !first.data) return;
    setChain(previous => {
      if (current.current !== owner) return previous;
      const pages = previous.owner === owner && previous.pages.length ? [...previous.pages] : [initial];
      let changed = previous.owner !== owner || !previous.pages.length;
      let error: string | undefined;
      for (const [index, page] of loaded) {
        const rows = rendered.get(index)!;
        const info = pages[index];
        if (!info) continue;
        if (page.nextCursor !== null && pages.slice(0, index + 1).some(info => info.cursor === page.nextCursor)) {
          error = 'The event feed continuation made no progress.';
          break;
        }
        const start = Math.min(...page.reply.rows.map(row => row.hour), info.end);
        const end = Math.max(...page.reply.rows.map(row => row.hour), start);
        if (info.count !== rows.length || info.start !== start || info.end !== end) {
          pages[index] = { ...info, start, end, count: rows.length }; changed = true;
        }
        if (index === pages.length - 1 && page.nextCursor !== null) {
          pages.push({ cursor: page.nextCursor, start: initial.start, end: start, count: PAGE_SIZE }); changed = true;
        }
      }
      return changed || previous.error !== error ? { owner, pages, error } : previous;
    });
  }, [owner, descriptors, loaded, rendered, active, first.data]);
  const committedPages = useRef<{ owner: typeof owner; pages: readonly LogPage<FeedEvent>[] }>({ owner, pages: [] });
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
  return { data: first.data?.reply, error: first.error ?? (chain.owner === owner ? chain.error : undefined), retry, identity, pages,
    warnings: first.data?.reply.warnings ?? NO_WARNINGS, onWantedPages, errors: chunks.errors, retryPages: chunks.retry };
}
