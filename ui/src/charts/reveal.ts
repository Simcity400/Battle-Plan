import { useMemo } from 'react';

export interface RevealClock {
  hour(): number;
  playing(): boolean;
  subscribe(listener: () => void): () => void;
}

export interface RevealSource {
  getHour(): number;
  getSnapshot(): { playing: boolean };
  subscribeFrame(listener: () => void): () => void;
  subscribe(listener: () => void): () => void;
}

export function revealClock(clock: RevealSource, at?: (clockHour: number) => number): RevealClock {
  return {
    hour: () => at ? at(clock.getHour()) : clock.getHour(),
    playing: () => clock.getSnapshot().playing,
    subscribe: listener => {
      const stops = [clock.subscribeFrame(listener), clock.subscribe(listener)];
      return () => { for (const stop of stops) stop(); };
    },
  };
}

export function useRevealClock(clock: RevealSource, following: boolean, at?: (clockHour: number) => number): RevealClock | undefined {
  return useMemo(() => following ? revealClock(clock, at) : undefined, [clock, following, at]);
}

export const AHEAD_SECONDS = 2;
export const AHEAD_ROWS = 120;

export function aheadEnd(reveals: ArrayLike<number>, base: number, hoursPerSecond: number, playing: boolean, known = reveals.length, complete = true): number {
  let low = 0, high = known;
  while (low < high) {
    const middle = (low + high) >> 1;
    if (reveals[middle] <= base) low = middle + 1; else high = middle;
  }
  const last = complete ? Infinity : known > low ? reveals[known - 1] : base + 1;
  if (!playing || !(hoursPerSecond > 0)) return Math.max(base + 1, Math.min(last, low < known ? reveals[low] : Infinity));
  let end = Math.min(last, base + Math.max(1, Math.ceil(hoursPerSecond * AHEAD_SECONDS)));
  if (low + AHEAD_ROWS < known) end = Math.min(end, reveals[low + AHEAD_ROWS]);
  return Math.max(base + 1, end);
}
