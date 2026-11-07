/** Reads several widgets ask for alike (the same product at the same hour), fetched and shaped once:
 * one in flight serves every asker, and the newest landed ones are kept for a moment, so a table
 * and a race of the same answer never read and parse it twice. A read stops only when every asker
 * has stopped waiting for it. */
const KEPT = 64;
interface Shared { value: Promise<unknown>; waiting: number; controller: AbortController; landed: boolean }
const reads = new Map<string, Shared>();

/** `keep` says whether a landed value may answer later askers (a partial one, read again, may not). */
export function sharedRead<T>(key: string, signal: AbortSignal, read: (signal: AbortSignal) => Promise<T>, keep: (value: T) => boolean = () => true): Promise<T> {
  let shared = reads.get(key);
  if (!shared) {
    const controller = new AbortController();
    const entry: Shared = { value: read(controller.signal), waiting: 0, controller, landed: false };
    shared = entry;
    reads.set(key, entry);
    entry.value.then(value => {
      entry.landed = true;
      if (!keep(value as T)) { if (reads.get(key) === entry) reads.delete(key); return; }
      // Kept newest last: the oldest landed read goes first.
      if (reads.size > KEPT) for (const [old, held] of reads) { if (reads.size <= KEPT) break; if (held.landed) reads.delete(old); }
    }, () => { if (reads.get(key) === entry) reads.delete(key); });
  } else { reads.delete(key); reads.set(key, shared); }
  const held = shared;
  held.waiting++;
  return new Promise<T>((resolve, reject) => {
    let done = false;
    const leave = () => {
      if (done) return;
      done = true;
      signal.removeEventListener('abort', stop);
      if (--held.waiting === 0 && !held.landed) { held.controller.abort(); if (reads.get(key) === held) reads.delete(key); }
    };
    const stop = () => { leave(); reject(new DOMException('Read cancelled', 'AbortError')); };
    if (signal.aborted) { stop(); return; }
    signal.addEventListener('abort', stop, { once: true });
    held.value.then(value => { if (!done) { leave(); resolve(value as T); } }, error => { if (!done) { leave(); reject(error); } });
  });
}
