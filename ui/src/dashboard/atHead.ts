import { useCallback, useMemo, useRef, useSyncExternalStore } from 'react';
import { useContentRevision } from './contentRevision.ts';
import { useHeldTimeline } from './countryNamesAt.ts';
import { identitySince } from './countryTimeline.ts';
import type { DashboardClient } from './client';
import type { BoundTrack } from './model';
import type { PlaybackController } from './playback';
import { useFallbackHour } from './follow/useFollow.ts';
import { atNextSlot, structureOpen } from './structurePace.ts';

const unsubscribed = () => () => {};

/** A value at the clock's head that changes rarely, such as a label: `read` takes the clock's hour
 * every frame (and at each `wake`), and the component renders only when what it gives changes (`same`), while playing
 * only in the page's structure slot (`structureOpen`). Off, it is read as rendered. */
export function useAtHead<T>(clock: PlaybackController, on: boolean, read: (hour: number) => T, same: (a: T, b: T) => boolean = Object.is,
  wake?: (changed: () => void) => () => void): T {
  const shown = useRef<{ value: T } | null>(null);
  const held = useRef<{ listeners: Set<() => void>; wake: (() => void) | null }>({ listeners: new Set(), wake: null });
  const subscribe = useCallback((changed: () => void) => {
    const pending = held.current;
    pending.listeners.add(changed);
    const stops = [clock.subscribeFrame(changed), ...wake ? [wake(changed)] : []];
    return () => {
      for (const stop of stops) stop();
      pending.listeners.delete(changed);
      if (!pending.listeners.size) { pending.wake?.(); pending.wake = null; }
    };
  }, [clock, wake]);
  const snapshot = () => {
    const value = read(clock.getHour());
    if (shown.current && same(shown.current.value, value)) return shown.current.value;
    if (shown.current && !structureOpen(clock.getSnapshot().playing)) {
      const pending = held.current;
      pending.wake ??= atNextSlot(() => { pending.wake = null; for (const changed of [...pending.listeners]) changed(); });
      return shown.current.value;
    }
    shown.current = { value };
    return value;
  };
  return useSyncExternalStore(on ? subscribe : unsubscribed, snapshot, snapshot);
}

/** The hour a held panel's rows are chosen at: its last push's hour `at`, moved to the last of `changes` (where the
 * chosen countries' membership or coverage changes) its values are current through (`through`), so a panel held
 * unchanged across a join, a leave or a gap is chosen again there with no push. */
export function choiceHour(at: number, through: number | null, changes: readonly number[]): number {
  if (through === null || through <= at) return at;
  let low = 0, high = changes.length - 1, found = at;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (changes[middle] <= through) { found = Math.max(found, changes[middle]); low = middle + 1; } else high = middle - 1;
  }
  return found;
}

/** `choiceHour` at the head: read each frame and as the panel's values move (`wake`), rendering only when it moves. `at`
 * is read then too: a panel landed again unchanged at an earlier hour (a seek back) moves it with no render of its own. */
export function useChoiceHour(clock: PlaybackController, on: boolean, at: () => number, through: () => number | null, changes: readonly number[] | undefined,
  wake?: (changed: () => void) => () => void): number {
  const sorted = useMemo(() => [...changes ?? []].sort((a, b) => a - b), [changes]);
  return useAtHead(clock, on, () => choiceHour(at(), through(), sorted), Object.is, wake);
}

/** A key of a track hour that changes only where one of `tags`' names, colours or flags does, from the held country
 * timeline (`country-facts`); null where the timeline does not hold the hour. Renders once the timeline arrives. */
export function useIdentityKey(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, on: boolean): (hour: number, tags: readonly string[]) => string | null {
  const content = useContentRevision(client);
  const timeline = useHeldTimeline(client, track, content, on);
  return useCallback((hour: number, tags: readonly string[]) => {
    const since = timeline && identitySince(timeline, hour, tags);
    return since === undefined || since === null ? null : String(since);
  }, [timeline]);
}

/** The track hour a widget following the head reads `tags`' names, colours and flags at: the last hour
 * at or before the head at which one of them changed, from the held country timeline (`country-facts`),
 * so it moves, and the widget renders, only when an identity changes. Where the timeline does not hold
 * the head, the fallback's clock. `trackHour` maps the clock's hour to the track's; null outside it. */
export function useIdentityHour(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, tags: readonly string[],
  clock: PlaybackController, on: boolean, trackHour: (clockHour: number) => number | null): number | null {
  const content = useContentRevision(client);
  const timeline = useHeldTimeline(client, track, content, on);
  const settled = useAtHead(clock, on, hour => {
    const at = trackHour(hour);
    return at === null ? null : (timeline ? identitySince(timeline, at, tags) : undefined) ?? Number.NaN;
  });
  const unheld = on && Number.isNaN(settled);
  const paced = useFallbackHour(clock, unheld);
  return !on || unheld ? trackHour(paced) : settled;
}
