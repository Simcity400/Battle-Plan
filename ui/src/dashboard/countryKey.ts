import type { CountryMetadata, CountryRef } from '../generated/dashboard';

/** What starts a slot's key: a NUL, which no tag holds (tags are refused with one, `validate_tag`). */
export const SLOT_MARK = '\u0000';

/** A country's name where a string names it, as the service names its rows (`Country::key`): the tag, or
 * [`SLOT_MARK`] and the slot's number. Never shown: a slot is named by the tag it goes by at the hour shown. */
export const countryKey = (country: CountryRef): string => typeof country === 'number' ? `${SLOT_MARK}${country}` : country;

/** The country a [`countryKey`] names: a slot, else the tag. */
export const countryFromKey = (key: string): CountryRef => key.startsWith(SLOT_MARK) && /^\d+$/.test(key.slice(1)) ? Number(key.slice(1)) : key;

/** Whether a key names a country by its slot. */
export const isSlotKey = (key: string): boolean => typeof countryFromKey(key) === 'number';

/** The recorded catalogue entry a [`countryKey`] names: a slot's first entry, or a tag's. */
export const metadataCountry = (countries: readonly CountryMetadata[], key: string): CountryMetadata | undefined =>
  isSlotKey(key) ? countries.find(country => country.id === String(countryFromKey(key))) : countries.find(country => country.tag === key);

/** Whether a reply is the one to `key`: the country its request named (its `asked` echo) is the one `key` names, so a
 * reply asked before the selection moved is never taken for the new country. */
export const answersKey = (asked: CountryRef | null | undefined, key: string | null | undefined): boolean =>
  key != null && asked != null && countryKey(asked) === key;
