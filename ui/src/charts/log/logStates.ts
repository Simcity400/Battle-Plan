import type { LogRow } from './logData';

type Mutable<T> = { -readonly [K in keyof T]: T[K] };

/** A state's text as a short key part: the same text, the same part, in every block. */
function digest(text: string): string {
  let hash = 0x811c9dc5;
  for (let at = 0; at < text.length; at++) { hash ^= text.charCodeAt(at); hash = Math.imul(hash, 0x01000193); }
  return (hash >>> 0).toString(36);
}

/** The rows of a list whose rows change as it plays, over a block of hours: each row's states, every one from the
 * hour it shows (`revealedHour`) to the hour its next state or its leaving replaces it (`leftHour`), so the list draws
 * the block once and its reveal clock shows each state at its hour. A state's key is its row's key and what `state`
 * says of it, so the same state keeps its key in every block (none enters again when the next block draws it); its id
 * is its row's identity, so a `LogList` given `states` draws the states as one row and its tooltip and keyboard follow
 * that row, always at the state its clock shows. */
export class LogStates<T> {
  private readonly held = new Map<string, { row: LogRow<T>; state: string; out: Mutable<LogRow<T>> }>();
  private readonly out: Mutable<LogRow<T>>[] = [];
  private readonly keys = new Set<string>();
  private readonly state: (row: LogRow<T>) => string;
  constructor(state: (row: LogRow<T>) => string) { this.state = state; }

  /** What the row `identity` shows from `hour` (null: it is not listed), a state of its own where it differs from the
   * one held. The block's first rows are opened with `hour` null: they show from their own reveal. */
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

  /** The identities held now. */
  identities(): IterableIterator<string> { return this.held.keys(); }

  /** Every state, ordered by `order` (each hour's rows in its order, as `order` sees each row alone), then by reveal. */
  rows(order: (a: LogRow<T>, b: LogRow<T>) => number): LogRow<T>[] {
    const shown = (row: LogRow<T>) => row.revealedHour ?? -Infinity;
    return [...this.out].sort((a, b) => order(a, b) || shown(a) - shown(b));
  }
}

/** The rows of a list over the block `hours` (ascending, its first hour first), read whole at each (`rowsAt`): see
 * `LogStates`. A row's identity is its id, else its key. */
export function logStates<T>(hours: readonly number[], rowsAt: (hour: number) => readonly LogRow<T>[], state: (row: LogRow<T>) => string,
  order: (a: LogRow<T>, b: LogRow<T>) => number): LogRow<T>[] {
  return logStatesBy(hours, rowsAt, () => null, () => null, state, order);
}

/** As `logStates`, read whole only at the block's first hour and where `changed` says any row may change (null); at
 * every other hour only the rows it names are read again (`rowOf`, null where the row is not listed then), so a block
 * costs its rows once plus one read per change, not its rows at every hour. */
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

/** The change hours after `hour` and before `limit`, at most `count`, from `next` (the first change after an hour). */
export function changesAfter(next: (hour: number) => number, hour: number, limit: number, count: number): number[] {
  const hours: number[] = [];
  for (let at = next(hour); at < limit && hours.length < count; at = next(at)) hours.push(at);
  return hours;
}
