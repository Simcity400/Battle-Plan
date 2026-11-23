import { useGate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime';
import { useMemo, useRef } from 'react';
import type { LogPage, LogRow } from '../../charts/log/logData';
import type { LogReplyRow } from '../../charts/log/logSource';
import { stableLogPages } from '../../charts/log/logPages';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { resolveWindow } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import { useLogPages } from '../../dashboard/useLogPages';
import type { LogCut, LogSpec } from '../../dashboard/useLogPages';
import type { EventFeedKind, EventFeedSpanReply, EventFeedUnavailable, MembershipRun, ScriptEdge } from '../../generated/dashboard';
import { useHeadHour } from '../army-size/headHour';
import { FEED_PAGE_SIZE, kindsRequest, pageNotes, pagesNoted, spanNotes, validateSpan } from './eventFeedData';
import { cardFacets, cardsKey, cutPages, recordingBreaks } from './scriptOccurrences';
import type { RecordingBreak } from './scriptOccurrences';
import { productRequest } from '../../dashboard/request.ts';

const NO_NOTES: string[] = [];

export interface CardList<C, A> {
  product: 'event-choices' | 'decisions';
  scope?: readonly MembershipRun[];
  openers: readonly EventFeedKind[];
  continuing: readonly EventFeedKind[];
  row: (reply: LogReplyRow) => LogRow<C | null>;
  edges: (card: C) => readonly ScriptEdge[];
  days: (card: C) => boolean;
  at: (row: LogRow<C | null>, hour: number, breaks: readonly RecordingBreak[]) => LogRow<A> | null;
}

export function useCards<C, A>(runtime: WidgetRuntime, widget: WidgetSpec, track: BoundTrack | undefined, active: boolean, list: CardList<C, A>) {
  const follow = widget.time.mode === 'follow';
  const gate = useGate(runtime, widget, SPEC, active);
  const window = track ? resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour()) : null;
  const from = widget.time.mode === 'range' ? window?.firstHour ?? null : null;
  const offset = track?.axisOffset ?? 0;
  const cut: LogCut = follow ? { follow: true, offset } : { follow: false, hour: widget.time.mode === 'campaign' ? null : window?.lastHour ?? null };
  const span = useProduct<EventFeedSpanReply>(runtime.client, track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'event-feed-span',
    params: { branchId: track.branch.id, categories: [] } }) : null, active, validateSpan, track?.source);
  const coded = useProduct<EventFeedSpanReply>(runtime.client, track ? kindsRequest(track) : null, active, validateSpan, track?.source);
  const kinds = coded.data?.kinds;
  const openersKey = list.openers.join(',');
  const logSpan = track ? gate.span(from ?? track.branch.firstHour, track.branch.lastHour) : null;
  const spec = useMemo<LogSpec | null>(() => track && logSpan && kinds && list.openers.length ? { source: track.source, branchId: track.branch.id, log: 'feed-events',
    product: list.product, facets: cardFacets(kinds, list.openers, list.continuing), ...(list.scope ? { scope: list.scope?.flatMap(run => { const held = gate.span(run.firstHour, run.lastHour); return held ? [{ ...run, ...held }] : []; }) } : {}), ...logSpan } : null,
  [track, kinds, openersKey, list.product, list.scope, logSpan?.firstHour, logSpan?.lastHour]);
  const feed = useLogPages<C | null, EventFeedUnavailable>(runtime.client, runtime.clock, spec, cut, active, list.row, FEED_PAGE_SIZE, pageNotes);
  const breaksKey = track ? JSON.stringify(recordingBreaks(track.branch)) : '[]';
  const breaks = useMemo<RecordingBreak[]>(() => JSON.parse(breaksKey), [breaksKey]);
  const held = useMemo(() => {
    const edges: number[] = [];
    let days = false;
    for (const page of feed.pages) for (const row of page.rows ?? []) if (row.item) {
      for (const edge of list.edges(row.item)) edges.push(edge.row.readHour);
      days ||= list.days(row.item);
    }
    return { edges: edges.sort((a, b) => a - b), days };
  }, [feed.pages, list]);
  const headHour = useHeadHour(runtime.clock, active && follow, hour => cardsKey(held.edges, breaks, held.days, Math.floor(hour) + offset));
  const hour = follow ? Math.floor(headHour) + offset : cut.follow ? null : cut.hour ?? track?.branch.lastHour ?? null;
  const shown = useRef<readonly LogPage<A>[]>([]);
  const pages = useMemo(() => shown.current = stableLogPages(shown.current, hour === null ? [] : cutPages(feed.pages, row => row.hour > hour ? null : list.at(row, hour, breaks))),
    [feed.pages, list, hour, breaks]);
  const notes = useMemo(() => span.data ? [...new Set([...spanNotes(span.data), ...pagesNoted(feed.notes)])] : NO_NOTES, [span.data, feed.notes]);
  return { span, feed, pages, hour, follow, from, notes, error: span.error ?? coded.error ?? feed.error, retry: () => { span.retry(); coded.retry(); feed.retry(); },
    loading: !kinds || (!!spec && !feed.counts) };
}
