import { CATEGORY_LIMIT, colour, DEFAULT_CARD_SURFACE } from '../../charts/palette.ts';

function keySlot(key: string): number {
  let hash = 2166136261;
  for (let i = 0; i < key.length; i++) {
    hash ^= key.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0) % CATEGORY_LIMIT;
}

export function partColours(keys: readonly string[]): ReadonlyMap<string, string> {
  const room = Math.max(CATEGORY_LIMIT, keys.length);
  const taken = new Set<number>();
  const out = new Map<string, string>();
  for (const key of [...new Set(keys)].sort()) {
    let slot = keySlot(key);
    while (taken.has(slot)) slot = (slot + 1) % room;
    taken.add(slot);
    out.set(key, colour(DEFAULT_CARD_SURFACE, slot));
  }
  return out;
}
