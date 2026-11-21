
export interface MovingRow<T> {
  key: string;
  item: T;
  slot: number;
  phase: 'entering' | 'staying' | 'leaving';
  since: number;
}

export function nextRows<T>(previous: readonly MovingRow<T>[], items: readonly { key: string; item: T }[], now: number, leaveMs: number, first = false,
  cutoff = now): MovingRow<T>[] {
  const remaining = new Set<string>();
  for (const row of previous) remaining.add(row.key);
  const rows: MovingRow<T>[] = items.map(({ key, item }, slot) => {
    const existed = remaining.delete(key);
    return { key, item, slot, phase: first || existed ? 'staying' : 'entering', since: now };
  });
  if (!remaining.size) return rows;
  for (const row of previous) {
    if (!remaining.has(row.key)) continue;
    if (row.phase !== 'leaving') { if (leaveMs > 0) rows.push({ ...row, phase: 'leaving', since: now }); }
    else if (cutoff - row.since < leaveMs) rows.push(row);
  }
  return rows;
}

export function nextDrop(rows: readonly MovingRow<unknown>[], leaveMs: number): number | null {
  let due: number | null = null;
  for (const row of rows) if (row.phase === 'leaving') due = Math.min(due ?? Infinity, row.since + leaveMs);
  return due;
}

export interface RowHistory<T> { items: readonly { key: string; item: T }[] | null; rows: readonly MovingRow<T>[] }

export const EMPTY_HISTORY: RowHistory<never> = { items: null, rows: [] };

const unfaded = <T>(rows: readonly MovingRow<T>[], now: number, leaveMs: number) =>
  rows.some(row => row.phase === 'leaving' && now - row.since >= leaveMs) ? rows.filter(row => row.phase !== 'leaving' || now - row.since < leaveMs) : rows;

export function shownRows<T>(history: RowHistory<T>, items: readonly { key: string; item: T }[], now: number, leaveMs: number): readonly MovingRow<T>[] {
  if (history.items === items) return unfaded(history.rows, now, leaveMs);
  return nextRows(history.rows, items, now, leaveMs, history.items == null);
}

export function commitRows<T>(history: RowHistory<T>, items: readonly { key: string; item: T }[], now: number, leaveMs: number,
  cutoff = now): RowHistory<T> {
  if (history.items !== items) return { items, rows: nextRows(history.rows, items, now, leaveMs, history.items == null, cutoff) };
  const rows = unfaded(history.rows, cutoff, leaveMs);
  return rows.length === history.rows.length ? history : { items, rows };
}

export function untakenRows<T extends { phase: MovingRow<unknown>['phase']; slot: number }>(rows: readonly T[]): T[] {
  const taken = new Set(rows.flatMap(row => row.phase === 'leaving' ? [] : [row.slot]));
  return rows.filter(row => row.phase !== 'leaving' || !taken.has(row.slot));
}
