import { useEffect, useMemo, useRef, useState } from 'react';
import type { DashboardClient, DashboardSource, ProductRequest } from '../../dashboard/client';
import { inFrame } from '../../dashboard/frameUpdates.ts';
import { useHeldFailures } from '../../dashboard/heldFailures.ts';
import { loadProduct } from '../../dashboard/products';
import { MAX_WINDOW_CONCURRENCY, type ProductWindows } from '../../dashboard/productWindows.ts';
import { type Failure, asFailure } from '../../dashboard/queryError.ts';
import { withWindowEntries } from '../../dashboard/windowMap.ts';

export interface FrameWindows<T> extends ProductWindows<T> { holds(window: number): boolean; check(): void }

interface Held<T> { owner: object; data: ReadonlyMap<number, T>; errors: ReadonlyMap<number, Failure>; attempt: number }

export function useFrameWindows<T>(client: DashboardClient, identity: string | null, request: (window: number) => ProductRequest,
  wanted: () => readonly number[], subscribe: (changed: () => void) => () => void, active: boolean,
  validate?: (data: unknown) => T, source?: DashboardSource): FrameWindows<T> {
  const owner = useMemo(() => ({ client, identity }), [client, identity]);
  const [held, setHeld] = useState<Held<T>>({ owner, data: new Map(), errors: new Map(), attempt: 0 });
  const current = held.owner === owner ? held : null;
  const shown = useRef(current);
  shown.current = current;
  const latest = useRef({ request, wanted, validate, source });
  latest.current = { request, wanted, validate, source };
  const check = useRef<() => void>(() => {});
  const attempt = current?.attempt ?? 0;
  useEffect(() => {
    if (!active || identity == null) return;
    const controller = new AbortController();
    const inFlight = new Map<number, AbortController>();
    const queued: number[] = [];
    const arrived: { data: [number, T][]; errors: [number, Failure][] } = { data: [], errors: [] };
    let scheduled = false;
    const commit = () => {
      scheduled = false;
      if (!arrived.data.length && !arrived.errors.length) return;
      const data = arrived.data.splice(0), errors = arrived.errors.splice(0);
      const want = new Set(latest.current.wanted());
      const keep = <V>(map: ReadonlyMap<number, V>) => new Map([...map].filter(([window]) => want.has(window)));
      setHeld(previous => previous.owner !== owner ? { owner, data: withWindowEntries(new Map(), data), errors: withWindowEntries(new Map(), errors), attempt: 0 }
        : { ...previous, data: withWindowEntries(keep(previous.data), data), errors: withWindowEntries(keep(previous.errors), errors) });
    };
    const pump = () => {
      if (controller.signal.aborted) return;
      while (inFlight.size < MAX_WINDOW_CONCURRENCY && queued.length > 0) {
        const window = queued.shift()!;
        const now = shown.current;
        if (inFlight.has(window) || now?.data.has(window) || now?.errors.has(window)) continue;
        const item = new AbortController();
        inFlight.set(window, item);
        void loadProduct<T>(client, latest.current.request(window), item.signal, latest.current.validate, latest.current.source, attempt > 0, undefined, undefined, true).then(
          data => { if (!item.signal.aborted && !controller.signal.aborted) { arrived.data.push([window, data]); if (!scheduled) { scheduled = true; inFrame(commit); } } },
          error => { if (!item.signal.aborted && !controller.signal.aborted) { arrived.errors.push([window, asFailure(error)]); if (!scheduled) { scheduled = true; inFrame(commit); } } },
        ).finally(() => {
          inFlight.delete(window);
          pump();
        });
      }
    };
    const run = () => {
      const now = shown.current;
      const wantedList = latest.current.wanted();
      const want = new Set(wantedList);
      for (const [window, item] of inFlight) {
        if (!want.has(window)) {
          item.abort();
          inFlight.delete(window);
        }
      }
      for (let i = queued.length - 1; i >= 0; i--) {
        if (!want.has(queued[i])) queued.splice(i, 1);
      }
      const pending = new Set(queued);
      for (const window of wantedList) {
        if (inFlight.has(window) || pending.has(window) || now?.data.has(window) || now?.errors.has(window)) continue;
        queued.push(window);
        pending.add(window);
      }
      pump();
    };
    check.current = run;
    run();
    const stop = subscribe(run);
    return () => {
      stop();
      controller.abort();
      for (const item of inFlight.values()) item.abort();
      inFlight.clear();
      queued.length = 0;
      check.current = () => {};
    };
  }, [owner, active, attempt]);
  useEffect(() => { check.current(); });
  const data = current?.data, errors = current?.errors;
  const retry = useMemo(() => () => {
    setHeld(previous => previous.owner !== owner ? previous : { ...previous, errors: new Map(), attempt: previous.attempt + 1 });
  }, [owner]);
  const heldErrors = data && errors ? [...errors].flatMap(([window, error]) => data.has(window) ? [error] : []) : [];
  const framed = useHeldFailures(heldErrors, retry) && heldErrors.length > 0;
  return useMemo(() => ({
    data: data ?? new Map<number, T>(),
    errors: errors && framed ? new Map([...errors].filter(([window]) => !data?.has(window))) : errors ?? new Map<number, Failure>(),
    retry,
    holds: (window: number) => !!shown.current?.data.has(window),
    check: () => check.current(),
  }), [data, errors, framed, retry]);
}
