import { useRef } from 'react';

/** The countries a followed top's per-country follows and looks name, and what they are kept for. */
export interface StickyMembers {
  readonly reset: string;
  /** Sorted, each once. */
  readonly tags: readonly string[];
}

/** How many countries a top of `top` keeps named before it starts again from the top now: each one named costs a row
 * in every per-country answer, so the set is bounded by the top it serves, never by the hours played. */
export const stickyLimit = (top: number) => Math.max(64, top * 4);

/** The members `held` names for the same `reset` (the query the top is, its measure and size), with every country of
 * `now` it lacks added: a country leaving the top and coming back, or the top reordering, names nothing new, so no
 * follow is asked again and nothing renders. Over `limit` it starts again from `now`. The same object while nothing
 * is added. */
export function stickyMembers(held: StickyMembers | null, reset: string, now: readonly string[], limit: number): StickyMembers {
  const fresh = new Set(now);
  if (held && held.reset === reset) {
    for (const tag of held.tags) fresh.delete(tag);
    if (!fresh.size) return held;
    if (held.tags.length + fresh.size <= limit) return { reset, tags: [...held.tags, ...fresh].sort() };
    return { reset, tags: [...new Set(now)].sort() };
  }
  return { reset, tags: [...fresh].sort() };
}

/** `stickyMembers` held across renders: the members a top named since `reset` changed. */
export function useStickyMembers(reset: string, now: readonly string[], limit: number): readonly string[] {
  const held = useRef<StickyMembers | null>(null);
  held.current = stickyMembers(held.current, reset, now, limit);
  return held.current.tags;
}
