import { useCallback, useRef, useSyncExternalStore } from 'react';
import type { PlaybackController } from './playback';

const unsubscribed = () => () => {};

export function useAtHead<T>(clock: PlaybackController, on: boolean, read: (hour: number) => T, same: (a: T, b: T) => boolean = Object.is,
  wake?: (changed: () => void) => () => void): T {
  const shown = useRef<{ value: T } | null>(null);
  const subscribe = useCallback((changed: () => void) => {
    const stop = clock.subscribeFrame(changed), off = wake?.(changed);
    return () => { stop(); off?.(); };
  }, [clock, wake]);
  const snapshot = () => {
    const value = read(clock.getHour());
    if (shown.current && same(shown.current.value, value)) return shown.current.value;
    shown.current = { value };
    return value;
  };
  return useSyncExternalStore(on ? subscribe : unsubscribed, snapshot, snapshot);
}
