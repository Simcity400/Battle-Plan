import type { StatusMarkKind } from '../../draw/kit.ts';
import type { Value } from '../value.ts';

export interface LogKindFilterItem {
  readonly value: string;
  readonly label: string;
  readonly group?: string;
  readonly checked: boolean;
  readonly country?: string;
  readonly countryName?: string;
  readonly countryFlag?: string;
  readonly detail?: string;
  readonly detailMissing?: string;
  readonly icon?: string;
  readonly state?: { readonly kind: StatusMarkKind; readonly label: string };
  readonly quantity?: { readonly value: Value<number>; readonly unit: string };
}

export interface Filters { readonly query: string; readonly country: string; readonly kind: string }

export type Entry = { readonly kind: 'group'; readonly label: string; readonly top: number; readonly height: number }
  | { readonly kind: 'item'; readonly item: LogKindFilterItem; readonly index: number; readonly last: boolean; readonly top: number; readonly height: number };

const GROUP_HEIGHT = 29;
const GROUP_GAP = 6;
const OVERSCAN = 6;
const collator = new Intl.Collator();

export function layoutEntries(items: readonly LogKindFilterItem[], rowHeight: number): { readonly entries: readonly Entry[]; readonly options: readonly Entry[]; readonly height: number } {
  const groups = new Map<string, LogKindFilterItem[]>();
  for (const item of items) {
    const list = groups.get(item.group ?? '');
    if (list) list.push(item);
    else groups.set(item.group ?? '', [item]);
  }
  const entries: Entry[] = [];
  const options: Entry[] = [];
  let top = 0;
  for (const [group, list] of groups) {
    if (group) {
      const height = GROUP_HEIGHT + (entries.length ? GROUP_GAP : 0);
      entries.push({ kind: 'group', label: group, top, height });
      top += height;
    }
    list.forEach((item, at) => {
      const entry: Entry = { kind: 'item', item, index: options.length, last: at === list.length - 1, top, height: rowHeight };
      entries.push(entry);
      options.push(entry);
      top += rowHeight;
    });
  }
  return { entries, options, height: top };
}

export function visibleEntries(entries: readonly Entry[], scrollTop: number, height: number): readonly Entry[] {
  let lo = 0;
  let hi = entries.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (entries[mid]!.top + entries[mid]!.height <= scrollTop) lo = mid + 1;
    else hi = mid;
  }
  let end = lo;
  while (end < entries.length && entries[end]!.top < scrollTop + height) end++;
  return entries.slice(Math.max(0, lo - OVERSCAN), Math.min(entries.length, end + OVERSCAN));
}

export function nextOption(key: string, active: number, count: number, page: number): number | null {
  if (!count) return null;
  switch (key) {
    case 'ArrowDown': return Math.min(count - 1, active + 1);
    case 'ArrowUp': return active < 0 ? count - 1 : Math.max(0, active - 1);
    case 'PageDown': return Math.min(count - 1, Math.max(0, active) + page);
    case 'PageUp': return Math.max(0, active - page);
    case 'Home': return 0;
    case 'End': return count - 1;
    default: return null;
  }
}

export interface FacetChoices {
  readonly countries: readonly (readonly [string, { readonly name: string; readonly flag?: string }])[];
  readonly kinds: readonly string[];
}

export function facetChoices(items: readonly LogKindFilterItem[]): FacetChoices {
  const byTag = new Map<string, { name: string; flag?: string }>();
  for (const item of items) if (item.country && !byTag.has(item.country)) byTag.set(item.country, { name: item.countryName ?? item.country, ...(item.countryFlag ? { flag: item.countryFlag } : {}) });
  return {
    countries: [...byTag].sort((a, b) => collator.compare(a[1].name, b[1].name)),
    kinds: [...new Set(items.flatMap(item => item.detail ? [item.detail] : []))].sort(collator.compare),
  };
}

export function appliedFacets(filters: Filters, choices: FacetChoices): { readonly country: string; readonly kind: string } {
  return {
    country: choices.countries.some(([tag]) => tag === filters.country) ? filters.country : '',
    kind: choices.kinds.includes(filters.kind) ? filters.kind : '',
  };
}
