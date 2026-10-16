import { useCallback, useMemo, useState } from 'react';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import type { HourSpan, PoliticsLedgerReply } from '../../generated/dashboard';
import { useProduct, useProducts } from '../../dashboard/products';
import { preparedSeriesQuery } from '../../dashboard/preparedSeries';
import { validatePowerBalance, powerBalanceIndex } from '../../dashboard/powerBalance';
import { isChartSummary } from '../../charts/chartSummary';
import type { PowerCurrency } from '../../dashboard/politicsSpending';
import type { PowerBreakdown } from './powerDrillData';
import { powerCountryLedger } from './powerCountryData';

const NO_PREPARED_CLIENT = {} as DashboardClient;

function validate(value: unknown): PoliticsLedgerReply {
  const reply = validatePowerBalance(value);
  if (reply.partSummaries !== undefined && (!Array.isArray(reply.partSummaries) || !reply.partSummaries.every(part => part
    && typeof part.id === 'string' && typeof part.label === 'string' && isChartSummary(part.summary)))) {
    throw new Error('The prepared income and causes are malformed.');
  }
  return reply;
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
  const summaryRequest = enabled ? { sourceId: source.id, revision: source.revision, product: 'politics-ledger',
    params: { branchId, tag, view: 'balance', hour: Math.ceil(view.to), summary: { from: view.from, to: view.to, width: view.width, currency, mode }, bindings: binding } } : null;
  const geometry = useProduct<PoliticsLedgerReply>(requestClient, summaryRequest, active && enabled, validate, source);
  const requests = enabled ? asked.flatMap(hour => [...bindings!].flatMap(([tag, within]) => {
    const from = Math.floor(hour / 24) * 24, to = Math.min(hour, Math.floor(through));
    return to < from ? [] : [{ sourceId: source.id, revision: source.revision, product: 'politics-ledger',
      params: { branchId, tag, view: 'balance', from, hour: to, within } }];
  })) : [];
  const exact = useProducts<PoliticsLedgerReply>(requestClient, requests, active && enabled, validate);
  const indexes = useMemo(() => {
    const result = new Map<number, ReturnType<typeof powerBalanceIndex>>();
    if (!enabled || !bindings) return result;
    let offset = 0;
    for (const hour of asked) {
      const from = Math.floor(hour / 24) * 24, to = Math.min(hour, Math.floor(through));
      if (to < from) continue;
      const replies = exact.data?.slice(offset, offset + bindings!.size); offset += bindings!.size;
      const reply = replies && powerCountryLedger(replies, bindings!, tag, countryId!);
      if (reply) result.set(hour, powerBalanceIndex(reply, currency));
    }
    return result;
  }, [exact.data, owner, JSON.stringify(asked), through]); // eslint-disable-line react-hooks/exhaustive-deps
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
