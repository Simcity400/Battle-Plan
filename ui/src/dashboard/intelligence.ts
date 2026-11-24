import { compareTags } from './tagOrder.ts';
import type { Country } from '../generated/dashboard';

export const countryKey = (value: Country) => value.tag ?? `#${value.id}`;
export const compareCountries = (a: Country, b: Country) => compareTags(countryKey(a), countryKey(b)) || a.id - b.id;

