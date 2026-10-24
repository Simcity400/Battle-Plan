import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import { loadProduct, useProduct } from '../../dashboard/products';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useHeldFailure } from '../../dashboard/heldFailures';
import { useViewRead } from '../../dashboard/viewRead';
import type { SupplyHealthReply } from '../../generated/dashboard';
import type { TimeChartProps } from '../../charts/TimeChart';
import { validateSupplyHealthReply } from './supplyHealthData';
import { useParityPreparation } from '../../dashboard/parityPreparation';
import { preparedSeriesQuery } from '../../dashboard/preparedSeries';
import { overlayCountryIdentities } from '../../dashboard/countryIdentity';
import type { ChartSummary } from '../../charts/chartSummary';
type PreparedReply = SupplyHealthReply & { summaries?: { tag: string; values: ChartSummary[] }[] };

/** One display viewport plus exact reached moments; source changes mask both. */
export function usePreparedSupply(client: DashboardClient, track: BoundTrack | undefined, tags: readonly string[],
  hour: number, hover: number | null, active: boolean, origin?: number, drawnTags: readonly string[] = tags) {
  const key = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, tags]);
  const owner = useMemo(() => ({ client, key }), [client, key]);
  const alive = useRef(owner); alive.current = owner;
  const navigation = useRef<AbortController | null>(null);
  useEffect(() => () => navigation.current?.abort(), [owner, active]);
  const [viewport, setViewport] = useState<{ key: string; from: number; to: number; width: number }>();
  const [keyboard, setKeyboard] = useState<{ owner: typeof owner; hour: number | null; revision: number }>();
  const [navigationError, setNavigationError] = useState<{ owner: typeof owner; error: string }>();
  const onSummaryView = useCallback<NonNullable<TimeChartProps['onSummaryView']>>(view => {
    if (view.to < view.from) return;
    const from = Math.floor(view.from), to = Math.ceil(view.to);
    setViewport(before => before?.key === key && before.from === from && before.to === to && before.width === view.width
      ? before : { key, from, to, width: view.width });
  }, [key]);
  const prepared = !!client.preparedQuery;
  const parity = import.meta.env.MODE === 'parity' ? useParityPreparation() : null;
  const view = parity?.requested && track ? { from: track.branch.firstHour, to: track.branch.lastHour, width: undefined }
    : viewport?.key === key ? viewport : { from: hour, to: hour, width: 1 };
  const request = (from: number, to: number, width?: number, selected = tags) => track && selected.length ? {
    sourceId: track.source.id, revision: track.source.revision, product: 'supply-health',
    params: { branchId: track.branch.id, tags: selected, ...(prepared ? { from, to, width } : {}) },
  } : null;
  // One display read at a time while the chart's view moves (`useViewRead`).
  const content = useContentRevision(client);
  const selected = parity?.requested || viewport?.key !== key ? tags : drawnTags;
  const read = useViewRead<{ from: number; to: number; width?: number }, PreparedReply>(
    request(view.from, view.to, view.width, selected) ? JSON.stringify([key, selected, prepared, !!parity?.requested, content]) : null,
    view, prepared ? JSON.stringify([view.from, view.to, view.width]) : '', active,
    (at, signal, publish, restart) => loadProduct(client, request(at.from, at.to, at.width, selected)!, signal,
      value => validateSupplyHealthReply(value) as PreparedReply, prepared ? track?.source : undefined, restart, publish), client);
  const display = { ...read, error: useHeldFailure(read.error, read.data !== undefined, read.retry) };
  const reached = useProduct<SupplyHealthReply>(client, request(hour, hour), active,
    validateSupplyHealthReply, prepared ? track?.source : undefined);
  const pointed = useProduct<SupplyHealthReply>(client, request(hover ?? origin ?? hour, hover ?? origin ?? hour), active,
    validateSupplyHealthReply, prepared ? track?.source : undefined);
  const data = useMemo(() => {
    const base = reached.data ?? display.data;
    if (!base) return undefined;
    const rough = display.data?.summaries?.some(line => line.values.some(summary => summary.approximate?.length)) === true;
    const replies = [display.data, reached.data, pointed.data].filter((reply): reply is PreparedReply => !!reply);
    const countries = new Map<string, SupplyHealthReply['series'][number]>();
    let identities: { tag: string; identities: NonNullable<SupplyHealthReply['series'][number]['identities']> }[] = [];
    for (const reply of replies) {
      identities = overlayCountryIdentities(identities, reply.series.flatMap(series => series.identities === undefined ? [] : [{ tag: series.tag, identities: series.identities }]));
      for (const series of reply.series) {
        const before = countries.get(series.tag);
        const runs = [...(before?.runs ?? [])];
        if (reply !== display.data || !rough) for (const run of series.runs) {
          if (!runs.some(before => before.firstHour <= run.firstHour && before.lastHour >= run.lastHour)) runs.push(run);
        }
        countries.set(series.tag, { ...series, runs: runs.sort((a, b) => a.firstHour - b.firstHour) });
      }
    }
    for (const identity of identities) {
      const country = countries.get(identity.tag);
      if (country) countries.set(identity.tag, { ...country, identities: identity.identities });
    }
    return { ...base, series: [...countries.values()] };
  }, [display.data, reached.data, pointed.data]);
  const geometry = useMemo(() => new Map(display.data?.series.map(series => [series.tag, series.runs])), [display.data]);
  const onNavigateHour = useCallback<NonNullable<TimeChartProps['onNavigateHour']>>((action, from, x, until) => {
    if (!track || !active) return;
    setNavigationError(undefined);
    navigation.current?.abort();
    const controller = new AbortController(); navigation.current = controller;
    void preparedSeriesQuery(client, track.source, 'series-next-change', { branchId: track.branch.id, family: { kind: 'supply-health' }, tags,
      key: action, from, firstHour: x[0], lastHour: Math.min(x[1], until ?? x[1]) }, controller.signal).then(value =>
      { if (!controller.signal.aborted && alive.current === owner) setKeyboard(previous => ({ owner, hour: (value as { hour: number | null }).hour, revision: (previous?.revision ?? 0) + 1 })); })
      .catch(error => { if (!controller.signal.aborted && alive.current === owner) setNavigationError({ owner, error: String(error) }); });
  }, [owner, active]);
  return { data, geometry, error: display.error ?? reached.error ?? pointed.error ?? (navigationError?.owner === owner ? navigationError.error : undefined),
    onNavigateHour: prepared ? onNavigateHour : undefined, keyboardHour: keyboard?.owner === owner ? keyboard.hour : undefined, keyboardRevision: keyboard?.owner === owner ? keyboard.revision : undefined,
    summaries: useMemo(() => new Map(display.data?.summaries?.map(item => [item.tag, item.values])), [display.data]),
    parityComplete: !!display.data && !!parity?.requested,
    parityPrepare: parity?.prepare,
    retry: () => { setNavigationError(undefined); display.retry(); reached.retry(); pointed.retry(); }, onSummaryView: prepared ? onSummaryView : undefined };
}
