import { useCallback, useMemo, useRef, useState } from 'react';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import { loadProduct, useFollowedProduct, useProduct } from '../../dashboard/products';
import { chartSummaryFrom } from '../../charts/chartSummary';
import type { TimeChartProps } from '../../charts/TimeChart';
import type { ChartSummary } from '../../charts/chartSummary';
import type { MarkCluster, MarkSummary } from '../../charts/markSummary';
import type { EventBadge } from '../../charts/badges';
import type { Range } from '../../charts/view';
import type { HourSpan } from "../../generated/dashboard";
import type { PartyHistoryReply } from '../../generated/dashboard';
import { validatePartyHistoryReply } from './partyData';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useHeldFailure } from '../../dashboard/heldFailures';
import { useViewRead } from '../../dashboard/viewRead';
import { isPreparationExtent } from '../../dashboard/preparedPrefix';
import type { PreparedCoverage } from '../../shell/preparation/contract';

export interface PartyPreparedReply {
  coverage: PreparedCoverage;
  reply: PartyHistoryReply;
  segments?: HourSpan[];
  summaries: { token: string; summary: ChartSummary }[];
  marks: { field: string; summary: MarkSummary; exact: boolean }[];
  cursor: string | null;
  total: string;
}
export function validatePreparedParty(value: unknown, requestedCursor?: string): PartyPreparedReply {
  const data = value as PartyPreparedReply;
  validatePartyHistoryReply(data?.reply);
  if (!Array.isArray(data.summaries) || !Array.isArray(data.marks) || !/^\d+$/.test(data.total)
    || data.cursor !== null && !/^\d+$/.test(data.cursor)
    || !isPreparationExtent(data.coverage)) throw new Error('The prepared party reply is malformed.');
  if (requestedCursor !== undefined && data.cursor !== null && BigInt(data.cursor) <= BigInt(requestedCursor)) throw new Error('The prepared party event cursor did not advance.');
  const summaries = data.summaries.map(line => {
    const summary = typeof line?.token === 'string' ? chartSummaryFrom(line.summary) : null;
    if (!summary) throw new Error('The prepared party summary is malformed.');
    return { ...line, summary };
  });
  for (const mark of data.marks) {
    if (!['ruling', 'leader', 'election'].includes(mark.field) || typeof mark.exact !== 'boolean' || !Array.isArray(mark.summary.clusters)
      || mark.summary.clusters.some(cluster => !Number.isInteger(cluster.firstHour) || !Number.isInteger(cluster.lastHour)
        || cluster.firstHour > cluster.lastHour || !/^\d+$/.test(cluster.firstOrdinal) || !/^[1-9]\d*$/.test(cluster.count))) throw new Error('The prepared party marks are malformed.');
  }
  return { ...data, summaries };
}

function validatePartyNavigation(value: unknown): { hour: number | null } {
  const result = value as { hour: number | null };
  if (!result || !(result.hour === null || Number.isFinite(result.hour))) throw new Error('The prepared party navigation is malformed.');
  return result;
}

/** Keep only the current viewport, reached hours and one reached event page. */
export function usePreparedParty(client: DashboardClient, source: DashboardSource | undefined, branchId: string | undefined,
  tag: string | undefined, head: number, hover: number | null, active: boolean, fields: readonly string[] = ["ruling", "leader", "election"], withValues = true, within?: readonly HourSpan[]) {
  const enabled = !!client.preparedQuery && !!source && !!branchId && !!tag;
  const content = useContentRevision(client);
  const ownerKey = JSON.stringify([source?.id, source?.revision, branchId, tag, fields, withValues, within, content]);
  const owner = useMemo(() => ({ client, key: ownerKey }), [client, ownerKey]);
  const [view, setView] = useState<{ owner: typeof owner; from: number; to: number; width: number }>();
  const [page, setPage] = useState<{ owner: typeof owner; field: string; from: number; to: number; cursor: string; start: string }>();
  const [keyboard, setKeyboard] = useState<{ owner: typeof owner; key: string; from: number | null; x: Range; until: number | null; view: string }>();
  const request = (params: Record<string, unknown>) => enabled ? { sourceId: source!.id, revision: source!.revision, product: 'party-history', params: { branchId, tag, within, ...params } } : null;
  const previous = useRef<{owner:typeof owner;data:PartyPreparedReply;hour:number}>(undefined);
  const currentView = view?.owner === owner ? view : undefined;
  const viewKey = JSON.stringify(currentView && [currentView.from,currentView.to,currentView.width]);
  // One summary read at a time while the chart's view moves (`useViewRead`).
  const summaryRequest = request({ mode: 'summary', fields, values: withValues });
  const summaryRead = useViewRead<{ from: number; to: number; width: number }, PartyPreparedReply>(summaryRequest && ownerKey, currentView,
    JSON.stringify([currentView?.from, currentView?.to, currentView?.width]), active, (at, signal, publish, restart) => loadProduct(client,
      request({ mode: 'summary', from: at.from, to: at.to, width: at.width, fields, values: withValues })!, signal, validatePreparedParty, source, restart, publish), client);
  const summary = { ...summaryRead, error: useHeldFailure(summaryRead.error, summaryRead.data !== undefined, summaryRead.retry) };
  // The playhead's hour first: an answer names the playhead it was read for.
  const hours = [...new Set([Math.floor(head), ...(hover == null ? [] : [Math.floor(hover)])])];
  const state = useFollowedProduct(client, withValues ? request({ mode: 'state', hours }) : null, ['hours'], active, validatePreparedParty, source);
  const currentPage = page?.owner === owner ? page : undefined;
  const events = useProduct(client, currentPage ? request({ mode: 'events', field: currentPage.field, from: currentPage.from, to: currentPage.to, cursor: currentPage.cursor, pageSize: 128 }) : null, active,
    value => validatePreparedParty(value, currentPage?.cursor), source);
  const reached = useProduct(client, hover == null ? null : request({mode:'events',field:'all',from:Math.floor(hover),to:Math.floor(hover),fields,pageSize:128}),active,validatePreparedParty,source);
  const currentKey = keyboard?.owner === owner && keyboard.view === viewKey ? keyboard : undefined;
  // Keys stop only where the chart shows a change: the same fields and values as the summary.
  const navigation = useProduct<{ hour: number | null }>(client, currentKey ? request({ mode: 'next', fields, values: withValues, navigation: { key: currentKey.key, from: currentKey.from, firstHour: currentKey.x[0], lastHour: Math.min(currentKey.x[1], currentKey.until ?? currentKey.x[1]) } }) : null, active,
    validatePartyNavigation, source);
  // The ruling-party widget awaits both party and native-government candidates
  // before publishing one keyboard position. Other party consumers own theirs.
  const navigate = useCallback(async (key: 'ArrowLeft' | 'ArrowRight' | 'Home' | 'End', from: number | null, x: Range, until: number | null, signal: AbortSignal) => {
    const next = request({ mode: 'next', fields, values: withValues, navigation: { key, from, firstHour: x[0], lastHour: Math.min(x[1], until ?? x[1]) } });
    if (!next) return null;
    return (await loadProduct(client, next, signal, validatePartyNavigation, source)).hour;
  }, [owner]); // eslint-disable-line react-hooks/exhaustive-deps
  const onSummaryView = useCallback<NonNullable<TimeChartProps['onSummaryView']>>(next => {
    setView(previous => previous?.owner === owner && previous.from === next.from && previous.to === next.to && previous.width === next.width ? previous : { owner, ...next });
  }, [owner]);
  const onMarkHover = useCallback((cluster: MarkCluster | null) => {
    if (!cluster) return;
    const field = cluster.tag;
    if (!field) return;
    // Membership pages count only member hours, so their cursor indexes those events from the cluster's first.
    const start = within ? '0' : cluster.firstOrdinal;
    setPage(previous => previous?.owner === owner && previous.field === field && previous.from === cluster.firstHour && previous.to === cluster.lastHour ? previous
      : { owner, field, from: cluster.firstHour, to: cluster.lastHour, cursor: start, start });
  }, [owner]); // eslint-disable-line react-hooks/exhaustive-deps
  const onNavigateHour = useCallback<NonNullable<TimeChartProps['onNavigateHour']>>((key, from, x, until) => setKeyboard({ owner, key, from, x, until, view: viewKey }), [owner,viewKey]);
  const answered = state?.moment ? (JSON.parse(state.moment) as [number[]])[0] : undefined;
  if (state?.data && answered) previous.current={owner,data:state.data,hour:answered[0]};
  const retained=previous.current?.owner===owner ? previous.current : undefined;
  const current = !!state?.data && answered?.[0] === Math.floor(head);
  const presentationHour=current || !retained?.data.segments?.some(span=>span.firstHour<=Math.floor(head) && span.lastHour>=Math.floor(head)) ? Math.floor(head) : retained.hour;
  const exact = state?.data?.reply ?? retained?.data.reply;
  const data = useMemo(() => {
    if (!exact) return summary?.data?.reply;
    const parties = new Map(exact.parties.map(party => [party.token, party]));
    for (const party of summary?.data?.reply.parties ?? []) if (!parties.has(party.token)) parties.set(party.token, { token: party.token, runs: [] });
    return { ...exact, parties: [...parties.values()].sort((a, b) => a.token < b.token ? -1 : a.token > b.token ? 1 : 0) };
  }, [exact, summary?.data]);
  return { enabled, owner, data, presentationHour, summary: summary?.data, events: events.data, reached: reached.data, page: currentPage, onSummaryView, onMarkHover, onNavigateHour, navigate,
    keyboardHour: navigation.data?.hour, keyboardRevision: navigation.data,
    loaded: (hour: number) => !!state?.data && !!answered?.includes(hour),
    closePage: () => setPage(undefined),
    previousPage: () => setPage(previous => previous?.owner === owner ? {...previous,cursor:String(BigInt(previous.cursor)-128n>BigInt(previous.start) ? BigInt(previous.cursor)-128n : BigInt(previous.start))} : previous),
    nextPage: () => { const cursor = events.data?.cursor; if (cursor) setPage(previous => previous?.owner === owner ? { ...previous, cursor } : previous); },
    error: state?.error ?? summary?.error ?? events.error ?? reached.error ?? navigation.error,
    retry: () => { state.retry(); summary.retry(); events.retry(); reached.retry(); navigation.retry(); },
  };
}

export function partyMarkSummaries(data: PartyPreparedReply | undefined, badges: readonly EventBadge[], clusters = false): MarkSummary[] | undefined {
  return data?.marks.filter(mark=>clusters || !mark.exact).map(mark=>({...mark.summary,clusters:mark.summary.clusters.map(cluster=>({...cluster,tag:mark.field,
    badge:cluster.count==='1' ? badges.find(badge=>badge.hour===cluster.firstHour && (badge.glyph==='\u2713' ? 'election' : badge.glyph==='\u2654' ? 'leader' : 'ruling')===mark.field) : undefined}))}));
}
