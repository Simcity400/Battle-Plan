import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { TimeChartProps } from '../../charts/TimeChart';
import type { ChartSummary } from '../../charts/chartSummary';
import type { Span } from '../../charts/series';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import { loadProduct, useProduct } from '../../dashboard/products';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useViewRead } from '../../dashboard/viewRead';
import { preparedSeriesQuery } from '../../dashboard/preparedSeries';
import { validateStockpileReply } from './stockpileData';
import type { Grouping, StockGroup } from './stockpileData';
import { exactStockpileRuns, stockpileAmountSummary, stockpileGroupKey, stockpileSummaries } from './preparedStockpileChart';
import type { PreparedStockpileReply, StockpileView } from './preparedStockpileChart';
import { stockpileCountryData } from './stockpileDrill';

const navigationHour = (value: unknown, forward: boolean, from: number | null) => {
  const replies = (Array.isArray(value) ? value : [value]) as { hour: number | null }[];
  const hours = replies.flatMap(reply => reply.hour === null ? [] : [reply.hour]);
  const moved = hours.filter(hour => hour !== from);
  return (moved.length ? moved : hours).reduce<number | null>((a, b) => a === null ? b : forward ? Math.min(a, b) : Math.max(a, b), null);
};

/** One visible display and one reached date. Exact edge walks keep only their
 * current point; requests share the existing product cache and cancellation. */
export function usePreparedStockpileChart(client: DashboardClient, track: BoundTrack, tag: string,
  groups: readonly StockGroup[], grouping: Grouping, head: number, active: boolean, keyboardFields: readonly string[],
  within?: readonly Span[], aliases?: ReadonlyMap<string, readonly Span[]> | null, neededGroup?: StockGroup) {
  const prepared = !!client.preparedQuery;
  const scope = JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag, grouping, groups.map(group => [group.key, group.variants.map(row => row.id), group.types, group.scope]), within, aliases && [...aliases], neededGroup?.key]);
  const owner = useMemo(() => ({ client, scope }), [client, scope]);
  const current = useRef(owner); current.current = owner;
  const [view, setView] = useState<{ scope: string; view: StockpileView }>();
  const viewport = view?.scope === scope ? view.view : { from: head, to: head, width: 1 };
  const selection = { branchId: track.branch.id, tag, within, ...(aliases ? { tags: [...aliases.keys()], withinByTag: Object.fromEntries(aliases) } : {}),
    included: groups.find(group => group.kind !== 'variant')?.scope ?? groups[0]?.scope ?? [], groups: groups.map(stockpileGroupKey) };
  // Scope contains every selector; stable callbacks cannot retain another holder.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const request = useCallback((from: number, to: number, width?: number) => ({ sourceId: track.source.id, revision: track.source.revision,
    product: 'equipment-stockpile', params: { ...selection, from, to, width } }), [owner]);
  const validate = (value: unknown): PreparedStockpileReply => aliases ? stockpileCountryData((value as unknown[]).map(validateStockpileReply), aliases, tag)! : validateStockpileReply(value);
  // One display read at a time while the chart's view moves (`useViewRead`); the scope
  // records each group's exact membership and inclusion operands.
  const content = useContentRevision(client);
  const display = useViewRead<StockpileView, { summaries: ReadonlyMap<string, ChartSummary>; needed?: ChartSummary; data: PreparedStockpileReply }>(
    prepared && groups.length ? JSON.stringify([scope, content]) : null, viewport, JSON.stringify([viewport.from, viewport.to, viewport.width]), active,
    async (at, signal, _publish, restart) => {
      const read = (hour: number) => loadProduct<PreparedStockpileReply>(client, request(hour, hour, 1), signal, validate, track.source, restart);
      const next = async (hour: number, forward: boolean, fields: readonly string[]) => {
        const value = await preparedSeriesQuery(client, track.source, 'equipment-stockpile', {
          ...selection, from: track.branch.firstHour, to: track.branch.lastHour, kind: 'edge', fields,
          navigation: { key: forward ? 'ArrowRight' : 'ArrowLeft', from: hour, firstHour: track.branch.firstHour, lastHour: track.branch.lastHour },
        }, signal, restart);
        return navigationHour(value, forward, hour);
      };
      const viewportRequest = request(at.from, at.to, at.width);
      const data = await loadProduct<PreparedStockpileReply>(client, { ...viewportRequest, params: { ...viewportRequest.params,
        fields: ['stock', 'balance', 'logistics-stock', 'available'] } }, signal, validate, track.source, restart);
      const exact = data.prepared?.coverage.every(summary => summary.runs !== undefined) ?? false;
      let summaries = stockpileSummaries(data, groups, grouping, at, exact);
      let needed = neededGroup ? stockpileAmountSummary(data, neededGroup, 'need') : undefined;
      if (exact) {
        const runs = await exactStockpileRuns(data, groups, grouping, at, [track.branch.firstHour, track.branch.lastHour], read, next);
        summaries = new Map([...summaries].map(([key, summary]) => [key, { ...summary, runs: runs.get(key) ?? [] }]));
        if (neededGroup) {
          const runs = await exactStockpileRuns(data, [neededGroup], grouping, at, [track.branch.firstHour, track.branch.lastHour], read, next, 'need');
          needed = { ...at, buckets: [], runs: runs.get(neededGroup.key) ?? [] };
        }
      }
      return { summaries, needed, data };
    }, client);
  const [navigationError, setNavigationError] = useState<{ owner: typeof owner; error: string }>();
  const [hover, setHover] = useState<number | null>(null);
  const pointed = useProduct<PreparedStockpileReply>(client, prepared ? request(hover ?? head, hover ?? head, 1) : null,
    active && prepared, validate, track.source);
  const navigation = useRef<AbortController | null>(null);
  const [keyboard, setKeyboard] = useState<{ owner: typeof owner; hour: number | null; revision: number }>();
  useEffect(() => () => navigation.current?.abort(), [owner, active]);
  const onNavigateHour = useCallback<NonNullable<TimeChartProps['onNavigateHour']>>((key, from, x, until) => {
    navigation.current?.abort(); const controller = new AbortController(); navigation.current = controller;
    setNavigationError(undefined);
    void preparedSeriesQuery(client, track.source, 'equipment-stockpile', { ...selection,
      from: x[0], to: Math.min(x[1], until ?? x[1]), kind: keyboardFields.every(field => field.startsWith('reads-')) ? 'next-change' : 'edge', fields: keyboardFields,
      navigation: { key, from, firstHour: x[0], lastHour: Math.min(x[1], until ?? x[1]) } }, controller.signal).then(value => {
      if (!controller.signal.aborted && current.current === owner) setKeyboard(previous => ({ owner, hour: navigationHour(value, key === 'Home' || key === 'ArrowRight', from), revision: (previous?.revision ?? 0) + 1 }));
    }).catch(error => { if (!controller.signal.aborted && current.current === owner) setNavigationError({ owner, error: String(error) }); });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [owner]);
  const onSummaryView = useCallback<NonNullable<TimeChartProps['onSummaryView']>>(next => {
    if (next.to < next.from) return;
    setView(before => before?.scope === scope && before.view.from === next.from && before.view.to === next.to && before.view.width === next.width ? before : { scope, view: next });
  }, [scope]);
  return { prepared, summaries: display.data?.summaries ?? new Map<string, ChartSummary>(),
    data: pointed.data, displayData: display.data?.data,
    error: display.error ?? (navigationError?.owner === owner ? navigationError.error : undefined) ?? pointed.error,
    needed: display.data?.needed,
    onSummaryView: prepared ? onSummaryView : undefined, onNavigateHour: prepared ? onNavigateHour : undefined,
    keyboardHour: keyboard?.owner === owner ? keyboard.hour : undefined, keyboardRevision: keyboard?.owner === owner ? keyboard.revision : undefined,
    onHover: prepared ? (hour: number | null) => setHover(hour === null ? null : Math.floor(hour)) : undefined,
    retry: () => { setNavigationError(undefined); display.retry(); pointed.retry(); } };
}
