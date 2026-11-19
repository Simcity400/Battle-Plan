import { flushSync } from 'react-dom';
import { busy } from '../shell/busy.ts';
import { engine } from '../engine/frame.ts';

const queued: (() => void)[] = [];
const urgent: (() => void)[] = [];
let framed = false;
/** In flight while updates wait for their frame. */
let waiting: (() => void) | null = null;

/** The page's one schedule for data updates (state set from a reply): every update queued before the
 * engine's next commit phase renders in one batch, so React commits once a frame however many
 * replies land. Without frames (outside a page), at once. */
export function inFrame(update: () => void): void {
  if (typeof requestAnimationFrame !== 'function') { update(); return; }
  queue(queued, update);
}

/** As `inFrame`, but rendered at the frame's own priority, never as a transition the frame's other renders (a block of
 * a list while playing) put off for frames: for what a row in view waits on, such as its name landing. */
export function inFrameNow(update: () => void): void {
  if (typeof requestAnimationFrame !== 'function') { update(); return; }
  queue(urgent, update);
}

function queue(into: (() => void)[], update: () => void): void {
  into.push(update);
  waiting ??= busy('updates');
  if (!framed) { framed = true; engine.next('commit', flush); }
}

function flush(): void {
  framed = false;
  const end = waiting;
  waiting = null;
  const updates = [...urgent.splice(0), ...queued.splice(0)];
  const committed = busy('commits');
  try { flushSync(() => { for (const run of updates) run(); }); }
  finally { committed(); end?.(); }
}
