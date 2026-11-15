import { useCallback, useRef, useState } from 'react';

/** State set from a callback a list or chart calls on each render or frame (the rows it drew, the windows they need, the
 * view it shows, what is hovered): set only when `same` (a stable function) says it changed. An updater is resolved
 * against the value held. An update resolving to the state held still renders its owner, which bails out and commits
 * with nothing performed: a commit while playing that no part of the page owns. */
export function useChangedState<T>(initial: T, same: (held: T, next: T) => boolean = Object.is): [T, (next: T | ((held: T) => T)) => void] {
  const [value, setValue] = useState(initial);
  const held = useRef(value);
  held.current = value;
  const set = useCallback((next: T | ((held: T) => T)) => {
    const resolved = typeof next === 'function' ? (next as (held: T) => T)(held.current) : next;
    if (same(held.current, resolved)) return;
    held.current = resolved;
    setValue(() => resolved);
  }, [same]);
  return [value, set];
}
