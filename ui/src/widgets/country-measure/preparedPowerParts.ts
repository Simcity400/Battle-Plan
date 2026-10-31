import { useCallback, useMemo, useState } from 'react';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import type { HourSpan, PoliticsLedgerReply } from '../../generated/dashboard';
import { loadProduct, useFollowedProducts } from '../../dashboard/products';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useHeldFailure } from '../../dashboard/heldFailures';
import { useViewRead } from '../../dashboard/viewRead';
import { preparedSeriesQuery } from '../../dashboard/preparedSeries';
import { validatePowerBalance, powerBalanceIndex } from '../../dashboard/powerBalance';
import { chartSummaryFrom } from '../../charts/chartSummary';
import type { ChartSummary } from '../../charts/chartSummary';
import type { PowerCurrency } from '../../dashboard/politicsSpending';
import type { PowerBreakdown } from './powerDrillData';
import { powerCountryLedger } from './powerCountryData';

const NO_PREPARED_CLIENT = {} as DashboardClient;

type PreparedLedger = Omit<PoliticsLedgerReply, 'partSummaries'> & { partSummaries?: { id: string; label: string; summary: ChartSummary }[] };

function validate(value: unknown): PreparedLedger {
  const { partSummaries: parts, ...reply } = validatePowerBalance(value);
  if (parts === undefined) return reply;
  const malformed = () => new Error('The prepared income and causes are malformed.');
  if (!Array.isArray(parts)) throw malformed();
  const partSummaries = parts.map(part => {
    const summary = part && typeof part.id === 'string' && typeof part.label === 'string' ? chartSummaryFrom(part.summary) : null;
    if (!summary) throw malformed();
    return { ...part, summary };
  });
  return { ...reply, partSummaries };
}

/** Geometry follows the chart's viewport; exact operands follow only reached days. */
export function usePreparedPowerParts(client: DashboardClient | undefined, source: DashboardSource | undefined, branchId: string | undefined,
  tag: string, countryId: number | undefined, currency: PowerCurrency, mode: PowerBreakdown,
  bindings: ReadonlyMap<string, readonly HourSpan[]> | undefined, extent: readonly [number, number],
  hour: number | null, through: number, active: boolean) {
  const requestClient = client ?? NO_PREPARED_CLIENT;
  const enabled = !!client?.preparedQuery && !!source && !!branchId && countryId != null && !!bindings;
  const bindingKey = JSON.stringify(bindings ? [...bindings] : []);
  const owner = useMemo(() => ({ client, source, branchId, tag, countryId, currency, mode, bindingKey }),
    [client, source, branchId, tag, countryId, currency, mode, bindingKey]);
  const [viewport, setViewport] = useState<{ owner: typeof owner; from: number; to: number; width: number }>();
  const view = viewport?.owner === owner ? viewport : { from: extent[0], to: Math.min(extent[1], through), width: 1 };
  const [hover, setHover] = useState<{ owner: typeof owner; hour: number | null }>();
  const asked = [...new Set([hour, hover?.owner === owner ? hover.hour : null].filter((hour): hour is number => hour !== null).map(Math.floor))];
  const binding = Object.fromEntries(bindings ?? []);
  // One summary read at a time while the chart's view moves (`useViewRead`).
  const content = useContentRevision(requestClient);
  const read = useViewRead<{ from: number; to: number; width: number }, PreparedLedger>(
    enabled ? JSON.stringify([source.id, source.revision, branchId, tag, countryId, currency, mode, bindingKey, content]) : null,
    view, JSON.stringify([view.from, view.to, view.width]), active && enabled,
    (at, signal, publish, restart) => loadProduct(requestClient, { sourceId: source!.id, revision: source!.revision, product: 'politics-ledger',
      params: { branchId, tag, view: 'balance', hour: Math.ceil(at.to), summary: { from: at.from, to: at.to, width: at.width, currency, mode }, bindings: binding } },
    signal, validate, source, restart, publish), requestClient);
  const geometry = { ...read, error: useHeldFailure(read.error, read.data !== undefined, read.retry) };
  const requests = enabled ? asked.flatMap(hour => [...bindings!].flatMap(([tag, within]) => {
    const from = Math.floor(hour / 24) * 24, to = Math.min(hour, Math.floor(through));
    return to < from ? [] : [{ sourceId: source.id, revision: source.revision, product: 'politics-ledger',
      params: { branchId, tag, view: 'balance', from, hour: to, within } }];
  })) : [];
  const exact = useFollowedProducts<PoliticsLedgerReply>(requestClient, requests, ['from', 'hour'], active && enabled, validatePowerBalance);
  // Replies are indexed by the day they answer, which can be an earlier ask still shown.
  const indexes = useMemo(() => {
    const result = new Map<number, ReturnType<typeof powerBalanceIndex>>();
    if (!enabled || !bindings || !exact.data || !exact.requests) return result;
    const size = bindings.size;
    for (let offset = 0; offset + size <= exact.requests.length; offset += size) {
      const { from, hour: to } = exact.requests[offset].params as { from: number; hour: number };
      const reply = powerCountryLedger(exact.data.slice(offset, offset + size), bindings, tag, countryId!);
      if (!reply) continue;
      const index = powerBalanceIndex(reply, currency);
      for (const hour of asked) if (Math.floor(hour / 24) * 24 === from && Math.min(hour, Math.floor(through)) === to) result.set(hour, index);
    }
    return result;
  }, [exact.data, exact.requests, owner, JSON.stringify(asked), through]); // eslint-disable-line react-hooks/exhaustive-deps
  const onSummaryView = useCallback((next: { from: number; to: number; width: number }) => setViewport(previous => previous?.owner === owner
    && previous.from === next.from && previous.to === next.to && previous.width === next.width ? previous : { owner, ...next }), [owner]);
  const navigate = useCallback(async (key: 'ArrowLeft' | 'ArrowRight' | 'Home' | 'End', from: number | null, bounds: readonly [number, number], cutoff: number | null, signal: AbortSignal) => {
    if (!enabled) return null;
    const value = await preparedSeriesQuery(client!, source!, 'politics-ledger', { branchId, tag, view: 'balance', hour: Math.ceil(Math.min(bounds[1], cutoff ?? bounds[1])),
      bindings: binding, summary: { ...view, currency, mode }, navigation: { key, from, firstHour: bounds[0], lastHour: Math.min(bounds[1], cutoff ?? bounds[1]) } }, signal) as { hour: number | null };
    if (!value || !(value.hour === null || Number.isSafeInteger(value.hour))) throw new Error('The prepared income navigation is malformed.');
    return value.hour;
  }, [owner, view.from, view.to, view.width]); // eslint-disable-line react-hooks/exhaustive-deps
  return { enabled, owner, summaries: geometry.data?.partSummaries, at: (hour: number) => indexes.get(Math.floor(hour)),
    error: geometry.error ?? exact.error, retry: () => { geometry.retry(); exact.retry(); },
    onSummaryView, onHover: (hour: number | null) => setHover({ owner, hour }), navigate };
}
