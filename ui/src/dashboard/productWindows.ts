import { type Failure, asFailure } from './queryError.ts';
import { withWindowEntries } from './windowMap.ts';
import { useEffect, useMemo, useState } from 'react';
import type { DashboardClient, DashboardSource, ProductRequest } from './client';
import { loadProduct } from './products.ts';
import { useHeldFailures } from './heldFailures.ts';
import { inFrame } from './frameUpdates.ts';

export interface ProductWindows<T> {
  data: ReadonlyMap<number, T>;
  errors: ReadonlyMap<number, Failure>;
  retry: () => void;
}

export interface Held<T> { identity: string | null; data: ReadonlyMap<number, T>; errors: ReadonlyMap<number, Failure> }

const withEntries = withWindowEntries;

function sameOnRerun<S extends object>(update: (previous: S) => S): (previous: S) => S {
  const made = new WeakMap<S, S>();
  return previous => {
    let result = made.get(previous);
    if (!result) made.set(previous, result = update(previous));
    return result;
  };
}


export function keepWanted<T>(held: Held<T>, identity: string | null, wanted: ReadonlySet<number>): Held<T> {
  if (held.identity !== identity) return { identity, data: new Map(), errors: new Map() };
  const keep = <V>(map: ReadonlyMap<number, V>) => [...map.keys()].every(window => wanted.has(window)) ? map : new Map([...map].filter(([window]) => wanted.has(window)));
  const [data, errors] = [keep(held.data), keep(held.errors)];
  return data === held.data && errors === held.errors ? held : { identity, data, errors };
}

export const MAX_WINDOW_CONCURRENCY = 4;

export function runBoundedQueue<T>(items: readonly T[], limit: number, run: (item: T) => Promise<void>): void {
  const cap = Math.max(1, Math.floor(limit));
  let next = 0;
  let active = 0;
  const pump = () => {
    while (active < cap && next < items.length) {
      const item = items[next++];
      active++;
      void run(item).finally(() => {
        active--;
        pump();
      });
    }
  };
  pump();
}

export function useProductWindows<T>(client: DashboardClient, identity: string | null, request: (window: number) => ProductRequest,
  wanted: readonly number[], active: boolean, validate?: (data: unknown) => T, source?: DashboardSource, progressive = false): ProductWindows<T> {
  const owner = useMemo(() => ({ client, identity }), [client, identity]);
  const [held, setHeld] = useState<Held<T> & { owner: typeof owner; attempt: number }>({ owner, identity, data: new Map(), errors: new Map(), attempt: 0 });
  const current = held.owner === owner ? held : null;
  const attempt = current?.attempt ?? 0;
  const key = wanted.join(',');
  useEffect(() => {
    if (!active || identity == null) return;
    const controller = new AbortController();
    const want = new Set(wanted);
    const asking = [...want].filter(window => !current?.errors.has(window) && !current?.data.has(window));
    const release = (previous: Held<T> & { owner: typeof owner; attempt: number }) => {
      if (previous.owner !== owner) return { owner, identity, data: new Map<number, T>(), errors: new Map<number, Failure>(), attempt: 0 };
      const kept = keepWanted(previous, identity, want);
      return kept === previous ? previous : { ...kept, owner, attempt: previous.attempt };
    };
    if (!asking.length || current === null) setHeld(sameOnRerun(release));
    const arrived: { data: [number, T][]; errors: [number, Failure][] } = { data: [], errors: [] };
    let scheduled = false;
    const commit = () => {
      scheduled = false;
      if (!arrived.data.length && !arrived.errors.length) return;
      const data = arrived.data.splice(0), errors = arrived.errors.splice(0);
      setHeld(sameOnRerun(previous => {
        if (previous.owner !== owner) return previous;
        const kept = release(previous);
        return { ...kept, data: withEntries(kept.data, data), errors: withEntries(kept.errors, errors) };
      }));
    };
    const arrive = (add: () => void) => {
      if (controller.signal.aborted) return;
      add();
      if (!scheduled) { scheduled = true; inFrame(commit); }
    };
    runBoundedQueue(asking, MAX_WINDOW_CONCURRENCY, window => {
      if (controller.signal.aborted) return Promise.resolve();
      const publish = progressive ? (data: T) => arrive(() => arrived.data.push([window, data])) : undefined;
      return loadProduct<T>(client, request(window), controller.signal, validate, source, attempt > 0, publish, undefined, true).then(
        data => arrive(() => arrived.data.push([window, data])),
        error => arrive(() => arrived.errors.push([window, asFailure(error)])),
      );
    });
    return () => { commit(); controller.abort(); };
  }, [owner, key, active, attempt]);
  const data = current?.data;
  const errors = current?.errors;
  const retry = useMemo(() => () => {
    setHeld(sameOnRerun(previous => previous.owner !== owner ? previous : { ...previous, errors: new Map(), attempt: previous.attempt + 1 }));
  }, [owner]);
  const heldErrors = data && errors ? [...errors].flatMap(([window, error]) => data.has(window) ? [error] : []) : [];
  const framed = useHeldFailures(heldErrors, retry) && heldErrors.length > 0;
  return useMemo(() => ({
    data: data ?? new Map<number, T>(),
    errors: errors && framed ? new Map([...errors].filter(([window]) => !data?.has(window))) : errors ?? new Map<number, Failure>(),
    retry,
  }), [data, errors, framed, retry]);
}
