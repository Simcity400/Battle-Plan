import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { TimeChartProps } from '../charts/TimeChart';
import { isChartSummary } from '../charts/chartSummary.ts';
import type { ChartSummary } from '../charts/chartSummary';
import type { FactionTotalsParams, FactionTotalsReply, FactionTotalSeries } from '../generated/dashboard';
import type { DashboardClient } from './client';
import type { BoundTrack } from './model';
import { validateFactionTotalsReply } from './factionTotals.ts';
import { preparedSeriesQuery } from './preparedSeries.ts';
import { useFollowedProduct, useProduct } from './products.ts';
import { NumericReply } from './numericReply.ts';
import { factionNumbers } from '../charts/race/factionParts.ts';

export interface PreparedFactionTotals extends FactionTotalsReply {
  summaries?: Record<string, ChartSummary>;
  memberSummaries?: Record<string, ChartSummary>;
}

export function validatePreparedFactionTotals(value: unknown): PreparedFactionTotals {
  const reply = validateFactionTotalsReply(value) as PreparedFactionTotals;
  for (const summaries of [reply.summaries, reply.memberSummaries]) {
    if (summaries !== undefined && (!summaries || typeof summaries !== 'object' || Array.isArray(summaries)
      || !Object.values(summaries).every(isChartSummary))) throw new Error('The prepared faction geometry is malformed.');
  }
  return reply;
}

/** Later reached facts replace overlapping window facts; dense geometry supplies no values. */
function dated<T extends { firstHour: number; lastHour: number }>(previous: readonly T[], rows: readonly T[]): T[] {
  let result = [...previous];
  for (const row of rows) {
    result = result.flatMap(old => {
      if (old.lastHour < row.firstHour || old.firstHour > row.lastHour) return [old];
      return [
        ...(old.firstHour < row.firstHour ? [{ ...old, lastHour: row.firstHour - 1 }] : []),
        ...(old.lastHour > row.lastHour ? [{ ...old, firstHour: row.lastHour + 1,
          ...('startsSegment' in old ? { startsSegment: false } : {}) }] : []),
      ];
    });
    result.push(row);
  }
  return result.sort((a, b) => a.firstHour - b.firstHour);
}

function members(previous: FactionTotalSeries['members'], rows: FactionTotalSeries['members']) {
  const byTag = new Map<string, FactionTotalSeries['members']>();
  for (const row of [...previous, ...rows]) byTag.set(row.tag, dated(byTag.get(row.tag) ?? [], [row]));
  return [...byTag.values()].flat();
}

export function mergeFactionReads(head: PreparedFactionTotals | undefined, hover?: PreparedFactionTotals,
  summary?: PreparedFactionTotals, origin?: PreparedFactionTotals): PreparedFactionTotals | undefined {
  if (!head) return undefined;
  const series = new Map<string, FactionTotalSeries>();
  let recorded: FactionTotalsReply['recorded'] = [], disjoint: FactionTotalsReply['disjoint'] = [];
  const tracks = new Map<string, NonNullable<FactionTotalsReply['memberTracks']>[number]>();
  for (const reply of [summary, head, hover, origin]) {
    if (!reply) continue;
    recorded = dated(recorded, reply.recorded); disjoint = dated(disjoint, reply.disjoint);
    for (const line of reply.series) {
      const before = series.get(line.id);
      series.set(line.id, { ...line, runs: dated(before?.runs ?? [], line.runs), names: dated(before?.names ?? [], line.names),
        membership: dated(before?.membership ?? [], line.membership), members: members(before?.members ?? [], line.members),
        countedMembers: members(before?.countedMembers ?? [], line.countedMembers) });
    }
    for (const track of reply.memberTracks ?? []) {
      const before = tracks.get(track.tag);
      tracks.set(track.tag, { ...track, runs: dated(before?.runs ?? [], track.runs) });
    }
  }
  return { ...head, series: [...series.values()], recorded, disjoint,
    ...(head.memberTracks === undefined ? {} : { memberTracks: [...tracks.values()] }),
    summaries: summary?.summaries, memberSummaries: summary?.memberSummaries,
    warnings: [...new Set([head, hover, summary, origin].flatMap(reply => reply?.warnings ?? []))] };
}

/** One window and a bounded set of exact reached hours, using the shared prepared transport. */
export function usePreparedFactionTotals(client: DashboardClient, track: BoundTrack | undefined, selection: FactionTotalsParams | null,
  hour: number, active: boolean, extent: readonly [number, number], origin?: number, summarize = true) {
  const key = JSON.stringify([track?.source.id, track?.source.revision, selection, extent, summarize]);
  const owner = useMemo(() => ({ client, key }), [client, key]);
  const current = useRef(owner); current.current = owner;
  const controllers = useRef(new Set<AbortController>());
  useEffect(() => () => { for (const controller of controllers.current) controller.abort(); controllers.current.clear(); }, [owner]);
  const [hover, setHover] = useState<{ owner: typeof owner; hour: number | null }>();
  const [view, setView] = useState<{ owner: typeof owner; from: number; to: number; width: number }>();
  const [navigation, setNavigation] = useState<{ owner: typeof owner; params: unknown; revision: number }>();
  const request = (product: string, extra: object) => track && selection ? { sourceId: track.source.id, revision: track.source.revision,
    product, params: { branchId: track.branch.id, family: { kind: 'faction-totals', selection }, ...extra } } : null;
  const head = useFollowedProduct<PreparedFactionTotals>(client, request('series-state', { reads: [{ hour }] }), ['reads'], active, validatePreparedFactionTotals, track?.source);
  const hovered = hover?.owner === owner ? hover.hour : null;
  const reached = useProduct<PreparedFactionTotals>(client, hovered == null ? null : request('series-state', { reads: [{ hour: hovered }] }), active, validatePreparedFactionTotals, track?.source);
  const anchor = useProduct<PreparedFactionTotals>(client, origin === undefined || origin === hour ? null : request('series-state', { reads: [{ hour: Math.floor(origin) }] }), active, validatePreparedFactionTotals, track?.source);
  const viewport = view?.owner === owner ? view : { from: extent[0], to: extent[1], width: 1 };
  // One summary read at a time while the chart's view moves.
  const summary = useFollowedProduct<PreparedFactionTotals>(client, summarize ? request('series-summary', { from: viewport.from, to: viewport.to, width: viewport.width }) : null,
    ['from', 'to', 'width'], active, validatePreparedFactionTotals, track?.source);
  const retained = useMemo(() => new NumericReply<PreparedFactionTotals>(), [client]);
  const geometryData = summarize ? retained.update(key, summary.data, reply => factionNumbers(reply)) : undefined;
  const pickOwner = useMemo(() => ({ owner, geometryData }), [owner, geometryData]);
  const currentPick = useRef(pickOwner); currentPick.current = pickOwner;
  const next = useProduct<{ hour: number | null }>(client, navigation?.owner === owner && track ? {
    sourceId: track.source.id, revision: track.source.revision, product: 'series-next-change', params: navigation.params,
  } : null, active, value => {
    const reply = value as { hour: number | null };
    if (!reply || reply.hour !== null && !Number.isFinite(reply.hour)) throw new Error('The prepared faction navigation is malformed.');
    return reply;
  }, track?.source);
  const onSummaryView = useCallback<NonNullable<TimeChartProps['onSummaryView']>>(value => {
    setView(before => before?.owner === owner && before.from === value.from && before.to === value.to && before.width === value.width ? before : { owner, ...value });
  }, [owner]);
  const onReachHour = useCallback((hour: number | null) => {
    const at = hour == null ? null : Math.floor(hour);
    setHover(before => before?.owner === owner && before.hour === at ? before : { owner, hour: at });
  }, [owner]);
  const onNavigateHour = useCallback<NonNullable<TimeChartProps['onNavigateHour']>>((key, from, x, until) => {
    const query = request('series-next-change', { key, from, firstHour: x[0], lastHour: Math.min(x[1], until ?? x[1]) });
    if (query) setNavigation(before => ({ owner, params: query.params, revision: (before?.revision ?? 0) + 1 }));
  }, [owner]);
  const exactLineTags = useCallback(async (hour: number): Promise<readonly string[] | null> => {
    const query = request('series-state', { reads: [{ hour: Math.floor(hour) }] });
    if (!query || !track || !active) return null;
    const controller = new AbortController(); controllers.current.add(controller);
    try {
      const value = await preparedSeriesQuery(client, track.source, 'series-state', query.params, controller.signal);
      if (controller.signal.aborted || current.current !== owner || currentPick.current !== pickOwner) return null;
      const at = Math.floor(hour);
      setHover(before => before?.owner === owner && before.hour === at ? before : { owner, hour: at });
      return validatePreparedFactionTotals(value).series.filter(series => series.runs.some(run => run.firstHour <= hour && run.lastHour >= hour)).map(series => series.id);
    } finally { controllers.current.delete(controller); }
  }, [owner, active, pickOwner]);
  const data = useMemo(() => mergeFactionReads(head.data, reached.data, geometryData, anchor.data), [head.data, reached.data, geometryData, anchor.data]);
  const lineSummaries = useMemo(() => new Map(Object.entries(geometryData?.summaries ?? {})), [geometryData]);
  // The hour the shown head answers; one kept while playback's newest loads holds its own.
  return { headHour: head.data && head.moment ? (JSON.parse(head.moment) as [{ hour: number }[]])[0][0]?.hour : undefined, data, geometryData, lineSummaries, onSummaryView, onReachHour, exactLineTags, owner: pickOwner,
    navigation: { onNavigateHour, keyboardHour: next.data?.hour, keyboardRevision: navigation?.revision },
    error: head.error ?? reached.error ?? summary.error ?? anchor.error ?? next.error,
    retry: () => { head.retry(); reached.retry(); summary.retry(); anchor.retry(); next.retry(); } };
}
