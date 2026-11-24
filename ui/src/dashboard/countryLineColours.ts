import { colour, nudgeApart } from '../charts/palette.ts';
import { parseColor, toHex } from '../shell/appearance/color.ts';
import type { CountryStyle } from '../generated/dashboard';

export function countryLineColours(tags: readonly string[], styles: ReadonlyMap<string, Pick<CountryStyle, 'color'>>, surface: string, fixed: boolean): Map<string, string> {
  const raw = tags.map((tag, index) => {
    const color = styles.get(tag)?.color;
    return color ? { r: color[0], g: color[1], b: color[2] } : parseColor(colour(surface, index))!;
  });
  const colours = fixed ? nudgeApart(raw, parseColor(surface) ?? { r: 0, g: 0, b: 0 }) : raw;
  return new Map(tags.map((tag, index) => [tag, toHex(colours[index]!)]));
}

