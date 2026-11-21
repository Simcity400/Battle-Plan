import type { BoundTrack, CountrySelection } from './model';
import { countryKey } from './countryKey.ts';

export function defaultCountryKey(reference: Pick<BoundTrack, 'selectedSlot' | 'source'> | undefined): string | null {
  if (reference?.selectedSlot != null) return countryKey(reference.selectedSlot);
  const player = reference?.source.metadata.playerCountryId;
  const slot = player == null ? NaN : Number(player);
  return Number.isSafeInteger(slot) ? countryKey(slot) : null;
}

export function singleCountryChoice(tag: string | null, fallback: string | null, fixed = false): CountrySelection {
  return tag && (fixed || tag !== fallback) ? { kind: 'tags', tags: [tag] } : { kind: 'selected' };
}

export function selectedCountryKeys(countries: CountrySelection, reference: Pick<BoundTrack, 'selectedSlot' | 'source'> | undefined): string[] {
  if (countries.kind === 'tags') return countries.tags;
  if (countries.kind === 'group' || countries.kind === 'all' || countries.kind === 'factions') return [];
  const country = defaultCountryKey(reference);
  return country === null ? [] : [country];
}
