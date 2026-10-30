import { startTransition } from 'react';

/** The longest a queued update waits when frames are suspended (a hidden window). */
const MOST_WAIT_MS = 250;

const queued: (() => void)[] = [];
let waiting = false;

/** Runs `update` (state set from a reply) with every other update queued before the next
 * animation frame, in one transition: replies landing one after another render in one
 * commit, which input and a newer hour interrupt. Without frames (outside a page), at once. */
export function inFrame(update: () => void): void {
  if (typeof requestAnimationFrame !== 'function') { startTransition(update); return; }
  queued.push(update);
  if (waiting) return;
  waiting = true;
  let frame = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const flush = () => {
    if (!waiting) return;
    if (typeof cancelAnimationFrame === 'function') cancelAnimationFrame(frame);
    clearTimeout(timer);
    waiting = false;
    const updates = queued.splice(0);
    startTransition(() => { for (const run of updates) run(); });
  };
  frame = requestAnimationFrame(flush);
  timer = setTimeout(flush, MOST_WAIT_MS);
}
