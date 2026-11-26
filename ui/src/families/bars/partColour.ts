import { colour, reservedGrey } from '../../charts/palette.ts';

export function partColours(ordinals: readonly (number | null | undefined)[], surface: string): string[] {
  return ordinals.map(ordinal => ordinal == null ? reservedGrey(surface) : colour(surface, ordinal));
}
