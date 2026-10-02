import { useEffect, useMemo, useState } from 'react';
import type { DashboardClient, ProductRequest } from './client';
import { loadProduct } from './products.ts';

/** A product asked for one fixed window of hours at a time (such as a month of a
 * country's daily reads), so no reply grows with the campaign. */
export interface ProductWindows<T> {
  /** The windows wanted and loaded, by index. */
  data: ReadonlyMap<number, T>;
  /** The windows wanted whose load failed, with why. */
  errors: ReadonlyMap<number, string>;
  /** Asks again for every failed window wanted. */
  retry: () => void;
}

/** The windows loaded (or failed) for one identity. */
export interface Held<T> { identity: string | null; data: Map<number, T>; errors: Map<number, string> }

/** What is held once `wanted` is all that is wanted: nothing of another identity, and
 * none of the windows no longer wanted; the same object when nothing changes, so a
 * render that wants the same windows is not repeated. */
export function keepWanted<T>(held: Held<T>, identity: string | null, wanted: ReadonlySet<number>): Held<T> {
  if (held.identity !== identity) return { identity, data: new Map(), errors: new Map() };
  const keep = <V>(map: Map<number, V>) => [...map.keys()].every(window => wanted.has(window)) ? map : new Map([...map].filter(([window]) => wanted.has(window)));
  const [data, errors] = [keep(held.data), keep(held.errors)];
  return data === held.data && errors === held.errors ? held : { identity, data, errors };
}

/** Loads the `wanted` windows of a product (`request` gives each window's request;
 * `identity` names everything else it asks, so a new one starts afresh) through the
 * shared product cache, and holds them while they are wanted: a window wanted ahead
 * (the next month during playback) is ready the moment it is shown, with no load in
 * between. Windows no longer wanted are released. */
export function useProductWindows<T>(client: DashboardClient, identity: string | null, request: (window: number) => ProductRequest,
  wanted: readonly number[], active: boolean, validate?: (data: unknown) => T): ProductWindows<T> {
  const [held, setHeld] = useState<Held<T>>({ identity, data: new Map(), errors: new Map() });
  const [attempt, setAttempt] = useState(0);
  const current = held.identity === identity ? held : null;
  const key = wanted.join(',');
  useEffect(() => {
    if (!active || identity == null) return;
    const controller = new AbortController();
    const want = new Set(wanted);
    // Release what is no longer wanted; a new identity starts afresh.
    setHeld(previous => keepWanted(previous, identity, want));
    for (const window of want) {
      if (current?.data.has(window) || current?.errors.has(window)) continue;
      void loadProduct<T>(client, request(window), controller.signal, validate).then(
        data => {
          if (controller.signal.aborted) return;
          setHeld(previous => previous.identity !== identity ? previous
            : { identity, data: new Map(previous.data).set(window, data), errors: previous.errors });
        },
        error => {
          if (controller.signal.aborted) return;
          const message = error instanceof Error ? error.message : String(error);
          setHeld(previous => previous.identity !== identity ? previous
            : { identity, data: previous.data, errors: new Map(previous.errors).set(window, message) });
        },
      );
    }
    return () => controller.abort();
    // The identity and the wanted windows name every request; `request` is rebuilt each render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, identity, key, active, attempt]);
  const data = current?.data;
  const errors = current?.errors;
  return useMemo(() => ({
    data: data ?? new Map<number, T>(),
    errors: errors ?? new Map<number, string>(),
    // A failed load already left the shared cache; retrying only asks again.
    retry: () => {
      setHeld(previous => ({ ...previous, errors: new Map() }));
      setAttempt(value => value + 1);
    },
  }), [data, errors]);
}
