import { useEffect, useMemo, useRef, useState } from 'react';
import type { DashboardClient, DashboardSource, ProductRequest } from '../../dashboard/client';
import { inFrame } from '../../dashboard/frameUpdates.ts';
import { useHeldFailures } from '../../dashboard/heldFailures.ts';
import { loadProduct } from '../../dashboard/products';
import type { ProductWindows } from '../../dashboard/productWindows';
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
    const asked = new Set<number>();
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
      for (const window of asked) if (!want.has(window) && !data.some(([at]) => at === window) && !errors.some(([at]) => at === window)) asked.delete(window);
    };
    const run = () => {
      const now = shown.current;
      for (const window of latest.current.wanted()) {
        if (asked.has(window) || now?.data.has(window) || now?.errors.has(window)) continue;
        asked.add(window);
        void loadProduct<T>(client, latest.current.request(window), controller.signal, latest.current.validate, latest.current.source, attempt > 0, undefined, undefined, true).then(
          data => { if (!controller.signal.aborted) { arrived.data.push([window, data]); if (!scheduled) { scheduled = true; inFrame(commit); } } },
          error => { if (!controller.signal.aborted) { arrived.errors.push([window, asFailure(error)]); if (!scheduled) { scheduled = true; inFrame(commit); } } },
        );
      }
    };
    check.current = run;
    run();
    const stop = subscribe(run);
    return () => { stop(); controller.abort(); check.current = () => {}; };
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
