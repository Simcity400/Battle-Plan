import type { GatedHour } from '../../dashboard/regionTime';
import { type Failure, asFailure } from '../../dashboard/queryError.ts';
import { useEffect, useState } from 'react';
import { useContentRevision } from '../../dashboard/contentRevision';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { loadTransientProduct } from '../../dashboard/products';
import { validateBalanceList } from './balanceOfPowerData';
import type { PoliticsStateAt } from '../../dashboard/politicsHistory';
import { repliedCells } from '../../dashboard/follow/liveCells.ts';
import { noValues } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import { busyWait } from '../../shell/busy.ts';
import { productRequest } from '../../dashboard/request.ts';

export interface BalanceDetails { hour: number | null; data?: PoliticsStateAt; values: LiveValues<number>; error?: Failure; retry: () => void }

const NOT_READ = noValues('Not read yet');

export function useBalanceDetails(runtime: WidgetRuntime, track: BoundTrack | undefined, hour: GatedHour | null, active: boolean): BalanceDetails {
  const revision = useContentRevision(runtime.client);
  const [attempt, setAttempt] = useState(0);
  const request = track && hour !== null && active ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'politics-state',
    params: { branchId: track.branch.id, hour, view: 'balances' } }) : null;
  const key = JSON.stringify([request, revision, attempt]);
  const [state, setState] = useState<{ client: WidgetRuntime['client']; key: string; data?: PoliticsStateAt; values?: LiveValues<number>; error?: Failure }>();
  useEffect(() => {
    if (!request || hour === null) return;
    const controller = new AbortController();
    void loadTransientProduct<unknown>(runtime.client, request, controller.signal).then(reply => {
      const data = { ...validateBalanceList(reply), hour };
      if (!controller.signal.aborted) setState({ client: runtime.client, key, data, values: repliedCells(reply).read() });
    }).catch(error => {
      if (!controller.signal.aborted) setState({ client: runtime.client, key, error: asFailure(error) });
    });
    return () => controller.abort();
  }, [runtime.client, key]);
  const current = state?.client === runtime.client && state.key === key ? state : undefined;
  return { hour, data: current?.data, values: current?.values ?? NOT_READ, error: current?.error, retry: () => setAttempt(value => value + 1) };
}

export function useSettledHour(hour: number | null, delay: number, now: boolean): number | null {
  const [settled, setSettled] = useState(hour);
  useEffect(() => {
    if (now || hour === null) { setSettled(hour); return; }
    return busyWait(() => setSettled(hour), delay);
  }, [hour, delay, now]);
  return now ? hour : settled;
}
