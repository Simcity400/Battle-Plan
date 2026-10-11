import { useMemo, useRef } from 'react';
import type { CountryStyle, CountryStyleReply } from '../generated/dashboard';
import type { DashboardClient } from './client';
import { flagReadMetadata, flagReadSpec } from './countryFlagsAt.ts';
import type { CountryLabel } from './countryFlagsAt';
import { nameKey, useCountryFactsRead } from './countryNamesAt.ts';
import { FAILED_FACT } from './countryFacts.ts';
import type { FactSpec, NameAt } from './countryFacts';
import { lookPlan } from './countryLooks.ts';
import { sampledLooks } from './looks.ts';
import type { Look, LookRead } from './looks';
import type { BoundTrack } from './model';
import { useContentRevision } from './contentRevision.ts';
import { useThrottled } from './useThrottled.ts';

/** How often names and flags follow a moving hour, in milliseconds. */
export const FOLLOW_MS = 400;

/** Styles from `country-styles`, which names at most 64 countries per request. */
export const STYLES: FactSpec<CountryStyle> = {
  product: 'country-styles', perRequest: 64,
  params: (branchId, hour, tags) => ({ branchId, hour, tags }),
  extract: data => new Map((data as CountryStyleReply).styles.map(style => [style.tag, style] as const)),
};
export const FLAGS = flagReadSpec('shield');

/** A country as a lane names it: its name and flag. */
export type CountryAt = CountryLabel;

/** Countries named and flagged as the recording showed them at `hour` (a timeline's
 * playhead, or its end). `tags` names the lanes in view; `nameTags` may add names
 * needed for sorting. While the hour moves it is read at most every
 * `FOLLOW_MS`, not once per hour played. What is shown follows the looks rule of the
 * shared kit (`nextLooks`), each part on its own: while a part's read is still going a
 * country keeps the part shown before, so nothing flashes; once it completes, what it
 * gave replaces it, and a part it did not give is not known, never an older value.
 * Looks are held within one recorded stretch and identity bound (`lookPlan`): an hour in a gap reads
 * nothing, and a moment past a gap starts afresh. A name not known falls back to the
 * loading/unavailable wording; a flag not known shows none. `revision`
 * changes as reads arrive, for a chart to redraw. Flag status describes the current
 * read independently of any URL held while it loads. `nameTags` may include every
 * lane being sorted by name; flag requests still use only `tags`. `segmentAt` gives
 * the current identity bound from recorded evidence, or null when unknown. Even
 * before the throttle advances, an older holder's sample is never applied. Name
 * status follows the read; a held native name remains known while its replacement loads. */
export function useCountriesAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, tags: readonly string[],
  hour: number, active: boolean, nameTags: readonly string[] = tags,
  segmentAt?: (tag: string, hour: number) => number | null, wording?: { pending: string; missing: string }): { country: CountryAt; revision: number } {
  const content = useContentRevision(client);
  const moment = Math.floor(hour);
  const shownHour = useThrottled(moment, FOLLOW_MS);
  const trackKey = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, content]) : '';
  const shown = useRef<{ trackKey: string; client: DashboardClient; looks: Map<string, Look>; segments: Map<string, number> }>({ trackKey, client, looks: new Map(), segments: new Map() });
  if (shown.current.trackKey !== trackKey || shown.current.client !== client) shown.current = { trackKey, client, looks: new Map(), segments: new Map() };
  const held = shown.current;
  const tagsKey = JSON.stringify(tags);
  const nameTagsKey = JSON.stringify(nameTags);
  const tagSets = useMemo(() => ({ flags: new Set(JSON.parse(tagsKey) as string[]), names: new Set(JSON.parse(nameTagsKey) as string[]) }), [tagsKey, nameTagsKey]);
  const allTags = useMemo(() => [...new Set([...tagSets.flags, ...tagSets.names])].sort(), [tagSets]);
  const plan = useMemo(() => lookPlan(track?.branch, allTags, shownHour, segmentAt, undefined, true), [track?.branch, allTags, shownHour, segmentAt]);
  // Held looks are replaced, never changed in place, so the map's identity stands for its countries.
  const heldTags = useMemo(() => JSON.stringify([...new Set([...held.looks.keys(), ...allTags])].sort()), [held.looks, allTags]);
  // Retained leaving rows are guarded too, without asking for their artwork.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const segments = useMemo(() => new Map(lookPlan(track?.branch, JSON.parse(heldTags) as string[], moment, segmentAt, undefined, true).samples.map(sample => [sample.tag, sample.stretch])),
    [heldTags, track?.branch, moment, segmentAt]);
  const samples = useMemo(() => plan.samples.filter(sample => segments.get(sample.tag) === sample.stretch), [plan, segments]);
  const wanted = useMemo<NameAt[]>(() => samples.filter(sample => tagSets.flags.has(sample.tag)).map(({ tag, hour }) => ({ tag, hour })), [samples, tagSets]);
  const nameWanted = useMemo<NameAt[]>(() => samples.filter(sample => tagSets.names.has(sample.tag)).map(({ tag, hour }) => ({ tag, hour })), [samples, tagSets]);
  const styles = useCountryFactsRead(client, track, nameWanted, active, STYLES);
  const flags = useCountryFactsRead(client, track, wanted, active, FLAGS);
  return useMemo(() => {
    // Merging what finished is idempotent, so a repeated render changes nothing.
    const next = sampledLooks(held, samples, sample => {
      const key = nameKey(sample.hour, sample.tag);
      const flag = flags.get(key);
      return { style: styles.get(key), flag: flag === undefined ? undefined : flag === FAILED_FACT ? FAILED_FACT : flag?.url ?? null };
    }, segments);
    held.looks = next.looks;
    held.segments = next.segments;
    const looks = held.looks;
    const flagTags = new Set(wanted.map(item => item.tag));
    const sampleHours = new Map(samples.map(sample => [sample.tag, sample.hour]));
    return {
      country: tag => {
        const flag = flags.get(nameKey(shownHour, tag));
        const metadata = flagReadMetadata(flag);
        const sampleHour = sampleHours.get(tag);
        const style = sampleHour === undefined ? undefined : styles.get(nameKey(sampleHour, tag));
        const heldName = looks.get(tag)?.name;
        const pending = { name: heldName ?? (sampleHour === undefined ? wording?.missing ?? 'Country name unavailable' : wording?.pending ?? 'Loading name…'),
          known: heldName != null ? true : sampleHour === undefined ? false : undefined, nameFailed: false };
        return { ...countryReadName(style, pending, wording?.missing ?? 'Country name unavailable'), ...metadata, flag: looks.get(tag)?.flag ?? undefined,
          flagPending: flagTags.has(tag) && metadata.flagPending };
      },
      revision: styles.size + flags.size * 1000 + looks.size * 1_000_000,
    };
  }, [samples, segments, wanted, styles, flags, held, shownHour, wording]);
}

/** Native empty names stay known; only an absent or failed completed read is missing. */
export function countryReadName(style: LookRead['style'], pending: ReturnType<CountryLabel>, missing: string): Pick<ReturnType<CountryLabel>, 'name' | 'known' | 'nameFailed'> {
  return { name: style === undefined ? pending.name : style === FAILED_FACT ? missing : style?.name ?? missing,
    known: style === undefined ? pending.known : style !== FAILED_FACT && typeof style?.name === 'string',
    nameFailed: style === undefined ? pending.nameFailed : style === FAILED_FACT };
}
