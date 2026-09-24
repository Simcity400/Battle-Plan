import type { CountryMetadata } from '../shell/api';

/** One pickable country with its search key computed once. */
export interface CountryChoice { country: CountryMetadata; search: string }

/** Fixed option height in CSS pixels; `.country-picker-option` uses the same value. */
export const COUNTRY_ROW_HEIGHT = 36;
/** Rows rendered beyond each edge of the viewport. */
export const COUNTRY_ROW_OVERSCAN = 6;

const collator = new Intl.Collator();

/** The first entry per tag, sorted by display name. */
export function countryChoices(countries: readonly CountryMetadata[]): CountryChoice[] {
  const byTag = new Map<string, CountryMetadata>();
  for (const country of countries) if (!byTag.has(country.tag)) byTag.set(country.tag, country);
  return [...byTag.values()]
    .map(country => ({ country, label: country.name || country.tag, search: `${country.name} ${country.tag}`.toLowerCase() }))
    .sort((a, b) => collator.compare(a.label, b.label))
    .map(({ country, search }) => ({ country, search }));
}

export function filterChoices(choices: CountryChoice[], query: string): CountryChoice[] {
  const needle = query.trim().toLowerCase();
  return needle ? choices.filter(choice => choice.search.includes(needle)) : choices;
}

/** The half-open row range a scrolled list renders; independent of how many rows exist. */
export function visibleRange(count: number, scrollTop: number, viewportHeight: number,
  rowHeight = COUNTRY_ROW_HEIGHT, overscan = COUNTRY_ROW_OVERSCAN): { start: number; end: number } {
  const first = Math.floor(Math.max(0, scrollTop) / rowHeight);
  const start = Math.min(count, Math.max(0, first - overscan));
  const end = Math.min(count, first + Math.ceil(Math.max(0, viewportHeight) / rowHeight) + 1 + overscan);
  return { start, end: Math.max(start, end) };
}

/** Keyboard movement by index: arrows wrap, Home and End jump. Null for other keys. */
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

/** The scroll offset that brings a row fully into view, moving as little as possible. */
export function revealRow(index: number, scrollTop: number, viewportHeight: number, rowHeight = COUNTRY_ROW_HEIGHT): number {
  const top = index * rowHeight;
  if (top < scrollTop) return top;
  if (top + rowHeight > scrollTop + viewportHeight) return top + rowHeight - viewportHeight;
  return scrollTop;
}
