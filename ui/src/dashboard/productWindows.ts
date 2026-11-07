import { type Failure, asFailure } from './queryError.ts';
import { withWindowEntries } from './windowMap.ts';
import { useEffect, useMemo, useState } from 'react';
import type { DashboardClient, DashboardSource, ProductRequest } from './client';
import { loadProduct } from './products.ts';
import { preparationCoverage } from './preparedPrefix.ts';
import { useHeldFailures } from './heldFailures.ts';
import { inFrame } from './frameUpdates.ts';

/** A product asked for one fixed window of hours at a time (such as a month of a
 * country's daily reads), so no reply grows with the campaign. */
export interface ProductWindows<T> {
  /** The windows wanted and loaded, by index. */
  data: ReadonlyMap<number, T>;
  /** Framed windows with published prefixes report failures through the frame, not this map. */
  errors: ReadonlyMap<number, Failure>;
  /** Asks again for every failed window wanted. */
  retry: () => void;
}

/** The windows loaded (or failed) for one identity. */
export interface Held<T> { identity: string | null; data: ReadonlyMap<number, T>; errors: ReadonlyMap<number, Failure> }

export interface WindowDisplay<T> { identity: string; data: T; hour: number }

/** Keep the previous window at its own hour while the next is loading. A new
 * recording/content identity never borrows the previous recording's rows. */
export function windowDisplay<T>(previous: WindowDisplay<T> | null, identity: string | null, data: T | undefined, hour: number): WindowDisplay<T> | null {
  if (identity === null) return null;
  if (data === undefined) return previous?.identity === identity ? previous : null;
  return previous?.identity === identity && previous.data === data && previous.hour === hour ? previous : { identity, data, hour };
}

export function useWindowDisplay<T>(identity: string | null, data: T | undefined, hour: number, client?: DashboardClient): WindowDisplay<T> | null {
  const owner = useMemo(() => ({ client, identity }), [client, identity]);
  const [held, setHeld] = useState<{ owner: typeof owner; display: WindowDisplay<T> | null }>({ owner, display: null });
  const shown = windowDisplay(held.owner === owner ? held.display : null, identity, data, hour);
  useEffect(() => { setHeld(previous => previous.owner === owner && previous.display === shown ? previous : { owner, display: shown }); }, [shown, owner]);
  return shown;
}

const withEntries = withWindowEntries;

/** Whether a prepared reply's coverage is still partial (it names some, not complete). */
const partial = (data: unknown) => preparationCoverage(data) !== undefined && preparationCoverage(data)?.complete !== true;

/** What is held once `wanted` is all that is wanted: nothing of another identity, and
 * none of the windows no longer wanted; the same object when nothing changes, so a
 * render that wants the same windows is not repeated. */
export function keepWanted<T>(held: Held<T>, identity: string | null, wanted: ReadonlySet<number>): Held<T> {
  if (held.identity !== identity) return { identity, data: new Map(), errors: new Map() };
  const keep = <V>(map: ReadonlyMap<number, V>) => [...map.keys()].every(window => wanted.has(window)) ? map : new Map([...map].filter(([window]) => wanted.has(window)));
  const [data, errors] = [keep(held.data), keep(held.errors)];
  return data === held.data && errors === held.errors ? held : { identity, data, errors };
}

/** Loads the `wanted` windows of a product (`request` gives each window's request;
 * `identity` names everything else it asks, so a new one starts afresh) through the
 * shared product cache, and holds them while they are wanted: a window wanted ahead
 * (the next month during playback) is ready the moment it is shown, with no load in
 * between. Windows no longer wanted are released. With `progressive`, a window
 * answered from its prepared prefix is held and replaced as that prefix grows. */
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
    // Release what is no longer wanted; a new identity starts afresh.
    setHeld(previous => {
      if (previous.owner !== owner) return { owner, identity, data: new Map(), errors: new Map(), attempt: 0 };
      const kept = keepWanted(previous, identity, want);
      return kept === previous ? previous : { ...kept, owner, attempt: previous.attempt };
    });
    // Windows landing before the next frame (a tick while playing) commit together: a widget
    // asking for a whole campaign renders once a frame, not once a window.
    const arrived: { data: [number, T][]; errors: [number, Failure][] } = { data: [], errors: [] };
    let scheduled = false;
    const commit = () => {
      scheduled = false;
      if (!arrived.data.length && !arrived.errors.length) return;
      const data = arrived.data.splice(0), errors = arrived.errors.splice(0);
      setHeld(previous => previous.owner !== owner ? previous
        : { ...previous, data: withEntries(previous.data, data), errors: withEntries(previous.errors, errors) });
    };
    const arrive = (add: () => void) => {
      if (controller.signal.aborted) return;
      add();
      if (!scheduled) { scheduled = true; inFrame(commit); }
    };
    for (const window of want) {
      // A window held from its prepared prefix is asked again until complete.
      if (current?.errors.has(window) || current?.data.has(window) && !(progressive && partial(current.data.get(window)))) continue;
      const publish = progressive ? (data: T) => arrive(() => arrived.data.push([window, data])) : undefined;
      void loadProduct<T>(client, request(window), controller.signal, validate, source, attempt > 0, publish, undefined, true).then(
        data => arrive(() => arrived.data.push([window, data])),
        error => arrive(() => arrived.errors.push([window, asFailure(error)])),
      );
    }
    // Replies that landed before this request set changed still count.
    return () => { commit(); controller.abort(); };
    // The identity and the wanted windows name every request; `request` is rebuilt each render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [owner, key, active, attempt]);
  const data = current?.data;
  const errors = current?.errors;
  // A failed load already left the shared cache; retrying only asks again.
  const retry = useMemo(() => () => {
    setHeld(previous => previous.owner !== owner ? previous : { ...previous, errors: new Map(), attempt: previous.attempt + 1 });
  }, [owner]);
  const heldErrors = data && errors ? [...errors].flatMap(([window, error]) => data.has(window) ? [error] : []) : [];
  const framed = useHeldFailures(heldErrors, retry) && heldErrors.length > 0;
  return useMemo(() => ({
    data: data ?? new Map<number, T>(),
    errors: errors && framed ? new Map([...errors].filter(([window]) => !data?.has(window))) : errors ?? new Map<number, Failure>(),
    retry,
  }), [data, errors, framed, retry]);
}
