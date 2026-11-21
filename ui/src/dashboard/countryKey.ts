import type { CountryMetadata, CountryRef } from '../generated/dashboard';

export const SLOT_MARK = '\u0000';

export const countryKey = (country: CountryRef): string => typeof country === 'number' ? `${SLOT_MARK}${country}` : country;

export const countryFromKey = (key: string): CountryRef => key.startsWith(SLOT_MARK) && /^\d+$/.test(key.slice(1)) ? Number(key.slice(1)) : key;

export const isSlotKey = (key: string): boolean => typeof countryFromKey(key) === 'number';

export const metadataCountry = (countries: readonly CountryMetadata[], key: string): CountryMetadata | undefined =>
  isSlotKey(key) ? countries.find(country => country.id === String(countryFromKey(key))) : countries.find(country => country.tag === key);

export const answersKey = (asked: CountryRef | null | undefined, key: string | null | undefined): boolean =>
  key != null && asked != null && countryKey(asked) === key;
