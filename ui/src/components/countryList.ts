import { nameReadMetadata, type NameRead } from '../dashboard/nameReads.ts';
import type { CountryMetadata } from '../shell/api';

export type CountryOption = Pick<CountryMetadata, 'tag' | 'name'> & Partial<Omit<CountryMetadata, 'tag' | 'name'>>;
export interface CountryChoice { country: CountryOption; search: string }

export const COUNTRY_ROW_HEIGHT = 36;
export const COUNTRY_ROW_OVERSCAN = 6;

const collator = new Intl.Collator();

export function countryChoices(countries: readonly CountryOption[]): CountryChoice[] {
  const byTag = new Map<string, CountryOption>();
  for (const country of countries) if (!byTag.has(country.tag)) byTag.set(country.tag, country);
  return [...byTag.values()]
    .map(country => ({ country, label: country.name.trim() || country.tag, search: `${country.name} ${country.tag}`.toLowerCase() }))
    .sort((a, b) => collator.compare(a.label, b.label))
    .map(({ country, search }) => ({ country, search }));
}

export function allowedCountryChoices(countries: readonly CountryMetadata[], allowedTags: readonly string[],
  at: { branchId: string; hour: number } | null, names?: ReadonlyMap<string, NameRead>): CountryChoice[] {
  const byTag = new Map<string, CountryMetadata>();
  for (const country of countries) if (!byTag.has(country.tag)) byTag.set(country.tag, country);
  const choices = countryChoices(allowedTags.map(tag => byTag.get(tag) ?? { tag, name: 'Country name unavailable',
    ...(at ? { observedBranchId: at.branchId, observedHour: at.hour } : {}) }));
  return choices.map(({ country }) => {
    const name = at ? nameReadMetadata(names?.get(country.tag)).name : country.name;
    return { country: { ...country, name }, search: `${name} ${country.tag}`.toLowerCase() };
  });
}

export function filterChoices(choices: CountryChoice[], query: string, shown?: (choice: CountryChoice) => string): CountryChoice[] {
  const needle = query.trim().toLowerCase();
  return needle ? choices.filter(choice => choice.search.includes(needle) || !!shown?.(choice).toLowerCase().includes(needle)) : choices;
}

export function visibleRange(count: number, scrollTop: number, viewportHeight: number,
  rowHeight = COUNTRY_ROW_HEIGHT, overscan = COUNTRY_ROW_OVERSCAN): { start: number; end: number } {
  const first = Math.floor(Math.max(0, scrollTop) / rowHeight);
  const start = Math.min(count, Math.max(0, first - overscan));
  const end = Math.min(count, first + Math.ceil(Math.max(0, viewportHeight) / rowHeight) + 1 + overscan);
  return { start, end: Math.max(start, end) };
}

export function moveIndex(key: string, index: number, count: number): number | null {
  if (!count) return null;
  switch (key) {
    case 'ArrowDown': return index < 0 ? 0 : (index + 1) % count;
    case 'ArrowUp': return index < 0 ? count - 1 : (index - 1 + count) % count;
    case 'Home': return 0;
    case 'End': return count - 1;
    default: return null;
  }
}

export function revealRow(index: number, scrollTop: number, viewportHeight: number, rowHeight = COUNTRY_ROW_HEIGHT): number {
  const top = index * rowHeight;
  if (top < scrollTop) return top;
  if (top + rowHeight > scrollTop + viewportHeight) return top + rowHeight - viewportHeight;
  return scrollTop;
}
