import { createContext, useContext, useEffect, useState } from 'react';

interface HeldFailure { error: string; retry: () => void }
const NONE: readonly string[] = [];

/** Widget frames report later failures without hiding already published replies. */
export class HeldFailures {
  readonly entries = new Map<object, readonly HeldFailure[]>();
  readonly listeners = new Set<() => void>();
  errors: readonly string[] = [];

  put(owner: object, failures: readonly HeldFailure[]): void {
    if (failures.length) this.entries.set(owner, failures);
    else if (!this.entries.delete(owner)) return;
    const errors = [...new Set([...this.entries.values()].flatMap(list => list.map(entry => entry.error)))];
    if (errors.length === this.errors.length && errors.every((error, index) => error === this.errors[index])) return;
    this.errors = errors;
    for (const listener of this.listeners) listener();
  }

  retry(error: string): void {
    for (const list of [...this.entries.values()]) if (list.some(entry => entry.error === error)) list[0].retry();
  }

  readonly subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };

  readonly read = () => this.errors;
}

export const HeldFailuresContext = createContext<HeldFailures | null>(null);

/** Returns false outside a widget frame, where readers must report their own failures. */
export function useHeldFailures(errors: readonly string[], retry: () => void): boolean {
  const failures = useContext(HeldFailuresContext);
  const [owner] = useState(() => ({}));
  const key = JSON.stringify(failures ? [...new Set(errors)] : NONE);
  // The retry is rebuilt each render; the frame hears only a changed list of errors.
  useEffect(() => { failures?.put(owner, (JSON.parse(key) as string[]).map(error => ({ error, retry }))); }, [failures, owner, key, retry]);
  useEffect(() => () => failures?.put(owner, []), [failures, owner]);
  return !!failures;
}

/** Framed readers delegate held-data failures so their error state cannot hide published data. */
export function useHeldFailure<E extends string | null | undefined>(error: E, held: boolean, retry: () => void): E | undefined {
  const beside = typeof error === 'string' && held ? error : undefined;
  const framed = useHeldFailures(beside === undefined ? NONE : [beside], retry);
  return framed && beside !== undefined ? undefined : error;
}
