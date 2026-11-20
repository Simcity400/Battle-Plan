import type { BoundTrack, CountrySelection } from './model';
import { countryKey } from './countryKey.ts';

/** The dashboard's selected country for this campaign, as its key ([`countryKey`]): its canonical slot, which follows
 * that country through tag changes; else its player country's. */
export function defaultCountryKey(reference: Pick<BoundTrack, 'selectedSlot' | 'source'> | undefined): string | null {
  if (reference?.selectedSlot != null) return countryKey(reference.selectedSlot);
  const player = reference?.source.metadata.playerCountryId;
  const slot = player == null ? NaN : Number(player);
  return Number.isSafeInteger(slot) ? countryKey(slot) : null;
}

/** A fixed choice keeps the picked tag even when it is the dashboard's country today. */
export function singleCountryChoice(tag: string | null, fallback: string | null, fixed = false): CountrySelection {
  return tag && (fixed || tag !== fallback) ? { kind: 'tags', tags: [tag] } : { kind: 'selected' };
}

/** The countries a widget shows, as keys: its own picked tags, or the dashboard's country. A group
 * resolves through the recording's group history (`groups.ts`), so it has no fixed countries. */
export function selectedCountryKeys(countries: CountrySelection, reference: Pick<BoundTrack, 'selectedSlot' | 'source'> | undefined): string[] {
  if (countries.kind === 'tags') return countries.tags;
  if (countries.kind === 'group' || countries.kind === 'all' || countries.kind === 'factions') return [];
  const country = defaultCountryKey(reference);
  return country === null ? [] : [country];
}
