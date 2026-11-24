import type { Live } from '../../charts/live';
import type { FollowEntry } from './store';

export function liveFollowed<T>(entry: FollowEntry, read: (entry: FollowEntry) => T): Live<T> {
  let made: { version: number; value: T } | null = null;
  return {
    read: () => {
      if (made?.version !== entry.version) made = { version: entry.version, value: read(entry) };
      return made.value;
    },
    subscribe: changed => entry.subscribe(changed),
  };
}

