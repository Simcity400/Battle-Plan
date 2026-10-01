import { useMemo, useRef } from 'react';
import type { CountryStyle, CountryStyleReply } from '../generated/dashboard';
import type { DashboardClient } from './client';
import { flagSpec } from './countryFlagsAt.ts';
import { nameKey, useCountryFactsRead } from './countryNamesAt.ts';
import type { FactSpec, NameAt } from './countryFacts';
import { lookHour } from './countryLooks.ts';
import { nextLooks } from './looks.ts';
import type { Look } from './looks';
import type { BoundTrack } from './model';
import { useThrottled } from './useThrottled.ts';

/** How often names and flags follow a moving hour, in milliseconds. */
const FOLLOW_MS = 400;

/** Styles from `country-styles`, which names at most 64 countries per request. */
export const STYLES: FactSpec<CountryStyle> = {
  product: 'country-styles', perRequest: 64,
  params: (branchId, hour, tags) => ({ branchId, hour, tags }),
  extract: data => new Map((data as CountryStyleReply).styles.map(style => [style.tag, style] as const)),
};
export const FLAGS = flagSpec('medium');

/** A country as a lane names it: its name and flag. */
export type CountryAt = (tag: string) => { name: string; flag?: string };

/** Countries named and flagged as the recording showed them at `hour` (a timeline's
 * playhead, or its end). `tags` names the lanes in view; `nameTags` may add names
 * needed for sorting. While the hour moves it is read at most every
 * `FOLLOW_MS`, not once per hour played. What is shown follows the looks rule of the
 * shared kit (`nextLooks`), each part on its own: while a part's read is still going a
 * country keeps the part shown before, so nothing flashes; once it completes, what it
 * gave replaces it, and a part it did not give is not known, never an older value.
 * Looks are held within one recorded stretch only (`lookHour`): an hour in a gap reads
 * nothing, and a moment past a gap starts afresh. A name not known falls back to the
 * recording's metadata name, else the tag; a flag not known shows none. `revision`
 * changes as reads arrive, for a chart to redraw. `nameTags` may include every
 * lane being sorted by name; flag requests still use only `tags`. */
export function useCountriesAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, tags: readonly string[],
  hour: number, active: boolean, nameTags: readonly string[] = tags): { country: CountryAt; revision: number } {
  const shownHour = useThrottled(Math.floor(hour), FOLLOW_MS);
  // The recorded stretch of the hour; none in a gap, where nothing is read.
  const stretch = track ? lookHour(track.branch, shownHour)?.stretch ?? null : null;
  const tagsKey = JSON.stringify(tags);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const wanted = useMemo<NameAt[]>(() => stretch == null ? [] : tags.map(tag => ({ hour: shownHour, tag })), [tagsKey, shownHour, stretch]);
  const nameTagsKey = JSON.stringify(nameTags);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const nameWanted = useMemo<NameAt[]>(() => stretch == null ? [] : nameTags.map(tag => ({ hour: shownHour, tag })), [nameTagsKey, shownHour, stretch]);
  const styles = useCountryFactsRead(client, track, nameWanted, active, STYLES);
  const flags = useCountryFactsRead(client, track, wanted, active, FLAGS);
  // Held per track and recorded stretch, so no look crosses a gap.
  const trackKey = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, stretch]) : '';
  const shown = useRef<{ trackKey: string; looks: Map<string, Look> }>({ trackKey, looks: new Map() });
  if (shown.current.trackKey !== trackKey) shown.current = { trackKey, looks: new Map() };
  const held = shown.current;
  const metadata = useMemo(() => new Map((track?.source.metadata.countries ?? []).map(country => [country.tag, country.name])), [track?.source]);
  return useMemo(() => {
    // Merging what finished is idempotent, so a repeated render changes nothing.
    held.looks = nextLooks(held.looks, [...new Set([...nameWanted, ...wanted].map(item => item.tag))], tag => ({ style: styles.get(nameKey(shownHour, tag)), flag: flags.get(nameKey(shownHour, tag)) }));
    const looks = held.looks;
    return {
      country: tag => ({ name: looks.get(tag)?.name || metadata.get(tag) || tag, flag: looks.get(tag)?.flag ?? undefined }),
      revision: styles.size + flags.size * 1000 + looks.size * 1_000_000,
    };
  }, [wanted, nameWanted, styles, flags, held, metadata, shownHour]);
}
