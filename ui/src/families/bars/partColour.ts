import { colour, DEFAULT_CARD_SURFACE } from '../../charts/palette.ts';

const SLOTS = 2048;

export function partColour(key: string): string {
  let hash = 2166136261;
  for (let i = 0; i < key.length; i++) {
    hash ^= key.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return colour(DEFAULT_CARD_SURFACE, (hash >>> 0) % SLOTS);
}
