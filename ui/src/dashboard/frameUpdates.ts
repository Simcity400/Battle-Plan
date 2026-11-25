import { flushSync } from 'react-dom';
import { busy } from '../shell/busy.ts';
import { engine } from '../engine/frame.ts';

const queued: (() => void)[] = [];
let framed = false;
let waiting: (() => void) | null = null;

export function inFrame(update: () => void): void {
  if (typeof requestAnimationFrame !== 'function') { update(); return; }
  queued.push(update);
  waiting ??= busy('updates');
  if (!framed) { framed = true; engine.next('commit', flush); }
}

function flush(): void {
  framed = false;
  const end = waiting;
  waiting = null;
  const updates = queued.splice(0);
  const committed = busy('commits');
  try { flushSync(() => { for (const run of updates) run(); }); }
  finally { committed(); end?.(); }
}
