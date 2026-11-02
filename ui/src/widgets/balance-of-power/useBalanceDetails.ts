import { type Failure, asFailure } from '../../dashboard/queryError.ts';
import { useEffect, useState } from 'react';
import { useContentRevision } from '../../dashboard/contentRevision';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { loadTransientProduct } from '../../dashboard/products';
import type { PoliticsStateReply } from '../../generated/dashboard';
import { validateBalanceDetails } from './balanceOfPowerData';

export interface BalanceDetails { hour: number | null; data?: PoliticsStateReply; error?: Failure; retry: () => void }

/** Every stored balance record at one hour (members, contributions and drift sources):
 * one uncached request per hour asked, only while one is asked. */
export function useBalanceDetails(runtime: WidgetRuntime, track: BoundTrack | undefined, hour: number | null, active: boolean): BalanceDetails {
  const revision = useContentRevision(runtime.client);
  const [attempt, setAttempt] = useState(0);
  const request = track && hour !== null && active ? { sourceId: track.source.id, revision: track.source.revision, product: 'politics-state',
    params: { branchId: track.branch.id, hour, view: 'balances' } } : null;
  const key = JSON.stringify([request, revision, attempt]);
  const [state, setState] = useState<{ client: WidgetRuntime['client']; key: string; data?: PoliticsStateReply; error?: Failure }>();
  useEffect(() => {
    if (!request || hour === null) return;
    const controller = new AbortController();
    void loadTransientProduct<unknown>(runtime.client, request, controller.signal).then(reply => {
      const data = validateBalanceDetails(reply, hour);
      if (!controller.signal.aborted) setState({ client: runtime.client, key, data });
    }).catch(error => {
      if (!controller.signal.aborted) setState({ client: runtime.client, key, error: asFailure(error) });
    });
    return () => controller.abort();
    // Identity includes the hour, branch, source revision, content binding and retry.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runtime.client, key]);
  const current = state?.client === runtime.client && state.key === key ? state : undefined;
  return { hour, data: current?.data, error: current?.error, retry: () => setAttempt(value => value + 1) };
}

/** `hour` once it has rested for `delay` ms (at once when `now`), so playback asks
 * for an hour's details only where it pauses. */
export function useSettledHour(hour: number | null, delay: number, now: boolean): number | null {
  const [settled, setSettled] = useState(hour);
  useEffect(() => {
    if (now || hour === null) { setSettled(hour); return; }
    const timer = setTimeout(() => setSettled(hour), delay);
    return () => clearTimeout(timer);
  }, [hour, delay, now]);
  return now ? hour : settled;
}
