import type { SetStateAction } from 'react';

export interface Latest<T> { current: T }

export function keepLatest<T>(latest: Latest<T>, show: (value: T) => void): (next: SetStateAction<T>) => void {
  return next => {
    latest.current = typeof next === 'function' ? (next as (previous: T) => T)(latest.current) : next;
    show(latest.current);
  };
}
