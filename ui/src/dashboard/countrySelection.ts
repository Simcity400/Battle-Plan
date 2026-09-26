import type { BoundTrack, CountrySelection } from './model';

/** The dashboard's selected country for this campaign, or its player country. */
export function defaultCountryTag(reference: Pick<BoundTrack, 'selectedTag' | 'source'> | undefined): string | null {
  return reference?.selectedTag ?? reference?.source.metadata.countries.find(country => country.id === reference.source.metadata.playerCountryId)?.tag ?? null;
}

/** The countries a widget shows: its own picks, or the dashboard's country. A group
 * resolves through the recording's group history (`groups.ts`), so it has no fixed tags. */
export function selectedTags(countries: CountrySelection, reference: Pick<BoundTrack, 'selectedTag' | 'source'> | undefined): string[] {
  if (countries.kind === 'tags') return countries.tags;
  if (countries.kind === 'group' || countries.kind === 'all') return [];
  const tag = defaultCountryTag(reference);
  return tag ? [tag] : [];
}
