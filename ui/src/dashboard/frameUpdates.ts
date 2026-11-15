import { startTransition, useLayoutEffect, useState } from 'react';
import { busy } from '../shell/busy.ts';
import { atNextSlot, atPagePause, pagePlaying } from './structurePace.ts';
import { engine } from '../engine/frame.ts';

const queued: (() => void)[] = [];
const urgent: (() => void)[] = [];
let framed = false;
/** In flight while updates wait for their frame. */
let waiting: (() => void) | null = null;

/** The page's one schedule for data updates (state set from a reply): every update queued before the
 * engine's next commit phase renders from it, in one transition, so React commits once a frame however many
 * replies land. Without frames (outside a page), at once. */
export function inFrame(update: () => void): void {
  if (typeof requestAnimationFrame !== 'function') { startTransition(update); return; }
  if (pagePlaying()) { held.push(update); heldBusy ??= busy('updates'); cancelHeld ??= atNextSlot(flushHeld); return; }
  queue(queued, update);
}

/** While the page plays, `inFrame`'s updates wait for the next slot of the structure pace and run in its task, with every
 * paced structure of the page, outside a transition so React commits them together: once a slot however many replies,
 * measurements and names land (run 13: a second and third commit after each structure commit). */
const held: (() => void)[] = [];
let cancelHeld: (() => void) | null = null;
let heldBusy: (() => void) | null = null;
function flushHeld(): void {
  cancelHeld?.();
  cancelHeld = null;
  const end = heldBusy;
  heldBusy = null;
  try { for (const run of held.splice(0)) run(); } finally { end?.(); }
}
atPagePause(flushHeld);

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
  try {
    for (const run of urgent.splice(0)) run();
    const updates = queued.splice(0);
    if (!updates.length) return;
    // The transition may render over several tasks: its updates are in flight until it commits. Paused only: while the
    // page plays a mark would commit where its updates commit nothing.
    const committing = pagePlaying() ? null : busy('commits');
    startTransition(() => { try { for (const run of updates) run(); } finally { if (committing) untilCommitted(committing); } });
  } finally { end?.(); }
}

/** The mounted `FrameCommits`' state setter, and the commits it waits on by their mark. */
let mark: ((at: number) => void) | null = null;
let marks = 0;
const awaiting: { at: number; end: () => void }[] = [];

/** Ends `end` when what the transition calling it renders commits: a mark set in that transition commits with it. With
 * no `FrameCommits` mounted, at once. */
function untilCommitted(end: () => void): void {
  if (!mark) { end(); return; }
  const at = ++marks;
  awaiting.push({ at, end });
  mark(at);
}

/** Marks each commit of `inFrame`'s updates, so they count as in flight until they commit: mounted once, at the root of
 * the page. Renders nothing. */
export function FrameCommits(): null {
  const [at, setAt] = useState(0);
  useLayoutEffect(() => {
    mark = setAt;
    return () => {
      mark = null;
      for (const { end } of awaiting.splice(0)) end();
    };
  }, []);
  useLayoutEffect(() => {
    while (awaiting.length && awaiting[0].at <= at) awaiting.shift()!.end();
  }, [at]);
  return null;
}
