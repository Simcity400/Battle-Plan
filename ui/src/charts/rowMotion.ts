/** Table rows that move like the signed-off tables (mock-up economy-1, style-5):
 * each row slides to its new place when the order changes, a new row flashes in,
 * and a row that is gone fades out where it was before it leaves. Pure: a widget
 * keeps the rows between renders and draws each at its `slot`. */

export interface MovingRow<T> {
  key: string;
  item: T;
  /** Where the row is drawn: its place in the order, or where a leaving row was. */
  slot: number;
  /** `entering` only in the update it first appears in; `leaving` until it is dropped. */
  phase: 'entering' | 'staying' | 'leaving';
  /** When a leaving row started to leave. */
  since: number;
}

/** The rows after an update: `items` in their new order, and the rows no longer in
 * it fading out for `leaveMs` from `now`. A row coming back while it fades stays.
 * On the `first` update nothing flashes in: the table is simply shown. */
export function nextRows<T>(previous: readonly MovingRow<T>[], items: readonly { key: string; item: T }[], now: number, leaveMs: number, first = false): MovingRow<T>[] {
  const remaining = new Set<string>();
  for (const row of previous) remaining.add(row.key);
  const rows: MovingRow<T>[] = items.map(({ key, item }, slot) => {
    const existed = remaining.delete(key);
    return { key, item, slot, phase: first || existed ? 'staying' : 'entering', since: now };
  });
  // Stable row keys are unique. If all previous rows remain, there are no
  // fades to inspect and no second key set to allocate on every redraw.
  if (!remaining.size) return rows;
  for (const row of previous) {
    if (!remaining.has(row.key)) continue;
    const since = row.phase === 'leaving' ? row.since : now;
    if (now - since < leaveMs) rows.push({ ...row, phase: 'leaving', since });
  }
  return rows;
}

/** When the next leaving row is due to be dropped, or null when none is leaving. */
export function nextDrop(rows: readonly MovingRow<unknown>[], leaveMs: number): number | null {
  let due: number | null = null;
  for (const row of rows) if (row.phase === 'leaving') due = Math.min(due ?? Infinity, row.since + leaveMs);
  return due;
}

/** The rows as last committed to the screen, and the items they were committed for. */
export interface RowHistory<T> { items: readonly { key: string; item: T }[] | null; rows: readonly MovingRow<T>[] }

export const EMPTY_HISTORY: RowHistory<never> = { items: null, rows: [] };

const unfaded = <T>(rows: readonly MovingRow<T>[], now: number, leaveMs: number) =>
  rows.filter(row => row.phase !== 'leaving' || now - row.since < leaveMs);

/** The rows to draw for `items`, from the committed `history` only, at `now`. Pure: a
 * render repeated (StrictMode) or abandoned (suspended) reads the same history and
 * gives the same rows, and leaves nothing behind. */
export function shownRows<T>(history: RowHistory<T>, items: readonly { key: string; item: T }[], now: number, leaveMs: number): MovingRow<T>[] {
  if (history.items === items) return unfaded(history.rows, now, leaveMs);
  return nextRows(history.rows, items, now, leaveMs, history.items == null);
}

/** The history once `items` has been committed at `now` (from an effect): the rows
 * drawn become the next update's past, and faded rows are dropped. Unchanged when
 * nothing moved, so committing twice is harmless. */
export function commitRows<T>(history: RowHistory<T>, items: readonly { key: string; item: T }[], now: number, leaveMs: number): RowHistory<T> {
  if (history.items !== items) return { items, rows: nextRows(history.rows, items, now, leaveMs, history.items == null) };
  const rows = unfaded(history.rows, now, leaveMs);
  return rows.length === history.rows.length ? history : { items, rows };
}
