import { type Failure, failureKey, failureWire, queryFailure } from './queryError.ts';
import { createContext, useContext, useEffect, useState } from 'react';

/** A failure a widget frame shows; without `retry` it is not one to try again (a toggle flip, which a repeat could undo). */
interface HeldFailure { error: Failure; retry?: () => void }
const NONE: readonly Failure[] = [];

/** Widget frames report later failures without hiding already published replies. */
export class HeldFailures {
  readonly entries = new Map<object, readonly HeldFailure[]>();
  readonly listeners = new Set<() => void>();
  errors: readonly Failure[] = [];

  put(owner: object, failures: readonly HeldFailure[]): void {
    if (failures.length) this.entries.set(owner, failures);
    else if (!this.entries.delete(owner)) return;
    const errors = [...new Map([...this.entries.values()].flatMap(list => list.map(entry => [failureKey(entry.error), entry.error] as const))).values()];
    if (errors.length === this.errors.length && errors.every((error, index) => failureKey(error) === failureKey(this.errors[index]))) return;
    this.errors = errors;
    for (const listener of this.listeners) listener();
  }

  retry(error: Failure): void {
    for (const list of [...this.entries.values()]) if (list.some(entry => failureKey(entry.error) === failureKey(error))) list[0].retry?.();
  }

  /** Whether some holder of `error` can try it again. */
  retryable(error: Failure): boolean {
    return [...this.entries.values()].some(list => list[0].retry && list.some(entry => failureKey(entry.error) === failureKey(error)));
  }

  readonly subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };

  readonly read = () => this.errors;
}

export const HeldFailuresContext = createContext<HeldFailures | null>(null);

/** Returns false outside a widget frame, where readers must report their own failures. */
export function useHeldFailures(errors: readonly Failure[], retry: () => void): boolean {
  const failures = useContext(HeldFailuresContext);
  const [owner] = useState(() => ({}));
  const key = JSON.stringify(failures ? errors.map(failureWire) : NONE);
  // The retry is rebuilt each render; the frame hears only a changed list of errors.
  useEffect(() => { failures?.put(owner, (JSON.parse(key) as import('../generated/dashboard').QueryError[]).map(error => ({ error: queryFailure(error), retry }))); }, [failures, owner, key, retry]);
  useEffect(() => () => failures?.put(owner, []), [failures, owner]);
  return !!failures;
}

/** Framed readers delegate held-data failures so their error state cannot hide published data. */
export function useHeldFailure<E extends Failure | null | undefined>(error: E, held: boolean, retry: () => void): E | undefined {
  const beside = error instanceof Error && held ? error : undefined;
  const framed = useHeldFailures(beside === undefined ? NONE : [beside], retry);
  return framed && beside !== undefined ? undefined : error;
}
