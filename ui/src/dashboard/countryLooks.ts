import { useMemo, useRef } from 'react';
import type { CountryStyle, CountryStyleReply } from '../generated/dashboard';
import type { DashboardClient } from './client';
import { flagReadMetadata } from './countryFlagsAt.ts';
import { useCountryFlagsRead } from './countryFlags.ts';
import { nameKey, useCountryFactsRead } from './countryNamesAt.ts';
import { FAILED_FACT, MAX_NAME_HOURS } from './countryFacts.ts';
import type { FactSpec } from './countryFacts';
import { matchesLookSegment, sampledLooks } from './looks.ts';
import { useContentRevision } from './contentRevision.ts';
import type { Look } from './looks';
import type { BoundTrack } from './model';
import { heldFacts } from './countryTimeline.ts';

/** Country looks are sampled about once a month of playback. */
export const LOOK_STEP = 30 * 24;

/** A branch's recorded hours: from its first to its last, less its gaps. */
export interface RecordedBranch { firstHour: number; lastHour: number; gaps: readonly { firstHour: number; lastHour: number }[] }

/** Where looks are read for a moment shown: a recorded hour of the same unbroken
 * stretch of the recording as the moment (`stretch`, the stretch's first hour), the
 * branch's first hour plus whole steps where that is in the stretch, else the
 * stretch's first hour; never an hour of a gap, so a read never waits on hours the
 * recording lacks. Null when the moment itself is not recorded (in a gap, or
 * outside the branch). `exact` uses the moment itself instead of the sample grid. */
export function lookHour(branch: RecordedBranch, hour: number, countryStart?: number | null, exact = false): { hour: number; stretch: number } | null {
  const moment = Math.floor(hour);
  if (countryStart === null || (countryStart !== undefined && countryStart > moment)) return null;
  if (moment < branch.firstHour || moment > branch.lastHour) return null;
  if (branch.gaps.some(gap => gap.firstHour <= moment && moment <= gap.lastHour)) return null;
  // The stretch starts after the last gap before the moment.
  const stretch = branch.gaps.reduce((start, gap) => gap.lastHour < moment ? Math.max(start, gap.lastHour + 1) : start, Math.max(branch.firstHour, countryStart ?? branch.firstHour));
  const step = branch.firstHour + Math.floor((moment - branch.firstHour) / LOOK_STEP) * LOOK_STEP;
  return { hour: exact ? moment : Math.max(step, stretch), stretch };
}

/** One identity-bounded sample for every part of a country's look. Optional
 * per-country hours date labels to their own moments; tags remain opaque. */
export function lookPlan(branch: RecordedBranch | undefined, tags: readonly string[], hour: number, segmentAt?: (tag: string, hour: number) => number | null,
  hours?: ReadonlyMap<string, number>, exact = false) {
  const samples = branch ? [...new Set(tags)].sort().flatMap(tag => {
    const at = hours ? hours.get(tag) : hour;
    if (at === undefined) return [];
    const moment = Math.floor(at);
    const sample = lookHour(branch, moment, segmentAt?.(tag, moment), exact);
    return sample ? [{ tag, ...sample }] : [];
  }) : [];
  const wanted = samples.map(({ tag, hour }) => ({ tag, hour }));
  // Per-country label hours can cover more hours than the tooltip default. Keep
  // every currently wanted sample protected, so long histories cannot cycle reads.
  return { samples, wanted, maxHours: Math.max(MAX_NAME_HOURS, new Set(wanted.map(item => item.hour)).size) };
}

/** Styles from `country-styles`, which names at most 64 countries per request. */
const STYLES: FactSpec<CountryStyle & { warnings?: readonly string[] }> = {
  product: 'country-styles', perRequest: 64,
  params: (branchId, hour, tags) => ({ branchId, hour, tags }),
  extract: data => {
    const reply = data as CountryStyleReply;
    return new Map(reply.styles.map(style => [style.tag, { ...style, warnings: reply.warnings }] as const));
  },
  held: heldFacts(style => style),
};
/** What a country's name reads while its read is still answering, as other name reads say it. */
export const NAME_LOADING = 'Loading name…';

export interface CountryLooks {
  /** Native diagnostics from current completed style reads. */
  warnings?: readonly string[];
  /** The localized name at the sample hour, else an unavailable-name label. */
  name: (tag: string, hour?: number) => string;
  /** The recorded map colour as sRGB bytes, or null when not known. */
  color: (tag: string, hour?: number) => [number, number, number] | null;
  /** The flag as an image URL, or undefined when not known. */
  flag: (tag: string, hour?: number) => string | undefined;
  /** Status of the current sampled read, apart from any held same-segment URL. */
  flagPending: (tag: string, hour?: number) => boolean;
  flagReason: (tag: string, hour?: number) => string | undefined;
  /** Whether this look's bound matches an hour being drawn before React replans. */
  matches?: (tag: string, hour: number) => boolean;
  /** Changes as looks arrive, for a chart to redraw. */
  revision: number;
}

/** Names, colours and flags at one sample hour within each country's segment.
 * `segmentAt` gives the first hour of its unbroken identity/data segment, independent
 * of display membership; null means unknown. Without it, use the recorded stretch.
 * Pending parts keep their previous look within the segment; completed missing
 * parts show the fallback. Optional `hours` gives each country its label hour;
 * `exact` reads that hour directly instead of the playback sample grid. `flagTags`
 * limits flag artwork to mounted rows; styles keep the full country set. */
export function useCountryLooks(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, tags: readonly string[], hour: number,
  active: boolean, segmentAt?: (tag: string, hour: number) => number | null, hours?: ReadonlyMap<string, number>, exact = false, flagTags?: readonly string[]): CountryLooks {
  const content = useContentRevision(client);
  const moment = Math.floor(hour);
  // The set of countries, not their order: a race re-ranking asks for nothing new.
  const tagKey = useMemo(() => JSON.stringify([...new Set(tags)].sort()), [tags]);
  const set = useMemo(() => JSON.parse(tagKey) as string[], [tagKey]);
  const plan = useMemo(() => lookPlan(track?.branch, set, moment, segmentAt, hours, exact), [set, track?.branch, moment, segmentAt, hours, exact]);
  const { samples, wanted } = plan;
  const styles = useCountryFactsRead(client, track, wanted, active, STYLES, plan.maxHours);
  const flagKey = useMemo(() => flagTags === undefined ? null : JSON.stringify([...new Set(flagTags)].sort()), [flagTags]);
  const flagWanted = useMemo(() => {
    if (flagKey === null) return wanted;
    const visible = new Set(JSON.parse(flagKey) as string[]);
    return wanted.filter(item => visible.has(item.tag));
  }, [wanted, flagKey]);
  const flagKeys = useMemo(() => new Map(flagWanted.map(item => [item.tag, nameKey(item.hour, item.tag)])), [flagWanted]);
  const flags = useCountryFlagsRead(client, track, flagWanted, active);
  const heldKey = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, content]) : '';
  const shown = useRef<{ key: string; client: DashboardClient; looks: Map<string, Look>; segments: Map<string, number> }>({ key: heldKey, client, looks: new Map(), segments: new Map() });
  if (shown.current.key !== heldKey || shown.current.client !== client) shown.current = { key: heldKey, client, looks: new Map(), segments: new Map() };
  const held = shown.current;
  // Rows fading out of a race still use their look. Retain them only while their
  // identity is known to be unchanged, without requesting countries no longer shown.
  // Memoised, so a render with nothing new keeps the looks object and the chart's layers.
  // Held looks are replaced, never changed in place, so the map's identity stands for its countries.
  const heldTags = useMemo(() => JSON.stringify([...new Set([...held.looks.keys(), ...set])].sort()), [held.looks, set]);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const segments = useMemo(() => new Map(lookPlan(track?.branch, JSON.parse(heldTags) as string[], moment, segmentAt, hours, exact).samples.map(item => [item.tag, item.stretch])),
    [heldTags, track?.branch, moment, segmentAt, hours, exact]);
  return useMemo(() => {
    // Merging what finished is idempotent, so a repeated render changes nothing.
    const next = sampledLooks(held, samples, sample => {
      const key = nameKey(sample.hour, sample.tag);
      const flag = flags.get(key);
      return { style: styles.get(key), flag: flag === undefined ? undefined : flag && flag !== FAILED_FACT ? flag.url ?? null : null };
    }, segments);
    held.looks = next.looks;
    held.segments = next.segments;
    const looks = held.looks;
    const matches = (tag: string, at: number) => matchesLookSegment(segments, tag,
      track ? lookHour(track.branch, at, segmentAt?.(tag, Math.floor(at)))?.stretch ?? null : null);
    const look = (tag: string, at?: number) => at === undefined || matches(tag, at) ? looks.get(tag) : undefined;
    const flagKeyAt = (tag: string, at?: number) => {
      if (at === undefined) return flagKeys.get(tag);
      const sample = track ? lookHour(track.branch, at, segmentAt?.(tag, Math.floor(at)), exact) : null;
      const key = sample ? nameKey(sample.hour, tag) : undefined;
      return key === flagKeys.get(tag) ? key : undefined;
    };
    const warningReplies = new Set([...styles.values()].flatMap(style => style && style !== FAILED_FACT && style.warnings ? [style.warnings] : []));
    const sampleKeys = new Map(samples.map(sample => [sample.tag, nameKey(sample.hour, sample.tag)]));
    // A name whose read has not answered yet is loading, not unavailable.
    const reading = (tag: string, at?: number) => (at === undefined || matches(tag, at)) && sampleKeys.has(tag) && !styles.has(sampleKeys.get(tag)!);
    return {
      warnings: [...new Set([...warningReplies].flat())],
      name: (tag, at) => look(tag, at)?.name ?? (reading(tag, at) ? NAME_LOADING : 'Country name unavailable'),
      color: (tag, at) => look(tag, at)?.color ?? null,
      flag: (tag, at) => look(tag, at)?.flag ?? undefined,
      flagPending: (tag, at) => { const key = flagKeyAt(tag, at); return key !== undefined && flagReadMetadata(flags.get(key)).flagPending; },
      flagReason: (tag, at) => { const key = flagKeyAt(tag, at); return flagReadMetadata(key === undefined ? undefined : flags.get(key)).flagReason; },
      matches,
      revision: styles.size + flags.size * 1000 + looks.size * 1_000_000,
    };
  }, [samples, styles, flags, flagKeys, held, segments, track, segmentAt]);
}
