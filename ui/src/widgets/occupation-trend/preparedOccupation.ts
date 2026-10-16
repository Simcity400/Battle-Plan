import { useCallback, useMemo, useState } from 'react';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products.ts';
import type { TimeChartProps } from '../../charts/TimeChart';
import type { ChartSummary } from '../../charts/chartSummary';
import type { HourSpan, OccupationTrendParams, OccupationTrendReply, OccupationTrendSeries } from '../../generated/dashboard';
import { validateMeasureSummaries } from '../country-measure/preparedMeasureData.ts';
import type { MeasureSummaries } from '../country-measure/preparedMeasureData';
import { validateOccupationTrendReply } from './occupationTrendData.ts';
import type { ShownMeasure } from './occupationTrendData';

export function validateOccupationSummaries(value: unknown): MeasureSummaries {
  const reply = value as MeasureSummaries;
  if (!reply || !['resistance', 'compliance'].includes(reply.measure)) throw new Error('The prepared occupation summary is malformed.');
  validateMeasureSummaries({ ...reply, measure: 'stability' });
  return reply;
}

export function usePreparedOccupation(client: DashboardClient, track: BoundTrack | undefined, selection: OccupationTrendParams | null,
  shown: ShownMeasure, hour: number, active: boolean, within?: ReadonlyMap<string, readonly HourSpan[]>) {
  const rank = selection?.top?.measure ?? (shown === 'compliance' ? 'compliance' : 'resistance');
  const ownerKey = JSON.stringify([track?.source.id, track?.source.revision, selection, shown, within && [...within]]);
  const owner = useMemo(() => ({ client, key: ownerKey }), [client, ownerKey]);
  const [hover, setHover] = useState<{ owner: typeof owner; hour: number | null }>();
  const [view, setView] = useState<{ owner: typeof owner; from: number; to: number; width: number }>();
  const [summaryMembership, setSummaryMembership] = useState<{owner:typeof owner;key:string;within:ReadonlyMap<string,readonly HourSpan[]>|undefined}>();
  const [navigation, setNavigation] = useState<{ owner: typeof owner; params: unknown; revision: number }>();
  const request = (product: string, measure: 'resistance' | 'compliance', extra: object) => track && selection ? {
    sourceId: track.source.id, revision: track.source.revision, product, params: {
      branchId: track.branch.id, family: { kind: 'occupation-trend', measure, rankMeasure: rank, top: selection.top?.count ?? null,
        collaboration: selection.collaboration, both: shown === 'both' }, tags: selection.tags, within: (product === 'series-summary' || product === 'series-next-change') && summaryMembership?.owner === owner
        ? summaryMembership.within && Object.fromEntries(summaryMembership.within) : within && Object.fromEntries(within), ...extra,
    },
  } : null;
  const head = useProduct<OccupationTrendReply>(client, request('series-state', rank, { reads: [{ hour }] }), active, validateOccupationTrendReply, track?.source);
  const hovered = hover?.owner === owner ? hover.hour : null;
  const reached = useProduct<OccupationTrendReply>(client, hovered == null ? null : request('series-state', rank, { reads: [{ hour: hovered }] }), active, validateOccupationTrendReply, track?.source);
  const viewport = view?.owner === owner ? view : undefined;
  const resistance = useProduct<MeasureSummaries>(client, viewport && shown !== 'compliance' ? request('series-summary', 'resistance', viewport) : null,
    active, validateOccupationSummaries, track?.source);
  const compliance = useProduct<MeasureSummaries>(client, viewport && shown !== 'resistance' ? request('series-summary', 'compliance', viewport) : null,
    active, validateOccupationSummaries, track?.source);
  const next = useProduct<{ hour: number | null }>(client, navigation?.owner === owner && track ? {
    sourceId: track.source.id, revision: track.source.revision, product: 'series-next-change', params: navigation.params,
  } : null, active, value => {
    const reply = value as { hour: number | null };
    if (!reply || reply.hour !== null && !Number.isFinite(reply.hour)) throw new Error('The prepared occupation navigation is malformed.');
    return reply;
  }, track?.source);
  const onSummaryView = useCallback<NonNullable<TimeChartProps['onSummaryView']>>(value => {
    setView(before => before?.owner === owner && before.from === value.from && before.to === value.to && before.width === value.width ? before : { owner, ...value });
  }, [owner]);
  const onSummaryMembership = useCallback((within:ReadonlyMap<string,readonly HourSpan[]>|undefined)=>{
    const key=JSON.stringify(within && [...within]);
    setSummaryMembership(before=>before?.owner===owner && before.key===key ? before : {owner,key,within});
  },[owner]);
  const onReachHour = useCallback((hour: number | null) => {
    const at = hour == null ? null : Math.floor(hour);
    setHover(before => before?.owner === owner && before.hour === at ? before : { owner, hour: at });
  }, [owner]);
  const onNavigateHour = useCallback<NonNullable<TimeChartProps['onNavigateHour']>>((key, from, x, until) => {
    const query = request('series-next-change', rank, { key, from, firstHour: x[0], lastHour: Math.min(x[1], until ?? x[1]) });
    if (query) setNavigation(before => ({ owner, params: query.params, revision: (before?.revision ?? 0) + 1 }));
  }, [owner, summaryMembership]);
  const lineSummaries = useMemo(() => new Map<string, ChartSummary>([resistance.data, compliance.data].flatMap(reply =>
    reply?.series.map(line => [JSON.stringify([line.tag, reply.measure]), line.summary] as const) ?? [])), [resistance.data, compliance.data]);
  const data = useMemo(() => mergeOccupationReads(head.data, reached.data, resistance.data, compliance.data), [head.data, reached.data, resistance.data, compliance.data]);
  return { data, lineSummaries, onSummaryView, onReachHour, onSummaryMembership, navigation: { onNavigateHour, keyboardHour: next.data?.hour, keyboardRevision: navigation?.revision },
    error: head.error ?? reached.error ?? resistance.error ?? compliance.error ?? next.error,
    retry: () => { head.retry(); reached.retry(); resistance.retry(); compliance.retry(); next.retry(); } };
}

/** Dated chart geometry never supplies exact state counts, reads or collaboration. */
export function mergeOccupationReads(head: OccupationTrendReply | undefined, hover: OccupationTrendReply | undefined,
  resistance?: MeasureSummaries, compliance?: MeasureSummaries): OccupationTrendReply | undefined {
  if (!head) return undefined;
  const series = new Map<string, OccupationTrendSeries>();
  for (const reply of [head, hover]) for (const line of reply?.series ?? []) {
    const unique = new Map([...(series.get(line.tag)?.runs ?? []), ...line.runs].map(run => [JSON.stringify(run), run]));
    series.set(line.tag, { ...line, runs: [...unique.values()].sort((a, b) => a.firstHour - b.firstHour) });
  }
  for (const reply of [resistance, compliance]) for (const line of reply?.series ?? []) {
    const metadata = line as typeof line & Pick<OccupationTrendSeries, 'segments' | 'identities'> & {endRun?:OccupationTrendSeries['runs'][number]|null};
    const previous = series.get(line.tag);
    const end = metadata.endRun;
    const extra = end && !previous?.runs.some(run => run.firstHour <= end.firstHour && run.lastHour >= end.lastHour) ? [end] : [];
    const exact = new Map([...(previous?.runs ?? []), ...extra].map(run => [JSON.stringify(run), run]));
    series.set(line.tag, { tag: line.tag, runs: [...exact.values()].sort((a,b)=>a.firstHour-b.firstHour), segments: metadata.segments ?? previous?.segments ?? [],
      identities: metadata.identities ?? previous?.identities, membership: previous?.membership ?? null });
  }
  return { ...head, series: [...series.values()], warnings: [...new Set([head, hover, resistance, compliance].flatMap(reply => reply?.warnings ?? []))] };
}
