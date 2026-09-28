import { useEffect, useLayoutEffect, useState } from 'react';
import { commitRows, EMPTY_HISTORY, nextDrop, shownRows } from './rowMotion.ts';
import type { MovingRow, RowHistory } from './rowMotion';

/** How long a row that is gone takes to fade out; the `.moving-row` styles use it too. */
export const LEAVE_MS = 450;

/** Rows that slide to their new place, flash in and fade out (`nextRows`). Pass the
 * rows in their order each render (the same array while they are unchanged); draw
 * each at its `slot` with the `.moving-row` classes (`movingRowClass`).
 *
 * Rendering only reads committed state (`shownRows`): the history is updated after a
 * commit (`commitRows`, in a layout effect), so a StrictMode double render shows a new
 * row entering on both passes and an abandoned render leaves no trace. */
export function useRowMotion<T>(items: readonly { key: string; item: T }[]): MovingRow<T>[] {
  const [history, setHistory] = useState<RowHistory<T>>(EMPTY_HISTORY);
  // The time rows are faded against: moved on by the drop timer, never read in render.
  const [clock, setClock] = useState(0);
  const rows = shownRows(history, items, clock, LEAVE_MS);
  useLayoutEffect(() => {
    setHistory(previous => commitRows(previous, items, performance.now(), LEAVE_MS));
  }, [items, clock]);
  // A fading row is dropped once it has faded.
  const due = nextDrop(history.rows, LEAVE_MS);
  useEffect(() => {
    if (due == null) return;
    const timer = window.setTimeout(() => setClock(performance.now()), Math.max(0, due - performance.now()) + 16);
    return () => window.clearTimeout(timer);
  }, [due]);
  return rows;
}

/** The classes of a moving row in its phase. */
export const movingRowClass = (row: MovingRow<unknown>) => `moving-row${row.phase === 'entering' ? ' is-entering' : row.phase === 'leaving' ? ' is-leaving' : ''}`;
