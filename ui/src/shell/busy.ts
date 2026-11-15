/** What the page has in flight, always counted: host requests not yet answered, data updates queued for a frame and the
 * commit they render in, chart draws asked for and not yet run, followed answers not yet painted or not yet at the hour
 * sought, and the deliberate waits before an update. Zero means the page has settled. The acceptance suite reads it each
 * frame as `window.__busy`. */
export type BusyKind = 'requests' | 'updates' | 'commits' | 'draws' | 'follow' | 'waits';

const counts: Record<BusyKind, number> = { requests: 0, updates: 0, commits: 0, draws: 0, follow: 0, waits: 0 };
let total = 0;
/** The prepare lane's requests for what is not shown: apart from the count, as nobody waits on them. */
let preparing = 0;
let background = false;

/** Runs `run`, every request it starts before returning counted as the prepare lane's. */
export function inBackground<T>(run: () => T): T {
  const before = background;
  background = true;
  try { return run(); } finally { background = before; }
}

/** Counts one thing of `kind` in flight until the returned function is called; calling it again does nothing. */
export function busy(kind: BusyKind): () => void {
  const prepared = kind === 'requests' && background;
  if (prepared) preparing++; else { counts[kind]++; total++; }
  let ended = false;
  return () => {
    if (ended) return;
    ended = true;
    if (prepared) preparing--; else { counts[kind]--; total--; }
  };
}

/** How many of the prepare lane's requests are in flight. */
export const busyPreparing = (): number => preparing;

/** How many things are in flight. */
export const busyCount = (): number => total;

/** How many of each kind are in flight. */
export const busyKinds = (): Record<BusyKind, number> => ({ ...counts });

/** `run` after `ms`, counted as in flight while it waits; the returned function cancels it. */
export function busyWait(run: () => void, ms: number): () => void {
  const end = busy('waits');
  const timer = setTimeout(() => { end(); run(); }, ms);
  return () => { clearTimeout(timer); end(); };
}

if (typeof window !== 'undefined') {
  Object.defineProperty(window, '__busy', { value: Object.freeze({ count: busyCount, kinds: busyKinds, preparing: busyPreparing }), configurable: true });
}
