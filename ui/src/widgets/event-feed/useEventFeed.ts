import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { LogPage } from '../../charts/log/logPages';
import { stableLogPages } from '../../charts/log/logPages';
import { logPlaybackAt, logPlaybackIndex } from '../../charts/log/logPlayback';
import type { LogPlaybackIndex, LogPlaybackSlice } from '../../charts/log/logPlayback';
import { rememberLogCounts, logCountAt } from '../../charts/log/logCounts';
import type { ObservedLogCount } from '../../charts/log/logCounts';
import { useContentRevision } from '../../dashboard/contentRevision';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { useProductWindows } from '../../dashboard/productWindows';
import { useProduct } from '../../dashboard/products';
import type { EventFeedReply } from '../../generated/dashboard';
import { eventWindowCount, filteredEventRows, feedWindows, validateEventFeedReply } from './eventFeedData';
import type { FeedEvent, FeedFilter } from './eventFeedData';

interface PreparedPage { reply: EventFeedReply; filter: FeedFilter; index: LogPlaybackIndex<FeedEvent>; completeBy: number }
const NO_WARNINGS: string[] = [];

/** The same branch product returns its page manifest or one bounded hour window. */
export function useEventFeed(runtime: WidgetRuntime, track: BoundTrack | undefined, active: boolean,
  hour: number | null, filter: FeedFilter, filterKey: string, live: boolean) {
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: 'event-feed',
    params: { branchId: track.branch.id, firstHour: null, lastHour: null } } : null;
  const manifest = useProduct<EventFeedReply>(runtime.client, request, active, validateEventFeedReply);
  const content = useContentRevision(runtime.client);
  const identity = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, content]) : null;
  const shown = useMemo(() => hour === null ? [] : feedWindows(manifest.data?.windows ?? [], hour, filter.firstHour)
    .filter(({ window }) => eventWindowCount(window, filter.categories) > 0),
    [manifest.data, hour, filter.firstHour, filter.categories]);
  const key = useMemo(() => ({ client: runtime.client, identity, filterKey }), [runtime.client, identity, filterKey]);
  const [wanted, setWanted] = useState<{ key: typeof key; pages: readonly string[] }>({ key, pages: [] });
  const visibleIndices = (wanted.key === key ? wanted.pages : shown.slice(0, 1).map(page => page.key))
    .flatMap(page => shown.some(window => window.key === page) ? [Number(page)] : []);
  // Hold the next chronological window during Live so crossing its first event
  // does not wait for a request. Its future rows are still hidden by their reads.
  const next = live && hour !== null ? manifest.data?.windows.findIndex(window => window.firstHour > hour
    && eventWindowCount(window, filter.categories) > 0) ?? -1 : -1;
  const wantedIndices = [...new Set([...visibleIndices, ...(next >= 0 ? [next] : [])])];
  const chunks = useProductWindows<EventFeedReply>(runtime.client, identity, index => ({
    sourceId: track!.source.id, revision: track!.source.revision, product: 'event-feed',
    params: { branchId: track!.branch.id, firstHour: manifest.data!.windows[index].firstHour,
      lastHour: manifest.data!.windows[index].lastHour },
  }), wantedIndices, active && filter.categories.size > 0 && !!manifest.data, validateEventFeedReply);
  const [counts, setCounts] = useState<{ key: typeof key; filter: FeedFilter; counts: ReadonlyMap<string, ObservedLogCount> }>({ key, filter, counts: new Map() });
  const committedPrepared = useRef<{ key: typeof key; pages: ReadonlyMap<string, PreparedPage> }>({ key, pages: new Map() });
  const prepared = useMemo(() => {
    const previous = committedPrepared.current.key === key ? committedPrepared.current.pages : new Map<string, PreparedPage>();
    return new Map<string, PreparedPage>([...chunks.data].map(([index, reply]) => {
      const page = String(index);
      const held = previous.get(page);
      if (held?.reply === reply && held.filter === filter) return [page, held];
      const rows = logPlaybackIndex(filteredEventRows(reply, filter));
      return [page, { reply, filter, index: rows, completeBy: Math.max(reply.lastHour, rows.reads.at(-1) ?? reply.lastHour) }];
    }));
  }, [chunks.data, filter, key]);
  useLayoutEffect(() => { committedPrepared.current = { key, pages: prepared }; }, [key, prepared]);
  const committedSlices = useRef<{ key: typeof key; pages: ReadonlyMap<string, LogPlaybackSlice<FeedEvent>> }>({ key, pages: new Map() });
  const slices = useMemo(() => {
    const previous = committedSlices.current.key === key ? committedSlices.current.pages : new Map<string, LogPlaybackSlice<FeedEvent>>();
    return new Map([...prepared].map(([page, data]) => [page, logPlaybackAt(data.index, hour, previous.get(page))]));
  }, [prepared, hour, key]);
  useLayoutEffect(() => { committedSlices.current = { key, pages: slices }; }, [key, slices]);
  const loaded = useMemo(() => new Map([...slices].map(([page, slice]) => [page, slice.rows])), [slices]);
  useEffect(() => {
    setCounts(previous => {
      if (hour === null || committedPrepared.current.key !== key) return previous;
      const held = previous.key === key && previous.filter === filter ? previous.counts : new Map();
      const samples = [...loaded].map(([page, rows]) => {
        return { key: page, count: rows.length, completeBy: prepared.get(page)!.completeBy };
      });
      const next = rememberLogCounts(held, samples, hour);
      return next === previous.counts && previous.key === key && previous.filter === filter ? previous : { key, filter, counts: next };
    });
  }, [loaded, key, filter, prepared, hour]);
  const committedPages = useRef<{ key: typeof key; pages: readonly LogPage<FeedEvent>[] }>({ key, pages: [] });
  const pages = useMemo(() => stableLogPages(committedPages.current.key === key ? committedPages.current.pages : [], filter.categories.size === 0 ? [] : shown.map(({ key: page, index, window }) => ({
    key: page, start: window.firstHour, end: window.lastHour,
    count: loaded.get(page)?.length ?? logCountAt(counts.key === key && counts.filter === filter ? counts.counts.get(page) : undefined,
      hour, eventWindowCount(window, filter.categories)),
    rows: loaded.get(page), error: chunks.errors.get(index),
  }))), [shown, loaded, counts, key, filter, hour, chunks.errors]);
  useLayoutEffect(() => { committedPages.current = { key, pages }; }, [key, pages]);
  const onWantedPages = useCallback((pages: readonly string[]) => {
    setWanted(previous => committedPages.current.key !== key
      || (previous.key === key && previous.pages.join(',') === pages.join(',')) ? previous : { key, pages });
  }, [key]);
  return { ...manifest, identity, pages, warnings: manifest.data?.warnings ?? NO_WARNINGS,
    onWantedPages, errors: chunks.errors, retryPages: chunks.retry };
}
