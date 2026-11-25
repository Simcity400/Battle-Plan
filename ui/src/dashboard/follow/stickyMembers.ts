export interface StickyMembers {
  readonly reset: string;
  readonly tags: readonly string[];
}

export const stickyLimit = (top: number) => Math.max(64, top * 4);

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
