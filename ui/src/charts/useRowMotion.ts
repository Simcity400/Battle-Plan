import { useLayoutEffect, useRef, useState } from 'react';
import { commitRows, EMPTY_HISTORY, nextDrop, shownRows } from './rowMotion.ts';
import { inFrame } from '../dashboard/frameUpdates.ts';
import type { MovingRow, RowHistory } from './rowMotion';

/** How long a row that is gone takes to fade out; the `.moving-row` styles use it too. */
export const LEAVE_MS = 450;

/** Rows that slide to their new place, flash in and fade out (`nextRows`). Pass the
 * rows in their order each render (the same array while they are unchanged); draw
 * each at its `slot` with the `.moving-row` classes (`movingRowClass`).
 *
 * Rendering only reads committed state (`shownRows`): the history is updated after a
 * commit (`commitRows`, in a layout effect), so a StrictMode double render shows a new
 * row entering on both passes and an abandoned render leaves no trace. The history is
 * held in a ref: what a render drew is already what the commit records, so recording it
 * renders nothing again. Only a faded row's drop renders again, unless `quiet`: then the
 * faded row (invisible and inert, `.is-leaving`) is dropped from the history and leaves
 * the page with the next render, never with a commit of its own. */
export function useRowMotion<T>(items: readonly { key: string; item: T }[], quiet = false): readonly MovingRow<T>[] {
  const history = useRef<RowHistory<T>>(EMPTY_HISTORY);
  // The time rows are faded against: moved on by the drop timer.
  const [clock, setClock] = useState(0);
  const rows = shownRows(history.current, items, clock, LEAVE_MS);
  useLayoutEffect(() => {
    // Faded by the render's own clock: the commit keeps every row it drew, and the timer drops the rest.
    history.current = commitRows(history.current, items, performance.now(), LEAVE_MS, clock);
    let timer: number | undefined;
    // A fading row is dropped once it has faded.
    const schedule = () => {
      const due = nextDrop(history.current.rows, LEAVE_MS);
      if (due == null) return;
      timer = window.setTimeout(() => {
        if (!quiet) { inFrame(() => setClock(performance.now())); return; }
        const now = performance.now();
        history.current = commitRows(history.current, items, now, LEAVE_MS, now);
        schedule();
      }, Math.max(0, due - performance.now()) + 16);
    };
    // The drop renders with the next frame's updates, not as a commit of its own.
    schedule();
    return () => window.clearTimeout(timer);
  }, [items, clock, quiet]);
  return rows;
}

/** The classes of a moving row in its phase. */
export const movingRowClass = (row: MovingRow<unknown>) => `moving-row${row.phase === 'entering' ? ' is-entering' : row.phase === 'leaving' ? ' is-leaving' : ''}`;
