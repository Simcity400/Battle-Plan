import { useCallback, useMemo, useRef, useSyncExternalStore } from 'react';
import { useContentRevision } from './contentRevision.ts';
import { useHeldTimeline } from './countryNamesAt.ts';
import { identitySince } from './countryTimeline.ts';
import { countryFromKey, isSlotKey } from './countryKey.ts';
import { useSlotTags } from './countrySlots.ts';
import type { SlotTags } from './countrySlots';
import type { DashboardClient } from './client';
import { type BoundTrack } from './model.ts';
import type { PlaybackController } from './playback';
import { usePlaybackHour } from './follow/useFollow.ts';

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

function slotIdentitySince(timeline: Parameters<typeof identitySince>[0], slots: SlotTags | undefined, hour: number, keys: readonly string[]): number | undefined {
  if (!keys.some(isSlotKey)) return identitySince(timeline, hour, keys);
  if (!slots) return undefined;
  const tags: string[] = [];
  let since = -Infinity;
  for (const key of keys) {
    const slot = countryFromKey(key);
    if (typeof slot !== 'number') { tags.push(key); continue; }
    const run = slots.runAt(slot, hour);
    if (!run) continue;
    tags.push(run.tag);
    since = Math.max(since, run.firstHour);
  }
  const held = identitySince(timeline, hour, tags);
  return held === undefined ? undefined : Math.max(held, since);
}

export function choiceHour(at: number, through: number | null, changes: readonly number[]): number {
  if (through === null || through <= at) return at;
  let low = 0, high = changes.length - 1, found = at;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (changes[middle] <= through) { found = Math.max(found, changes[middle]); low = middle + 1; } else high = middle - 1;
  }
  return found;
}

export function useChoiceHour(clock: PlaybackController, on: boolean, at: () => number, through: () => number | null, changes: readonly number[] | undefined,
  wake?: (changed: () => void) => () => void): number {
  const sorted = useMemo(() => [...changes ?? []].sort((a, b) => a - b), [changes]);
  return useAtHead(clock, on, () => choiceHour(at(), through(), sorted), Object.is, wake);
}

export function useIdentityKey(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, on: boolean): (hour: number, tags: readonly string[]) => string | null {
  const content = useContentRevision(client);
  const timeline = useHeldTimeline(client, track, content, on);
  return useCallback((hour: number, tags: readonly string[]) => {
    const since = timeline && identitySince(timeline, hour, tags);
    return since === undefined || since === null ? null : String(since);
  }, [timeline]);
}

export function useIdentityHour(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, tags: readonly string[],
  clock: PlaybackController, on: boolean, trackHour: (clockHour: number) => number | null): number | null {
  const content = useContentRevision(client);
  const timeline = useHeldTimeline(client, track, content, on);
  const slots = useSlotTags(client, track, on && tags.some(isSlotKey));
  const settled = useAtHead(clock, on, hour => {
    const at = trackHour(hour);
    return at === null ? null : (timeline ? slotIdentitySince(timeline, slots, at, tags) : undefined) ?? Number.NaN;
  });
  const unheld = on && Number.isNaN(settled);
  const requestHour = usePlaybackHour(clock, unheld);
  return !on || unheld ? trackHour(requestHour) : settled;
}
