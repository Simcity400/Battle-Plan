import { type Failure, asFailure } from '../../dashboard/queryError.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import { loadProduct } from '../../dashboard/products';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useViewRead } from '../../dashboard/viewRead';
import { preparedSeriesQuery } from '../../dashboard/preparedSeries';
import { useHeldFailure } from '../../dashboard/heldFailures';
import type { ArmySizeParams, ArmySizeReply, CountryIdentityRun } from '../../generated/dashboard';
import type { ChartSummary } from '../../charts/chartSummary';
import { chartSummaryLines } from '../../charts/chartSummary';
import type { TimeChartProps } from '../../charts/TimeChart';
import { validateArmySizeReply } from './armySizeData';
import { countryValueAt, countryValuesModel } from '../../charts/race/countryValues';

export function usePreparedArmy(client: DashboardClient, track: BoundTrack | undefined, params: ArmySizeParams | null, manifest: ArmySizeReply | undefined, active: boolean) {
  const key = JSON.stringify([track?.source.id, track?.source.revision, params]);
  const owner = useMemo(() => ({ client, key }), [client, key]);
  const alive = useRef(owner); alive.current = owner;
  const pending = useRef(new Set<AbortController>());
  // Only the latest key's request may answer the chart's pending navigation.
  const navigation = useRef<AbortController | null>(null);
  const [keyboard, setKeyboard] = useState<{ owner: typeof owner; hour: number | null; revision: number }>();
  const [navigationError, setNavigationError] = useState<{ owner: typeof owner; error: Failure }>();
  useEffect(() => () => { for (const request of pending.current) request.abort(); pending.current.clear(); }, [owner, active]);
  const [view, setView] = useState<{ key: string; from: number; to: number; width: number }>();
  const dense = !!client.preparedQuery && view?.key === key && Math.ceil(view.to) - Math.floor(view.from) + 1 > view.width;
  // One summary read at a time while the chart's view moves (`useViewRead`).
  const content = useContentRevision(client);
  const shown = view?.key === key ? view : null;
  const incoming = useViewRead<{ from: number; to: number; width: number }, { series: { tag: string; summary: ChartSummary; identities?: CountryIdentityRun[] }[] }>(
    track && params ? JSON.stringify([key, content]) : null, shown, JSON.stringify([shown?.from, shown?.to, shown?.width]), active,
    (at, signal, publish, restart) => loadProduct(client, { sourceId: track!.source.id, revision: track!.source.revision, product: 'series-summary',
      params: { branchId: params!.branchId, family: { kind: 'army-size', measure: params!.measure, countFor: params!.countFor, top: params!.top },
        tags: params!.tags, from: at.from, to: at.to, width: at.width } }, signal, value => {
      const series = chartSummaryLines<{ tag: string; summary: ChartSummary; identities?: CountryIdentityRun[] }>((value as { series?: unknown } | null)?.series,
        line => typeof line.tag === 'string');
      if (!series) throw Error('The army summary is malformed.');
      return { ...value as object, series };
    }, track!.source, restart, publish), client);
  const summaries = useMemo(() => new Map(incoming.data?.series.map(line => [line.tag, line.summary])), [incoming.data]);
  const onSummaryView = useCallback<NonNullable<TimeChartProps['onSummaryView']>>(next => {
    if (next.to < next.from) return;
    setView(before => before?.key === key && before.from === next.from && before.to === next.to && before.width === next.width ? before : { key, ...next });
  }, [key]);
  const exactLineTags = useCallback(async (hour: number) => {
    const window = manifest?.windows.findIndex(span => span.firstHour <= hour && hour <= span.lastHour) ?? -1;
    if (!track || !params || !active || window < 0) return null;
    const controller = new AbortController(); pending.current.add(controller);
    try {
      const reply = await loadProduct(client, { sourceId: track.source.id, revision: track.source.revision, product: 'army-size',
        params: { ...params, window } }, controller.signal, validateArmySizeReply, track.source);
      if (controller.signal.aborted || alive.current !== owner) return null;
      const model = countryValuesModel(reply);
      return model.tags.filter(tag => countryValueAt(model, tag, Math.floor(hour))?.total != null);
    } catch (error) {
      if (!controller.signal.aborted && alive.current === owner) setNavigationError({ owner, error: asFailure(error) });
      return null;
    } finally { pending.current.delete(controller); }
  }, [owner, manifest, active]);
  const onNavigateHour = useCallback<NonNullable<TimeChartProps['onNavigateHour']>>((action, from, x, until) => {
    if (!track || !params || !active) return;
    setNavigationError(undefined);
    navigation.current?.abort();
    const controller = new AbortController(); pending.current.add(controller); navigation.current = controller;
    void preparedSeriesQuery(client, track.source, 'series-next-change', { branchId: params.branchId,
      family: { kind: 'army-size', measure: params.measure, countFor: params.countFor, top: params.top }, tags: params.tags,
      key: action, from, firstHour: x[0], lastHour: Math.min(x[1], until ?? x[1]) }, controller.signal).then(value => {
      if (!controller.signal.aborted && alive.current === owner) setKeyboard(previous => ({ owner, hour: (value as { hour: number | null }).hour, revision: (previous?.revision ?? 0) + 1 }));
    }).catch(error => { if (!controller.signal.aborted && alive.current === owner) setNavigationError({ owner, error: asFailure(error) }); }).finally(() => pending.current.delete(controller));
  }, [owner, active]);
  const retry = () => { setNavigationError(undefined); incoming.retry(); };
  const navigationFailure = useHeldFailure(navigationError?.owner === owner ? navigationError.error : undefined,
    incoming.data !== undefined || manifest !== undefined, retry);
  return { dense, summaries, identities: useMemo(() => incoming.data?.series.map(line => ({ tag: line.tag, identities: line.identities ?? [] })), [incoming.data]), exactLineTags, onNavigateHour, keyboardHour: keyboard?.owner === owner ? keyboard.hour : undefined,
    keyboardRevision: keyboard?.owner === owner ? keyboard.revision : undefined,
    onSummaryView: client.preparedQuery ? onSummaryView : undefined, ...incoming,
    error: incoming.error ?? navigationFailure, retry };
}
