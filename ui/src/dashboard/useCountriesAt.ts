import { failureText } from './queryError.ts';
import { useMemo, useRef } from 'react';
import type { CountryStyle, CountryStyleReply } from '../generated/dashboard';
import type { DashboardClient } from './client';
import { flagReadMetadata } from './countryFlagsAt.ts';
import { useCountryFlagsRead } from './countryFlags.ts';
import type { CountryLabel } from './countryFlagsAt';
import { nameKey, useCountryFactsByKey } from './countryNamesAt.ts';
import { isFailedFact } from './countryFacts.ts';
import type { FactSpec, NameAt } from './countryFacts';
import { lookPlan, looksRead } from './countryLooks.ts';
import { useIdentityHour } from './atHead.ts';
import { PlaybackController } from './playback.ts';
import { sampledLooks } from './looks.ts';
import type { Look, LookRead } from './looks';
import type { BoundTrack } from './model';
import { useContentRevision } from './contentRevision.ts';
import { useThrottled } from './useThrottled.ts';
import { heldFacts } from './countryTimeline.ts';

export const FOLLOW_MS = 400;

export const STYLES: FactSpec<CountryStyle> = {
  product: 'country-styles', perRequest: Infinity, shared: true,
  params: (branchId, hour, tags) => ({ branchId, hour, tags }),
  extract: data => new Map((data as CountryStyleReply).styles.map(style => [style.tag, style] as const)),
  held: heldFacts(({ tag, name, color }) => ({ tag, name, color })),
};

export type CountryAt = CountryLabel;

export interface CountriesHead { clock: PlaybackController; hour: (clockHour: number) => number | null; on: boolean }
const IDLE = new PlaybackController();
const noHour = () => null;

export function useCountriesAt(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, tags: readonly string[],
  hour: number, active: boolean, nameTags: readonly string[] = tags,
  segmentAt?: (tag: string, hour: number) => number | null, wording?: { pending: string; missing: string }, head?: CountriesHead): { country: CountryAt; revision: number } {
  const content = useContentRevision(client);
  const moment = Math.floor(hour);
  const throttled = useThrottled(moment, FOLLOW_MS);
  const identityHour = useIdentityHour(client, track, nameTags, head?.clock ?? IDLE, active && !!head?.on, head?.hour ?? noHour);
  const read = looksRead(active && !!head?.on, identityHour, throttled, segmentAt);
  const shownHour = read.hour;
  const trackKey = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, content]) : '';
  const shown = useRef<{ trackKey: string; client: DashboardClient; looks: Map<string, Look>; segments: Map<string, number>; revision: number }>({ trackKey, client, looks: new Map(), segments: new Map(), revision: 0 });
  if (shown.current.trackKey !== trackKey || shown.current.client !== client) shown.current = { trackKey, client, looks: new Map(), segments: new Map(), revision: 0 };
  const held = shown.current;
  const tagsKey = JSON.stringify(tags);
  const nameTagsKey = JSON.stringify(nameTags);
  const tagSets = useMemo(() => ({ flags: new Set(JSON.parse(tagsKey) as string[]), names: new Set(JSON.parse(nameTagsKey) as string[]) }), [tagsKey, nameTagsKey]);
  const allTags = useMemo(() => [...new Set([...tagSets.flags, ...tagSets.names])].sort(), [tagSets]);
  const plan = useMemo(() => lookPlan(track?.branch, allTags, shownHour, read.segmentAt, undefined, true), [track?.branch, allTags, shownHour, read.segmentAt]);
  const heldTags = useMemo(() => JSON.stringify([...new Set([...held.looks.keys(), ...allTags])].sort()), [held.looks, allTags]);
  const segments = useMemo(() => new Map(lookPlan(track?.branch, JSON.parse(heldTags) as string[], read.segmentAt ? moment : shownHour, read.segmentAt, undefined, true).samples.map(sample => [sample.tag, sample.stretch])),
    [heldTags, track?.branch, moment, shownHour, read.segmentAt]);
  const samples = useMemo(() => plan.samples.filter(sample => segments.get(sample.tag) === sample.stretch), [plan, segments]);
  const wanted = useMemo<NameAt[]>(() => samples.filter(sample => tagSets.flags.has(sample.tag)).map(({ tag, hour }) => ({ tag, hour })), [samples, tagSets]);
  const nameWanted = useMemo<NameAt[]>(() => samples.filter(sample => tagSets.names.has(sample.tag)).map(({ tag, hour }) => ({ tag, hour })), [samples, tagSets]);
  const styles = useCountryFactsByKey(client, track, nameWanted, active, STYLES);
  const flags = useCountryFlagsRead(client, track, wanted, active);
  return useMemo(() => {
    const next = sampledLooks(held, samples, sample => {
      const key = nameKey(sample.hour, sample.tag);
      const flag = flags.get(key);
      return { style: styles.get(key), flag: flag === undefined ? undefined : isFailedFact(flag) ? flag : flag?.url ?? null };
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
        const heldLook = looks.get(tag), heldName = heldLook?.name;
        const reading = active && !!track && sampleHour !== undefined;
        const pending = { name: heldName ?? (reading ? wording?.pending ?? 'Loading name…' : wording?.missing ?? 'Country name unavailable'),
          known: heldName != null ? true : reading ? undefined : false, nameFailed: false };
        return { ...countryReadName(style === undefined ? heldLook?.nameFailure : style, pending, wording?.missing ?? 'Country name unavailable'), ...metadata, flag: looks.get(tag)?.flag ?? undefined,
          flagPending: active && !!track && flagTags.has(tag) && metadata.flagPending };
      },
      revision: ++held.revision,
    };
  }, [samples, segments, wanted, styles, flags, held, shownHour, wording, active, track]);
}

export function countryReadName(style: LookRead['style'], pending: ReturnType<CountryLabel>, missing: string): Pick<ReturnType<CountryLabel>, 'name' | 'known' | 'nameFailed' | 'nameFailure' | 'nameDetail'> {
  return { name: style === undefined ? pending.name : isFailedFact(style) ? failureText(style, 'Couldn’t load country name.') : style?.name ?? missing,
    known: style === undefined ? pending.known : !isFailedFact(style) && typeof style?.name === 'string',
    nameFailed: style === undefined ? pending.nameFailed : isFailedFact(style),
    ...(isFailedFact(style) ? { nameFailure: style, nameDetail: style.message } : {}) };
}
