import { useCallback, useRef, useSyncExternalStore } from 'react';
import { atNextSlot, STRUCTURE_SLOT_MS, structureOpen } from '../../dashboard/structurePace.ts';

/** The parts of a dashboard clock the widget's hour reads. */
export interface HeadClock {
  getHour(): number;
  getSnapshot(): { hoursPerSecond: number; playing: boolean };
  subscribeFrame(listener: () => void): () => void;
  subscribe(listener: () => void): () => void;
}

/** While playing, a key change renders at most once a slot of the page's shared structure pace (`structurePace.ts`):
 * a structural change (a row entering a race) is one commit a quarter second at most, however fast the rows churn. */
export const STRUCTURE_PACE_MS = STRUCTURE_SLOT_MS;

/** A key as a widget renders it: `next` (at `hour`) once it differs from the one `held`, unless playing outside the
 * slot's task (`structureOpen`), when the held one stays until the slot asks again. */
export function paced(held: { key: string; hour: number } | null, next: string, hour: number, playing: boolean): { key: string; hour: number } {
  if (held?.key === next) return held;
  if (held && !structureOpen(playing)) return held;
  return { key: next, hour };
}

/** The clock's hour as the widget renders it while `enabled`: the hour at which `key` of the hour last changed. The
 * widget renders only when the key does (the race's rows, the window its reads come from), at most every
 * `STRUCTURE_PACE_MS` while playing (a held change is taken when the pace allows), never because the hour moved on inside it. `woken` holds the listeners told when
 * the key may have moved without the clock (a pushed answer while paused). Not enabled, the hour is read as rendered
 * and nothing re-renders for it. */
export function useHeadHour(clock: HeadClock, enabled: boolean, key: (hour: number) => string, woken?: Set<() => void>): number {
  const keyed = useRef(key); keyed.current = key;
  const held = useRef<{ key: string; hour: number } | null>(null);
  // A change the pace holds is taken at the next slot, with every other widget's, not at the clock's next frame: a
  // reply landing just after a structural render would otherwise render later, as the hour moves.
  const due = useRef<{ timer: (() => void) | null; listeners: Set<() => void> }>({ timer: null, listeners: new Set() });
  const subscribe = useCallback((changed: () => void) => {
    if (!enabled) return () => {};
    const stops = [clock.subscribeFrame(changed), clock.subscribe(changed)];
    woken?.add(changed);
    const pending = due.current;
    pending.listeners.add(changed);
    return () => {
      for (const stop of stops) stop();
      woken?.delete(changed);
      pending.listeners.delete(changed);
      if (!pending.listeners.size && pending.timer !== null) { pending.timer(); pending.timer = null; }
    };
  }, [clock, enabled, woken]);
  const read = () => {
    if (!enabled) return '';
    const hour = clock.getHour(), next = keyed.current(hour);
    held.current = paced(held.current, next, hour, clock.getSnapshot().playing);
    const pending = due.current;
    if (held.current.key !== next && pending.timer === null && pending.listeners.size) {
      pending.timer = atNextSlot(() => {
        pending.timer = null;
        for (const changed of [...pending.listeners]) changed();
      });
    }
    return held.current.key;
  };
  useSyncExternalStore(subscribe, read, read);
  return enabled && held.current ? held.current.hour : clock.getHour();
}

/** A key of the hour moving at most four times a second of playback, for reads that name no span they hold over. */
export const pacedKey = (clock: Pick<HeadClock, 'getSnapshot'>) => (hour: number) => {
  const { hoursPerSecond, playing } = clock.getSnapshot();
  return String(playing ? Math.floor(hour / Math.max(1, hoursPerSecond / 4)) : hour);
};

/** `key` of a race's rows, made to move at every hour while paused: after a paused seek that keeps the same rows, what
 * the widget renders at its hour (a static label, an empty race's text) is of the hour sought, not the one before. */
export const pausedKey = (key: string, hour: number, playing: boolean) => `${key}|${playing ? '' : Math.floor(hour)}`;
