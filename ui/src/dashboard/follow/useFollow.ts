import { useCallback, useContext, useId, useSyncExternalStore } from 'react';
import type { PlaybackController } from '../playback';
import { FollowedWidgetKey } from './store.ts';

export { NEAR_SCREEN, DASHBOARD_SCROLL } from './onScreen.ts';

export function usePlaybackHour(clock: PlaybackController, on = true): number {
  const subscribe = useCallback((changed: () => void) => on ? clock.subscribe(changed) : () => {}, [clock, on]);
  const read = useCallback(() => on ? clock.getHour() : null, [on, clock]);
  return useSyncExternalStore(subscribe, read, read) ?? clock.getHour();
}

export function useWidgetKey(): string {
  const own = useId();
  return useContext(FollowedWidgetKey) ?? own;
}
