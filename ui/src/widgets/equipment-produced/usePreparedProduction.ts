import { type Failure, asFailure } from '../../dashboard/queryError.ts';
import { useEffect, useState } from 'react';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import { useContentRevision } from '../../dashboard/contentRevision.ts';
import { validatePreparationReply } from '../../shell/preparation/contract.ts';
import { answeredHour, validateProductionSelection } from './preparedProducedData.ts';
import { gateMoved, preparedPrefix } from '../../dashboard/preparedPrefix.ts';
import type { ProductionSelection, ProductionSelectionRequest } from './preparedProducedData';

export interface ProductionQueryClient {
  preparedQuery<T>(source: DashboardSource, product: string, params: unknown,
    options: { signal: AbortSignal; gate?: (entries: readonly string[] | undefined) => void }): Promise<T>;
}
export function hasPreparedProduction(client: DashboardClient): client is DashboardClient & ProductionQueryClient {
  return client != null && typeof (client as Partial<ProductionQueryClient>).preparedQuery === 'function';
}

export async function queryProductionSelection(client: ProductionQueryClient, source: DashboardSource, request: ProductionSelectionRequest, signal: AbortSignal,
  gate?: (entries: readonly string[] | undefined) => void): Promise<ProductionSelection | null> {
  const value = await client.preparedQuery<unknown>(source, 'production-select', request, { signal, gate });
  if (signal.aborted) throw new DOMException('Production query cancelled', 'AbortError');
  return value === null ? null : validateProductionSelection(value, request);
}

/** Pending presentation retains its own supplying hour and source/selector owner.
 * A page answered from a prepared prefix shows the prefix's last hour and is
 * asked again once the coverage it waits for (its gate) moves. */
export function usePreparedProduction(client: DashboardClient, source: DashboardSource | undefined, request: ProductionSelectionRequest | null, active: boolean) {
  const content = useContentRevision(client);
  const owner = JSON.stringify([source?.id, source?.revision, content, request?.selection, request?.ranked]);
  const requestKey = JSON.stringify(request);
  const rangeKey = JSON.stringify([request?.offset, request?.limit]);
  const [attempt, setAttempt] = useState(0);
  const [stored, setStored] = useState<{ client: DashboardClient; owner: string; rangeKey?: string; data?: ProductionSelection; hour?: number; pending: boolean; unavailable?: boolean; error?: Failure }>();
  const owned = stored?.client === client && stored.owner === owner ? stored : undefined;
  const covered = request != null && owned?.rangeKey === rangeKey && owned.data != null && owned.data.firstHour <= request.hour && request.hour <= owned.data.lastHour;
  useEffect(() => {
    if (!active || !source || !request || !hasPreparedProduction(client) || covered && preparedPrefix(owned?.data)?.coverage.complete !== false) return;
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    let started = false;
    const statusRequest = { sourceId: source.id, revision: source.revision, requestId: crypto.randomUUID() };
    setStored(previous => ({ client, owner, rangeKey: previous?.client === client && previous.owner === owner ? previous.rangeKey : undefined, data: previous?.client === client && previous.owner === owner ? previous.data : undefined,
      hour: previous?.client === client && previous.owner === owner ? previous.hour : undefined, pending: true }));
    const query = async () => {
      try {
        let gate: readonly string[] | undefined;
        const data = await queryProductionSelection(client, source, request, controller.signal, entries => { gate = entries; });
        if (controller.signal.aborted) return;
        if (data === null) {
          if (client.preparationStatus) {
            let status = validatePreparationReply(await client.preparationStatus(statusRequest, controller.signal), statusRequest);
            if (controller.signal.aborted) return;
            if (attempt > 0 && !started && client.prepareSource && ['idle', 'cancelled', 'paused', 'failed', 'ready'].includes(status.state)) {
              started = true;
              status = validatePreparationReply(await client.prepareSource(statusRequest, controller.signal), statusRequest);
              if (controller.signal.aborted) return;
            }
            if (status.state === 'failed' || status.state === 'cancelled') throw new Error(status.error ?? (status.state === 'failed' ? 'Production preparation failed.' : 'Production preparation was cancelled.'));
            if (status.state === 'ready' || status.state === 'idle') { setStored({ client, owner, pending: false, unavailable: true }); return; }
          }
          timer = setTimeout(() => { void query(); }, 250); return;
        }
        setStored({ client, owner, rangeKey, data, hour: answeredHour(data, request), pending: false });
        const prefix = preparedPrefix(data);
        if (!prefix || prefix.coverage.complete || !client.preparationStatus) return;
        const follow = async () => {
          try {
            const status = validatePreparationReply(await client.preparationStatus!(statusRequest, controller.signal), statusRequest);
            if (controller.signal.aborted) return;
            // A failed or cancelled preparation leaves the prefix shown.
            if (status.state === 'failed' || status.state === 'cancelled') return;
            if (gateMoved(status, gate, request.selection.branchId, prefix.coverage)) { void query(); return; }
            timer = setTimeout(() => { void follow(); }, 500);
          } catch (error) {
            if (!controller.signal.aborted) setStored({ client, owner, pending: false, error: asFailure(error) });
          }
        };
        timer = setTimeout(() => { void follow(); }, 500);
      } catch (error) {
        if (!controller.signal.aborted) setStored({ client, owner, pending: false, error: asFailure(error) });
      }
    };
    void query();
    return () => { controller.abort(); if (timer !== undefined) clearTimeout(timer); };
    // Request identity covers source, selector, hour and pagination, without retaining metadata objects.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, source?.id, source?.revision, owner, requestKey, active, attempt]);
  return { data: owned?.data, hour: covered ? request?.hour : owned?.hour, pending: owned?.pending ?? (active && request != null), unavailable: owned?.unavailable, error: owned?.error,
    retry: () => setAttempt(value => value + 1) };
}
