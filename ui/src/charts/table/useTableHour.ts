import { useMemo, useSyncExternalStore } from 'react';
import type { PlaybackController } from '../../dashboard/playback';

/** One shared playback clock; at most ten table updates/second while playing.
 * A seek or pause publishes immediately, with the exact hour. */
export function useTableHour(clock: PlaybackController, follow: boolean): number {
  const store = useMemo(() => {
    let hour = clock.getHour();
    let last = 0;
    return {
      get: () => hour,
      subscribe: (notify: () => void) => {
        if (!follow) return () => {};
        let timer: ReturnType<typeof setTimeout> | undefined;
        const publish = () => { timer = undefined; last = performance.now(); hour = clock.getHour(); notify(); };
        const stop = clock.subscribe(() => {
          if (!clock.getSnapshot().playing || performance.now() - last >= 100) {
            clearTimeout(timer); publish();
          } else timer ??= setTimeout(publish, 100 - (performance.now() - last));
        });
        publish();
        return () => { clearTimeout(timer); stop(); };
      },
    };
  }, [clock, follow]);
  return useSyncExternalStore(store.subscribe, store.get, store.get);
}
