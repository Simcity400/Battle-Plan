import { useLayoutEffect, useRef, useState } from 'react';
import { commitRows, EMPTY_HISTORY, nextDrop, shownRows } from './rowMotion.ts';
import { inFrame } from '../dashboard/frameUpdates.ts';
import type { MovingRow, RowHistory } from './rowMotion';

export const LEAVE_MS = 450;

export function useRowMotion<T>(items: readonly { key: string; item: T }[], quiet = false): readonly MovingRow<T>[] {
  const history = useRef<RowHistory<T>>(EMPTY_HISTORY);
  const [clock, setClock] = useState(0);
  const rows = shownRows(history.current, items, clock, LEAVE_MS);
  useLayoutEffect(() => {
    history.current = commitRows(history.current, items, performance.now(), LEAVE_MS, clock);
    let timer: number | undefined;
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
    schedule();
    return () => window.clearTimeout(timer);
  }, [items, clock, quiet]);
  return rows;
}

export const movingRowClass = (row: MovingRow<unknown>) => `moving-row${row.phase === 'entering' ? ' is-entering' : row.phase === 'leaving' ? ' is-leaving' : ''}`;
