import type { LogRow } from './logData';

type Mutable<T> = { -readonly [K in keyof T]: T[K] };

function digest(text: string): string {
  let hash = 0x811c9dc5;
  for (let at = 0; at < text.length; at++) { hash ^= text.charCodeAt(at); hash = Math.imul(hash, 0x01000193); }
  return (hash >>> 0).toString(36);
}

export class LogStates<T> {
  private readonly held = new Map<string, { row: LogRow<T>; state: string; out: Mutable<LogRow<T>> }>();
  private readonly out: Mutable<LogRow<T>>[] = [];
  private readonly keys = new Set<string>();
  private readonly state: (row: LogRow<T>) => string;
  constructor(state: (row: LogRow<T>) => string) { this.state = state; }

  set(identity: string, row: LogRow<T> | null, hour: number | null): void {
    const before = this.held.get(identity);
    if (!before && !row) return;
    const state = row ? this.state(row) : '';
    if (before && row && before.row.key === row.key && before.state === state) return;
    if (before) {
      if (hour !== null) before.out.leftHour = hour;
      this.held.delete(identity);
    }
    if (!row) return;
    const base = `${row.key}#${digest(state)}`;
    let key = base;
    for (let repeat = 2; this.keys.has(key); repeat++) key = `${base}~${repeat}`;
    this.keys.add(key);
    const out: Mutable<LogRow<T>> = { ...row, key, id: identity };
    if (hour !== null) out.revealedHour = hour;
    this.out.push(out);
    this.held.set(identity, { row, state, out });
  }

  identities(): IterableIterator<string> { return this.held.keys(); }

  rows(order: (a: LogRow<T>, b: LogRow<T>) => number): LogRow<T>[] {
    const shown = (row: LogRow<T>) => row.revealedHour ?? -Infinity;
    return [...this.out].sort((a, b) => order(a, b) || shown(a) - shown(b));
  }
}

export function logStates<T>(hours: readonly number[], rowsAt: (hour: number) => readonly LogRow<T>[], state: (row: LogRow<T>) => string,
  order: (a: LogRow<T>, b: LogRow<T>) => number): LogRow<T>[] {
  return logStatesBy(hours, rowsAt, () => null, () => null, state, order);
}

export function logStatesBy<T>(hours: readonly number[], rowsAt: (hour: number) => readonly LogRow<T>[], changed: (hour: number) => Iterable<string> | null,
  rowOf: (identity: string, hour: number) => LogRow<T> | null, state: (row: LogRow<T>) => string, order: (a: LogRow<T>, b: LogRow<T>) => number): LogRow<T>[] {
  const states = new LogStates<T>(state);
  hours.forEach((hour, step) => {
    const some = step === 0 ? null : changed(hour);
    if (some) { for (const identity of some) states.set(identity, rowOf(identity, hour), hour); return; }
    const listed = new Set<string>();
    for (const row of rowsAt(hour)) {
      const identity = row.id ?? row.key;
      listed.add(identity);
      states.set(identity, row, step === 0 ? null : hour);
    }
    if (step > 0) for (const identity of [...states.identities()]) if (!listed.has(identity)) states.set(identity, null, hour);
  });
  return states.rows(order);
}
