import { countryLineColours } from '../../dashboard/countryLineColours.ts';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { CountryStyle } from '../../generated/dashboard';

export function sideColours(tags: readonly string[], color: (tag: string) => CountryStyle['color'], surface: string, fixed: boolean): Map<string, string> {
  const styles = new Map<string, CountryStyle>();
  for (const tag of tags) styles.set(tag, { tag, name: null, color: color(tag) });
  const colours = countryLineColours([...styles.keys()].sort(compareTags), styles, surface, fixed);
  return new Map([...colours].filter(([tag]) => styles.get(tag)?.color != null));
}
