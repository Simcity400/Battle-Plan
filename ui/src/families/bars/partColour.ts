import { colour, DEFAULT_CARD_SURFACE, reservedGrey } from '../../charts/palette.ts';

export const PART_SURFACE = DEFAULT_CARD_SURFACE;

export function partColours(ordinals: readonly (number | null | undefined)[]): string[] {
  return ordinals.map(ordinal => ordinal == null ? reservedGrey(PART_SURFACE) : colour(PART_SURFACE, ordinal));
}
