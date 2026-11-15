import { useMemo } from 'react';

/** The clock a list reveals the rows it holds ahead by, each frame: the hour reached (on the list's own timeline) and
 * whether that hour moves forward on its own (playing), which is when a revealed row enters with motion. */
export interface RevealClock {
  hour(): number;
  playing(): boolean;
  /** Told every frame while playing and at every anchor (play, pause, seek, step). */
  subscribe(listener: () => void): () => void;
}

/** The parts of the dashboard's clock a reveal reads. */
export interface RevealSource {
  getHour(): number;
  getSnapshot(): { playing: boolean };
  subscribeFrame(listener: () => void): () => void;
  subscribe(listener: () => void): () => void;
}

/** `clock` as a reveal clock, its hour mapped onto the list's timeline by `at` (the clock's hour as is when absent). */
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

/** The reveal clock of a list following `clock` (`at` kept stable by its owner), none when it does not follow. */
export function useRevealClock(clock: RevealSource, following: boolean, at?: (clockHour: number) => number): RevealClock | undefined {
  return useMemo(() => following ? revealClock(clock, at) : undefined, [clock, following, at]);
}

/** Playing, how much playback a list draws its rows ahead for: a list renders once per block, not per row. */
export const AHEAD_SECONDS = 2;
/** The most rows a block draws ahead of its first hour, so the render that draws them stays within a frame. */
export const AHEAD_ROWS = 120;

/** The end (exclusive) of the block of hours a list renders at `base`: rows revealed after `base` and before the end are
 * drawn ahead and revealed by the frame loop. Playing at `hoursPerSecond`, the block spans `AHEAD_SECONDS` of playback,
 * ended earlier at the hour of the `AHEAD_ROWS`-th row past `base` (`reveals` ascending); paused, it ends at the next
 * row, so a scrub inside the same rows renders nothing. Only the first `known` of `reveals` are known; unless
 * `complete` says they are every row there is, the block ends at the last known row's hour, whose fellows at that hour
 * may not be known yet. Always past `base`. */
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
