import { categoricalPalette, nudgeApart } from '../charts/palette.ts';
import { parseColor, toHex } from '../shell/appearance/color.ts';
import type { CountryStyle } from '../generated/dashboard';

/** Recorded map colours, with the chart's optional appearance adjustment. Missing
 * colours take the palette, exactly as the country balance charts do. */
export function countryLineColours(tags: readonly string[], styles: ReadonlyMap<string, CountryStyle>, surface: string, fixed: boolean): Map<string, string> {
  const palette = categoricalPalette(surface);
  const raw = tags.map((tag, index) => {
    const color = styles.get(tag)?.color;
    return color ? { r: color[0], g: color[1], b: color[2] } : parseColor(palette[index % palette.length])!;
  });
  const colours = fixed ? nudgeApart(raw, parseColor(surface) ?? { r: 0, g: 0, b: 0 }) : raw;
  return new Map(tags.map((tag, index) => [tag, toHex(colours[index])]));
}
