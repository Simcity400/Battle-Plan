export type BusyKind = 'requests' | 'updates' | 'commits' | 'draws' | 'follow' | 'waits';

const counts: Record<BusyKind, number> = { requests: 0, updates: 0, commits: 0, draws: 0, follow: 0, waits: 0 };
let total = 0;
let preparing = 0;
let background = false;
const foregroundRequests = new Set<() => void>();

export function onForegroundRequest(listener: () => void): () => void {
  foregroundRequests.add(listener);
  return () => { foregroundRequests.delete(listener); };
}

export function inBackground<T>(run: () => T): T {
  const before = background;
  background = true;
  try { return run(); } finally { background = before; }
}

export function busy(kind: BusyKind): () => void {
  const prepared = kind === 'requests' && background;
  if (prepared) preparing++;
  else {
    counts[kind]++; total++;
    if (kind === 'requests') for (const listener of foregroundRequests) listener();
  }
  let ended = false;
  return () => {
    if (ended) return;
    ended = true;
    if (prepared) preparing--; else { counts[kind]--; total--; }
  };
}

export const busyPreparing = (): number => preparing;

export const busyCount = (): number => total;

export const busyKinds = (): Record<BusyKind, number> => ({ ...counts });

export function busyWait(run: () => void, ms: number): () => void {
  const end = busy('waits');
  const timer = setTimeout(() => { end(); run(); }, ms);
  return () => { clearTimeout(timer); end(); };
}

if (typeof window !== 'undefined') {
  Object.defineProperty(window, '__busy', { value: Object.freeze({ count: busyCount, kinds: busyKinds, preparing: busyPreparing }), configurable: true });
}
