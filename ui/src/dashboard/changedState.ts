import { useCallback, useRef, useState } from 'react';

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
