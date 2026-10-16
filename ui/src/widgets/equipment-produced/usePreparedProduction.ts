import { useEffect, useState } from 'react';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import { useContentRevision } from '../../dashboard/contentRevision.ts';
import { validatePreparationReply } from '../../shell/preparation/contract.ts';
import { validateProductionSelection } from './preparedProducedData.ts';
import type { ProductionSelection, ProductionSelectionRequest } from './preparedProducedData';

export interface ProductionQueryClient {
  preparedQuery<T>(source: DashboardSource, product: string, params: unknown, options: { signal: AbortSignal }): Promise<T>;
}
export function hasPreparedProduction(client: DashboardClient): client is DashboardClient & ProductionQueryClient {
  return client != null && typeof (client as Partial<ProductionQueryClient>).preparedQuery === 'function';
}

export async function queryProductionSelection(client: ProductionQueryClient, source: DashboardSource, request: ProductionSelectionRequest, signal: AbortSignal): Promise<ProductionSelection | null> {
  const value = await client.preparedQuery<unknown>(source, 'production-select', request, { signal });
  if (signal.aborted) throw new DOMException('Production query cancelled', 'AbortError');
  return value === null ? null : validateProductionSelection(value, request);
}

/** Pending presentation retains its own supplying hour and source/selector owner. */
export function usePreparedProduction(client: DashboardClient, source: DashboardSource | undefined, request: ProductionSelectionRequest | null, active: boolean) {
  const content = useContentRevision(client);
  const owner = JSON.stringify([source?.id, source?.revision, content, request?.selection, request?.ranked]);
  const requestKey = JSON.stringify(request);
  const rangeKey = JSON.stringify([request?.offset, request?.limit]);
  const [attempt, setAttempt] = useState(0);
  const [stored, setStored] = useState<{ client: DashboardClient; owner: string; rangeKey?: string; data?: ProductionSelection; hour?: number; pending: boolean; unavailable?: boolean; error?: string }>();
  const owned = stored?.client === client && stored.owner === owner ? stored : undefined;
  const covered = request != null && owned?.rangeKey === rangeKey && owned.data != null && owned.data.firstHour <= request.hour && request.hour <= owned.data.lastHour;
  useEffect(() => {
    if (!active || !source || !request || !hasPreparedProduction(client) || covered) return;
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    let started = false;
    const statusRequest = { sourceId: source.id, revision: source.revision, requestId: crypto.randomUUID() };
    setStored(previous => ({ client, owner, rangeKey: previous?.client === client && previous.owner === owner ? previous.rangeKey : undefined, data: previous?.client === client && previous.owner === owner ? previous.data : undefined,
      hour: previous?.client === client && previous.owner === owner ? previous.hour : undefined, pending: true }));
    const query = async () => {
      try {
        const data = await queryProductionSelection(client, source, request, controller.signal);
        if (controller.signal.aborted) return;
        if (data === null) {
          if (client.preparationStatus) {
            let status = validatePreparationReply(await client.preparationStatus(statusRequest, controller.signal), statusRequest);
            if (controller.signal.aborted) return;
            if (attempt > 0 && !started && client.prepareSource && ['idle', 'cancelled', 'failed', 'ready'].includes(status.state)) {
              started = true;
              status = validatePreparationReply(await client.prepareSource(statusRequest, controller.signal), statusRequest);
              if (controller.signal.aborted) return;
            }
            if (status.state === 'failed' || status.state === 'cancelled') throw new Error(status.error ?? (status.state === 'failed' ? 'Production preparation failed.' : 'Production preparation was cancelled.'));
            if (status.state === 'ready' || status.state === 'idle') { setStored({ client, owner, pending: false, unavailable: true }); return; }
          }
          timer = setTimeout(() => { void query(); }, 250); return;
        }
        setStored({ client, owner, rangeKey, data, hour: request.hour, pending: false });
      } catch (error) {
        if (!controller.signal.aborted) setStored({ client, owner, pending: false, error: error instanceof Error ? error.message : String(error) });
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
