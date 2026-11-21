import { compareTags } from './tagOrder.ts';
import type { Country, Operative, OperativePlacementReply } from '../generated/dashboard';

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const integer = (value: unknown): value is number => Number.isSafeInteger(value);
const text = (value: unknown): value is string | null => value === null || typeof value === 'string';
const country = (value: unknown): value is Country => object(value) && integer(value.id) && (value.tag === null || (typeof value.tag === 'string' && value.tag.length > 0));
const warnings = (value: Record<string, unknown>) => Array.isArray(value.warnings) && value.warnings.every(item => typeof item === 'string');
const operative = (value: unknown): value is Operative => object(value) && typeof value.id === 'string' && /^\d{1,20}$/.test(value.id) && text(value.name) && text(value.nameMissing)
  && (value.name === null) !== (value.nameMissing === null);
const malformed = (what: string) => new Error(`The ${what} reply is malformed.`);

function rows<T>(value: unknown, field: string, row: (item: unknown) => boolean, what: string): T {
  if (!object(value) || !Array.isArray(value[field]) || !(value[field] as unknown[]).every(row) || !warnings(value)
    || ('missing' in value && !text(value.missing))) throw malformed(what);
  return value as T;
}

export const validatePlacements = (value: unknown) => rows<OperativePlacementReply>(value, 'placements', item => object(item) && country(item.owner) && country(item.target)
  && Array.isArray(item.operatives) && item.operatives.length > 0 && item.operatives.every(operative), 'operative placements');

export const countryKey = (value: Country) => value.tag ?? `#${value.id}`;
export const compareCountries = (a: Country, b: Country) => compareTags(countryKey(a), countryKey(b)) || a.id - b.id;

export function tagsOf(countries: Iterable<Country | null>): string[] {
  const tags = new Set<string>();
  for (const value of countries) if (value?.tag) tags.add(value.tag);
  return [...tags].sort(compareTags);
}

export const fixed5 = (value: string) => Number(BigInt(value)) / 100_000;
