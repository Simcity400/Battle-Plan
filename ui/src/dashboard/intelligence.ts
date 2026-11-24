import { compareTags } from './tagOrder.ts';
import type { Country } from '../generated/dashboard';

export const countryKey = (value: Country) => value.tag ?? `#${value.id}`;
export const compareCountries = (a: Country, b: Country) => compareTags(countryKey(a), countryKey(b)) || a.id - b.id;

export function tagsOf(countries: Iterable<Country | null>): string[] {
  const tags = new Set<string>();
  for (const value of countries) if (value?.tag) tags.add(value.tag);
  return [...tags].sort(compareTags);
}

export const fixed5 = (value: string) => Number(BigInt(value)) / 100_000;
