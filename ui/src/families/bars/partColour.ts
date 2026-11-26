import { colour, DEFAULT_CARD_SURFACE } from '../../charts/palette.ts';

const slots = new Map<string, Map<string, number>>();

export function partColour(parts: string, key: string): string {
  let held = slots.get(parts);
  if (!held) {
    held = new Map();
    slots.set(parts, held);
  }
  let slot = held.get(key);
  if (slot === undefined) {
    slot = held.size;
    held.set(key, slot);
  }
  return colour(DEFAULT_CARD_SURFACE, slot);
}
