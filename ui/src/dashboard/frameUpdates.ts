import { startTransition } from 'react';

/** The longest a queued update waits when neither a frame nor a tick comes (a hidden window): past
 * two ticks, so a playing clock's own tick always comes first. */
const MOST_WAIT_MS = 500;

const queued: (() => void)[] = [];
let frame = 0;
let timer: ReturnType<typeof setTimeout> | undefined;
/** The clocks playing on this page: while one plays, updates render only with its ticks. */
const playing = new Set<object>();

/** The page's one schedule for data updates (state set from a reply). While a clock plays,
 * queued updates render with its next tick, the hour and every reply landed since in one
 * transition, so React commits once a tick however many replies land. Otherwise they render
 * at the next animation frame, so interaction answers at once. Without frames (outside a
 * page), at once. */
export function inFrame(update: () => void): void {
  if (typeof requestAnimationFrame !== 'function') { startTransition(update); return; }
  queued.push(update);
  arm();
}

/** One tick of a playing clock, or an anchor of any clock (a seek, a pause): renders every
 * queued update with `tell` (the clock telling its hour) in one transition. */
export function tick(tell?: () => void): void {
  const updates = queued.splice(0);
  disarm();
  if (!updates.length && !tell) return;
  startTransition(() => {
    for (const run of updates) run();
    tell?.();
  });
}

/** Whether updates wait for a tick. */
export const updatesQueued = () => queued.length > 0;

/** `clock` started or stopped playing. */
export function clockPlaying(clock: object, on: boolean): void {
  if (on) playing.add(clock); else playing.delete(clock);
  if (queued.length) arm();
}

function arm(): void {
  timer ??= setTimeout(() => tick(), MOST_WAIT_MS);
  if (!playing.size && !frame) frame = requestAnimationFrame(() => { frame = 0; tick(); });
}

function disarm(): void {
  if (frame) cancelAnimationFrame(frame);
  clearTimeout(timer);
  frame = 0;
  timer = undefined;
}
