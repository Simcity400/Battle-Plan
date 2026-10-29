import { useCallback, useMemo, useState } from 'react';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import type { LedgerGovernment, MeasureDropReply, PoliticsLedgerReply } from '../../generated/dashboard';
import type { MarkCluster } from '../../charts/markSummary';
import { loadProduct, useProduct } from '../../dashboard/products';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useHeldFailure } from '../../dashboard/heldFailures';
import { useViewRead } from '../../dashboard/viewRead';
import { preparedSeriesQuery } from '../../dashboard/preparedSeries';
import { object, unsigned64 } from '../../dashboard/validation';
import { validateGovernmentLedger } from './governmentData';

const empty = (branchId: string, tag: string): PoliticsLedgerReply => ({ branchId, tag, decimals: 5, warnings: [],
  spending: [], spendItems: [], spendCategories: [], spendingCoverage: [], ideaTooltips: null, commandHolds: [] });

/** Native marks follow the viewport; supplying dates gate only reached facts. */
export function usePreparedGovernments(client: DashboardClient, source: DashboardSource | undefined, branchId: string | undefined,
  tag: string | undefined, extent: readonly [number, number], through: number, active: boolean) {
  const enabled = !!client.preparedQuery && !!source && !!branchId && !!tag && through >= extent[0] && through <= extent[1];
  const owner = useMemo(() => ({ client, source, branchId, tag }), [client, source, branchId, tag]);
  const [viewport, setViewport] = useState<{ owner: typeof owner; from: number; to: number; width: number }>();
  const view = viewport?.owner === owner ? viewport : { from: extent[0], to: Math.min(extent[1], through), width: 1 };
  const exact = view.to - view.from + 1 <= view.width;
  // One read at a time while the chart's view and the hour shown move (`useViewRead`).
  // Exact and marked replies differ in kind, so crossing between them starts a new lane.
  const content = useContentRevision(client);
  // The viewport bounds the native dates; the reached read alone gates supplying reads, so
  // a fact already supplied stays shown however the view moves.
  const asked = { from: view.from, to: view.to, width: view.width, hour: Math.floor(through) };
  const readable = enabled && Math.ceil(asked.from) <= asked.hour;
  const geometryRead = useViewRead<typeof asked, PoliticsLedgerReply>(readable ? JSON.stringify([source.id, source.revision, branchId, tag, exact, content]) : null,
    asked, JSON.stringify([asked.from, asked.to, asked.width, asked.hour]), active && readable, (at, signal, publish, restart) => loadProduct(client,
      { sourceId: source!.id, revision: source!.revision, product: 'politics-ledger', params: { branchId, tag, view: 'governments', from: Math.ceil(at.from), hour: at.hour,
        ...(at.to - at.from + 1 <= at.width ? { matches: true, within: [{ firstHour: Math.ceil(at.from), lastHour: Math.floor(at.to) }] }
          : { marks: { from: at.from, to: at.to, width: at.width } }) } },
      signal, value => validateGovernmentLedger(value, { branchId: branchId!, tag: tag! }), source, restart, publish), client);
  const geometry = { ...geometryRead, error: useHeldFailure(geometryRead.error, geometryRead.data !== undefined, geometryRead.retry) };
  const [hover, setHover] = useState<{ owner: typeof owner; hour: number | null }>();
  const reached = hover?.owner === owner ? hover.hour : null;
  const hoverRequest = enabled && reached != null ? { sourceId: source.id, revision: source.revision, product: 'politics-ledger',
    params: { branchId, tag, view: 'governments', from: Math.floor(reached / 24) * 24, hour: Math.floor(Math.min(reached, through)) } } : null;
  const details = useProduct<PoliticsLedgerReply>(client, hoverRequest, active && enabled, value => validateGovernmentLedger(value, { branchId: branchId!, tag: tag! }), source);
  const [picked, setPicked] = useState<{ owner: typeof owner; through: number; cluster: MarkCluster }>();
  const cluster = picked?.owner === owner && picked.through === through ? picked.cluster : undefined;
  const [position, setPosition] = useState<{ cluster: MarkCluster; cursors: string[] }>();
  const cursors = cluster && position?.cluster === cluster ? position.cursors : cluster ? [cluster.firstOrdinal] : [];
  const cursor = cursors.at(-1) ?? '0';
  const pageRequest = enabled && cluster?.tag === 'native' ? { sourceId: source.id, revision: source.revision, product: 'politics-ledger',
    params: { branchId, tag, view: 'governments', hour: Math.floor(through), page: {
      from: cluster.firstHour, to: cluster.lastHour, cursor, pageSize: 32,
    } } } : null;
  const page = useProduct<MeasureDropReply>(client, pageRequest, active && enabled, value => {
    const reply = value as MeasureDropReply;
    if (!object(reply) || !unsigned64(reply.firstOrdinal) || !unsigned64(reply.total)
      || !(reply.cursor == null || unsigned64(reply.cursor) && BigInt(reply.cursor) > BigInt(cursor)) || !Array.isArray(reply.rows)
      || !reply.rows.every(row => object(row) && row.tag === tag && unsigned64(row.ordinal))) throw new Error('The government event page is malformed.');
    validateGovernmentLedger({ ...empty(branchId!, tag!), governments: reply.rows.map(row => row.row) });
    return reply;
  }, source);
  const supplemental = geometry.data?.governments ?? [];
  const first = Number(cursor);
  const end = cluster ? Number(BigInt(cluster.firstOrdinal) + BigInt(cluster.count)) : 0;
  const rows: LedgerGovernment[] | undefined = cluster?.tag === 'native' ? page.data?.rows.map(event => event.row as LedgerGovernment)
    : cluster ? supplemental.slice(first, Math.min(first + 32, end)) : undefined;
  const next = cluster?.tag === 'native' ? page.data?.cursor : first + 32 < end ? String(first + 32) : null;
  const parityKey = JSON.stringify([source?.id, source?.revision, branchId, tag, through]);
  const [parity, setParity] = useState<{ key: string; data?: PoliticsLedgerReply; error?: string }>();
  const parityRows = parity?.key === parityKey ? parity.data : undefined;
  const prepareParity = () => {
    if (import.meta.env.MODE !== 'parity' || !enabled || parity?.key === parityKey) return;
    setParity({ key: parityKey });
    void preparedSeriesQuery(client, source!, 'politics-ledger', { branchId, tag, view: 'governments', from: extent[0], hour: Math.floor(through) }, new AbortController().signal)
      .then(value => setParity({ key: parityKey, data: validateGovernmentLedger(value) }))
      .catch(error => setParity({ key: parityKey, error: String(error) }));
  };
  const onSummaryView = useCallback((view: { from: number; to: number; width: number }) => setViewport(previous => previous?.owner === owner
    && previous.from === view.from && previous.to === view.to && previous.width === view.width ? previous : { owner, ...view }), [owner]);
  const navigate = useCallback(async (key: 'ArrowLeft' | 'ArrowRight' | 'Home' | 'End', from: number | null,
    bounds: readonly [number, number], signal: AbortSignal) => {
    const value = await preparedSeriesQuery(client, source!, 'politics-ledger', { branchId, tag, view: 'governments', hour: Math.floor(through),
      navigation: { key, from, firstHour: bounds[0], lastHour: Math.min(bounds[1], through) } }, signal) as { hour: number | null };
    if (!value || !(value.hour === null || Number.isFinite(value.hour))) throw new Error('The government navigation is malformed.');
    return value.hour;
  }, [owner, through]); // eslint-disable-line react-hooks/exhaustive-deps
  return { enabled, owner, exact, view, data: geometry.data, details: details.data, parityRows, prepareParity,
    parityError: parity?.key === parityKey ? parity.error : undefined,
    error: geometry.error ?? details.error ?? page.error, retry: () => { geometry.retry(); details.retry(); page.retry(); },
    onSummaryView, onHover: (hour: number | null) => setHover(previous => previous?.owner === owner && previous.hour === hour ? previous : { owner, hour }), navigate,
    cluster, rows, onMarkHover: (cluster: MarkCluster | null) => {
      if (cluster) setPicked(previous => previous?.owner === owner && previous.through === through
        && previous.cluster.tag === cluster.tag && previous.cluster.firstOrdinal === cluster.firstOrdinal && previous.cluster.count === cluster.count
        ? previous : { owner, through, cluster });
    }, close: () => setPicked(undefined), previous: cursors.length > 1 ? () => setPosition({ cluster: cluster!, cursors: cursors.slice(0, -1) }) : undefined,
    next: next ? () => setPosition({ cluster: cluster!, cursors: [...cursors, next] }) : undefined };
}
