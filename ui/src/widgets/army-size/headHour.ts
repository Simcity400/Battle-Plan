import { useCallback, useRef, useSyncExternalStore } from 'react';

export interface HeadClock {
  getHour(): number;
  getSnapshot(): { hoursPerSecond: number; playing: boolean };
  subscribeFrame(listener: () => void): () => void;
  subscribe(listener: () => void): () => void;
}

type Held = { key: string; hour: number };
export function heldAt(previous: Held | null, key: string, hour: number): Held {
  return previous?.key === key && previous.hour <= hour ? previous : { key, hour };
}

export function useHeadHour(clock: HeadClock, enabled: boolean, key: (hour: number) => string, woken?: Set<() => void>, frames = true): number {
  const keyed = useRef(key); keyed.current = key;
  const held = useRef<{ key: string; hour: number } | null>(null);
  const subscribe = useCallback((changed: () => void) => {
    if (!enabled) return () => {};
    const stops = [...frames ? [clock.subscribeFrame(changed)] : [], clock.subscribe(changed)];
    woken?.add(changed);
    return () => {
      for (const stop of stops) stop();
      woken?.delete(changed);
    };
  }, [clock, enabled, woken, frames]);
  const read = () => {
    if (!enabled) return null;
    const hour = clock.getHour(), next = pausedKey(keyed.current(hour), hour, clock.getSnapshot().playing);
    return held.current = heldAt(held.current, next, hour);
  };
  useSyncExternalStore(subscribe, read, read);
  return enabled && held.current ? held.current.hour : clock.getHour();
}

export const pausedKey = (key: string, hour: number, playing: boolean) => `${key}|${playing ? '' : Math.floor(hour)}`;
