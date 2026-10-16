import { useCallback, useEffect, useMemo, useState } from 'react';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import type { EquipmentProducedReply, ProducedParams } from '../../generated/dashboard';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { MIN_SLOT } from '../../charts/race/race';
import { dayLabel } from '../../charts/draw';
import { useViewportHeight } from '../../charts/useChartTokens';
import { formatDateTime } from '../../dashboard/calendar';
import { formatProductionAmount, productionKeyLabel, productionRaceEntries } from './preparedProducedData';
import type { ProductionSelection } from './preparedProducedData';
import { countDecimals } from './producedView';
import { usePreparedProduction } from './usePreparedProduction';

/** A single selected endpoint for the existing numeric model: counts are already
 * measured by the host, and this adapter never subtracts or extends their range. */
export function productionEndpointReply(page: ProductionSelection, params: ProducedParams): EquipmentProducedReply {
  const firstHours = [page.firstHour], lastHours = [page.lastHour];
  const coded = (amount: string | null) => ({ from: 0, deltas: amount === null ? [0] : [Number(amount)], unknown: amount === null ? [0, 1] : [] });
  const keyIndices = new Map(page.keys.map((key, index) => [key.key, index]));
  return { branchId: page.branchId, measure: params.measure, count: params.count, decimals: countDecimals(params.count), grouping: params.grouping,
    types: page.types, keys: page.keys, reads: [...new Set(page.countries.map(row => row.readHour))].sort((a, b) => a - b),
    observations: [...new Set(page.countries.flatMap(row => row.observation === null ? [] : [row.observation]))].sort((a, b) => a - b),
    known: page.known ? [{ firstHour: page.firstHour, lastHour: page.lastHour }] : [], warnings: page.warnings,
    series: page.countries.map(row => ({ tag: row.tag, firstHours, lastHours, breaks: [0], identities: [], membership: null,
      total: coded(row.total), values: row.values.map(value => ({ key: keyIndices.get(value.key)!, values: coded(value.amount), equipmentValues: coded(value.equipmentAmount) })) })) };
}

export function productionEndpointTooltips(page: ProductionSelection, params: ProducedParams, tag: string, hour: number, name: string, what: string, observation?: number | null): TooltipContent[] | null {
  if (hour < page.firstHour || hour > page.lastHour) return null;
  const country = page.countries.find(row => row.tag === tag);
  if (!country) return null;
  const decimals = countDecimals(params.count);
  const keyById = new Map(page.keys.map(key => [key.key, key]));
  const parts = country.values.map((part, order) => ({ ...part, order })).filter(part => part.amount !== '0')
    .sort((a, b) => (a.amount === null ? 1 : 0) - (b.amount === null ? 1 : 0)
      || (a.amount === null || b.amount === null || a.amount === b.amount ? 0 : BigInt(a.amount) > BigInt(b.amount) ? -1 : 1) || a.order - b.order);
  const row = (label: string, value: string | null): TooltipRow => ({ label, value: formatProductionAmount(value, decimals), ...(value === null ? { tone: 'muted' } : {}) });
  const equipment = parts.flatMap(part => part.amount !== null || part.equipmentAmount === null ? []
    : [row(`${productionKeyLabel(keyById.get(part.key)!, params.grouping, hour)} — Equipment lines`, part.equipmentAmount)]);
  const observations = page.countries.flatMap(row => row.observation === null ? [] : [row.observation]);
  const since = observation === undefined ? observations.length ? Math.max(...observations) : null : observation;
  const note = params.measure === 'total' && since !== null ? `Counted since ${formatDateTime(since)}.` : undefined;
  const base = { title: name, subtitle: `${dayLabel(hour)} · ${what}`,
    note: equipment.length ? `${note ?? ''} Total unavailable; equipment subtotal shown.`.trim() : note };
  const empty: TooltipRow[] = parts.length ? [] : [{ label: 'Nothing delivered', tone: 'muted' }];
  const total = { label: 'Total', value: formatProductionAmount(country.total, decimals), strong: true, divider: true };
  return [{ ...base, rows: [...empty, ...parts.map(part => row(productionKeyLabel(keyById.get(part.key)!, params.grouping, hour), part.amount)), ...equipment, total] },
    { ...base, rows: [...empty, total] }];
}

/** Visible endpoint demand and its current ownership; full demand is test-only. */
export function usePreparedProducedView(client: DashboardClient, source: DashboardSource | undefined, params: ProducedParams | null, hour: number, active: boolean) {
  const height = useViewportHeight();
  const pageSize = Math.max(1, Math.ceil(height / MIN_SLOT));
  const scope = JSON.stringify([source?.id, source?.revision, params, pageSize]);
  const [storedRange, setRange] = useState<{ scope: string; first: number; count: number }>();
  const range = storedRange?.scope === scope ? storedRange : { scope, first: 0, count: pageSize };
  const onRange = useCallback((first: number, count: number) => {
    if (!Number.isSafeInteger(first) || first < 0 || !Number.isSafeInteger(count) || count < 1) return;
    setRange(previous => previous?.scope === scope && previous.first === first && previous.count === count ? previous : { scope, first, count });
  }, [scope]);
  const readyParams = params && (params.top !== null || params.tags.length > 0) ? params : null;
  const result = usePreparedProduction(client, source, readyParams ? { hour, selection: readyParams, offset: String(range.first), limit: range.count, ranked: true } : null, active);
  const page = result.data;
  useEffect(() => {
    if (page && !result.pending && range.first > 0 && range.first >= page.totalCountries) setRange({ scope, first: 0, count: pageSize });
  }, [page, result.pending, range.first, scope, pageSize]);
  const moment = result.hour ?? hour;
  const decimals = params ? countDecimals(params.count) : 0;
  const reply = useMemo(() => page && params ? productionEndpointReply(page, params) : undefined,
    // The selector's value owns the model, rather than its rebuilt request object.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [page, scope]);
  const entries = useMemo(() => page ? productionRaceEntries(page, () => [], false, moment, decimals) : [], [page, moment, decimals]);
  const owner = useMemo(() => ({ client, snapshot: JSON.stringify([source?.id, source?.revision, params, page?.firstHour, page?.lastHour]) }),
    [client, source?.id, source?.revision, JSON.stringify(params), page?.firstHour, page?.lastHour]);
  const remoteRows = page ? { total: page.totalCountries, offset: Number(page.offset), min: Number(page.valueMin) / 10 ** decimals,
    max: Number(page.valueMax) / 10 ** decimals, onRange, owner } : undefined;
  let fullPage: ProductionSelection | undefined, parityComplete = false;
  let parityPreparation;
  if (import.meta.env.MODE === 'parity') {
    const [requested, setRequested] = useState(false);
    const full = usePreparedProduction(client, source, requested && readyParams && page
      ? { hour: moment, selection: readyParams, offset: '0', limit: page.totalCountries, ranked: true } : null, active);
    fullPage = full.data;
    parityComplete = requested && !!fullPage && !full.pending && fullPage.countries.length === fullPage.totalCountries;
    parityPreparation = { prepare: () => setRequested(true), ready: () => parityComplete, error: () => full.error ?? result.error ?? null };
  }
  return { ...result, page, reply, entries, remoteRows, fullPage, parityComplete, parityPreparation,
    tooltip: (tag: string, at: number, name: string, what: string, observation?: number | null) => page && params ? productionEndpointTooltips(page, params, tag, at, name, what, observation) : null };
}
