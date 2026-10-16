import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { TimeChartProps } from '../../charts/TimeChart';
import type { ChartSummary } from '../../charts/chartSummary';
import type { Span } from '../../charts/series';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import { loadProduct, useProduct } from '../../dashboard/products';
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
  const [display, setDisplay] = useState<{ owner: typeof owner; view: StockpileView; summaries: ReadonlyMap<string, ChartSummary>; needed?: ChartSummary; data?: PreparedStockpileReply; error?: string }>();
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!prepared || !active || !groups.length) return;
    const controller = new AbortController();
    const read = (hour: number) => loadProduct<PreparedStockpileReply>(client, request(hour, hour, 1), controller.signal, validate, track.source);
    const next = async (hour: number, forward: boolean, fields: readonly string[]) => {
      const value = await preparedSeriesQuery(client, track.source, 'equipment-stockpile', {
        ...selection, from: track.branch.firstHour, to: track.branch.lastHour, kind: 'edge', fields,
        navigation: { key: forward ? 'ArrowRight' : 'ArrowLeft', from: hour, firstHour: track.branch.firstHour, lastHour: track.branch.lastHour },
      }, controller.signal);
      return navigationHour(value, forward, hour);
    };
    const viewportRequest = request(viewport.from, viewport.to, viewport.width);
    void loadProduct<PreparedStockpileReply>(client, { ...viewportRequest, params: { ...viewportRequest.params,
      fields: ['stock', 'balance', 'logistics-stock', 'available'] } }, controller.signal,
      validate, track.source).then(async data => {
      const exact = data.prepared?.coverage.every(summary => summary.runs !== undefined) ?? false;
      let summaries = stockpileSummaries(data, groups, grouping, viewport, exact);
      let needed = neededGroup ? stockpileAmountSummary(data, neededGroup, 'need') : undefined;
      if (exact) {
        const runs = await exactStockpileRuns(data, groups, grouping, viewport, [track.branch.firstHour, track.branch.lastHour], read, next);
        summaries = new Map([...summaries].map(([key, summary]) => [key, { ...summary, runs: runs.get(key) ?? [] }]));
        if (neededGroup) {
          const runs = await exactStockpileRuns(data, [neededGroup], grouping, viewport, [track.branch.firstHour, track.branch.lastHour], read, next, 'need');
          needed = { ...viewport, buckets: [], runs: runs.get(neededGroup.key) ?? [] };
        }
      }
      if (!controller.signal.aborted) setDisplay({ owner, view: viewport, summaries, needed, data });
    }).catch(error => { if (!controller.signal.aborted) setDisplay({ owner, view: viewport, summaries: new Map(), error: String(error) }); });
    return () => controller.abort();
    // The scope records each group's exact membership and inclusion operands.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [owner, prepared, active, viewport.from, viewport.to, viewport.width, attempt]);
  const [hover, setHover] = useState<number | null>(null);
  const pointed = useProduct<PreparedStockpileReply>(client, prepared ? request(hover ?? head, hover ?? head, 1) : null,
    active && prepared, validate, track.source);
  const navigation = useRef<AbortController | null>(null);
  const [keyboard, setKeyboard] = useState<{ owner: typeof owner; hour: number | null; revision: number }>();
  useEffect(() => () => navigation.current?.abort(), [owner, active]);
  const onNavigateHour = useCallback<NonNullable<TimeChartProps['onNavigateHour']>>((key, from, x, until) => {
    navigation.current?.abort(); const controller = new AbortController(); navigation.current = controller;
    void preparedSeriesQuery(client, track.source, 'equipment-stockpile', { ...selection,
      from: x[0], to: Math.min(x[1], until ?? x[1]), kind: keyboardFields.every(field => field.startsWith('reads-')) ? 'next-change' : 'edge', fields: keyboardFields,
      navigation: { key, from, firstHour: x[0], lastHour: Math.min(x[1], until ?? x[1]) } }, controller.signal).then(value => {
      if (!controller.signal.aborted && current.current === owner) setKeyboard(previous => ({ owner, hour: navigationHour(value, key === 'Home' || key === 'ArrowRight', from), revision: (previous?.revision ?? 0) + 1 }));
    }).catch(error => { if (!controller.signal.aborted && current.current === owner) setDisplay({ owner, view: viewport, summaries: new Map(), error: String(error) }); });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [owner]);
  const onSummaryView = useCallback<NonNullable<TimeChartProps['onSummaryView']>>(next => {
    if (next.to < next.from) return;
    setView(before => before?.scope === scope && before.view.from === next.from && before.view.to === next.to && before.view.width === next.width ? before : { scope, view: next });
  }, [scope]);
  return { prepared, summaries: display?.owner === owner ? display.summaries : new Map<string, ChartSummary>(),
    data: pointed.data, displayData: display?.owner === owner ? display.data : undefined, error: (display?.owner === owner ? display.error : undefined) ?? pointed.error,
    needed: display?.owner === owner ? display.needed : undefined,
    onSummaryView: prepared ? onSummaryView : undefined, onNavigateHour: prepared ? onNavigateHour : undefined,
    keyboardHour: keyboard?.owner === owner ? keyboard.hour : undefined, keyboardRevision: keyboard?.owner === owner ? keyboard.revision : undefined,
    onHover: prepared ? (hour: number | null) => setHover(hour === null ? null : Math.floor(hour)) : undefined,
    retry: () => { setAttempt(value => value + 1); pointed.retry(); } };
}
