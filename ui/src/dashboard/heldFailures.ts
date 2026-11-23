import { type Failure, failureKey } from './queryError.ts';
import { createContext, useContext, useEffect, useRef, useState } from 'react';

interface HeldFailure { error: Failure; retry?: () => void }
const NONE: readonly Failure[] = [];

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

export function useHeldFailures(errors: readonly Failure[], retry: () => void): boolean {
  const failures = useContext(HeldFailuresContext);
  const [owner] = useState(() => ({}));
  const latest = useRef(errors);
  latest.current = errors;
  const key = failures ? JSON.stringify(errors.map(failureKey)) : '';
  useEffect(() => { failures?.put(owner, latest.current.map(error => ({ error, retry }))); }, [failures, owner, key, retry]);
  useEffect(() => () => failures?.put(owner, []), [failures, owner]);
  return !!failures;
}

export function useHeldFailure<E extends Failure | null | undefined>(error: E, held: boolean, retry: () => void): E | undefined {
  const beside = error instanceof Error && held ? error : undefined;
  const framed = useHeldFailures(beside === undefined ? NONE : [beside], retry);
  return framed && beside !== undefined ? undefined : error;
}
