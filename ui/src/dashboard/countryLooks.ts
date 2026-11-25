import { failureText, type Failure } from './queryError.ts';
import { useMemo, useRef } from 'react';
import type { CountryStyle, CountryStyleReply } from '../generated/dashboard';
import type { DashboardClient } from './client';
import { flagReadMetadata } from './countryFlagsAt.ts';
import { useCountryFlagsRead } from './countryFlags.ts';
import { nameKey, useCountryFactsByKey } from './countryNamesAt.ts';
import { MAX_NAME_HOURS, isFailedFact } from './countryFacts.ts';
import type { FactSpec } from './countryFacts';
import { matchesLookSegment, sampledLooks } from './looks.ts';
import { useContentRevision } from './contentRevision.ts';
import type { Look } from './looks';
import type { BoundTrack } from './model';
import { heldFacts } from './countryTimeline.ts';

export const LOOK_STEP = 30 * 24;

export interface RecordedBranch { firstHour: number; lastHour: number; gaps: readonly { firstHour: number; lastHour: number }[] }

export function lookHour(branch: RecordedBranch, hour: number, countryStart?: number | null, exact = false): { hour: number; stretch: number } | null {
  const moment = Math.floor(hour);
  if (countryStart === null || (countryStart !== undefined && countryStart > moment)) return null;
  if (moment < branch.firstHour || moment > branch.lastHour) return null;
  if (branch.gaps.some(gap => gap.firstHour <= moment && moment <= gap.lastHour)) return null;
  const stretch = branch.gaps.reduce((start, gap) => gap.lastHour < moment ? Math.max(start, gap.lastHour + 1) : start, Math.max(branch.firstHour, countryStart ?? branch.firstHour));
  const step = branch.firstHour + Math.floor((moment - branch.firstHour) / LOOK_STEP) * LOOK_STEP;
  return { hour: exact ? moment : Math.max(step, stretch), stretch };
}

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
  return { samples, wanted, maxHours: Math.max(MAX_NAME_HOURS, new Set(wanted.map(item => item.hour)).size) };
}

const STYLES: FactSpec<CountryStyle & { warnings?: readonly string[] }> = {
  product: 'country-styles', perRequest: Infinity,
  params: (branchId, hour, tags) => ({ branchId, hour, tags }),
  extract: data => {
    const reply = data as CountryStyleReply;
    return new Map(reply.styles.map(style => [style.tag, { ...style, warnings: reply.warnings }] as const));
  },
  held: heldFacts(style => style),
};
export const NAME_LOADING = 'Loading name…';

export interface CountryLooks {
  warnings?: readonly string[];
  name: (tag: string, hour?: number) => string;
  nameFailure?: (tag: string, hour?: number) => Failure | undefined;
  color: (tag: string, hour?: number) => [number, number, number] | null;
  flag: (tag: string, hour?: number) => string | undefined;
  flagPending: (tag: string, hour?: number) => boolean;
  flagReason: (tag: string, hour?: number) => string | undefined;
  flagDetail?: (tag: string, hour?: number) => string | undefined;
  matches?: (tag: string, hour: number) => boolean;
  revision: number;
}

export function useCountryLooks(client: DashboardClient, track: Pick<BoundTrack, 'source' | 'branch'> | undefined, tags: readonly string[], hour: number,
  active: boolean, segmentAt?: (tag: string, hour: number) => number | null, hours?: ReadonlyMap<string, number>, exact = false, flagTags?: readonly string[]): CountryLooks {
  const content = useContentRevision(client);
  const moment = Math.floor(hour);
  const tagKey = useMemo(() => JSON.stringify([...new Set(tags)].sort()), [tags]);
  const set = useMemo(() => JSON.parse(tagKey) as string[], [tagKey]);
  const plan = useMemo(() => lookPlan(track?.branch, set, moment, segmentAt, hours, exact), [set, track?.branch, moment, segmentAt, hours, exact]);
  const { samples, wanted } = plan;
  const styles = useCountryFactsByKey(client, track, wanted, active, STYLES, plan.maxHours);
  const flagKey = useMemo(() => flagTags === undefined ? null : JSON.stringify([...new Set(flagTags)].sort()), [flagTags]);
  const flagWanted = useMemo(() => {
    if (flagKey === null) return wanted;
    const visible = new Set(JSON.parse(flagKey) as string[]);
    return wanted.filter(item => visible.has(item.tag));
  }, [wanted, flagKey]);
  const flagKeys = useMemo(() => new Map(flagWanted.map(item => [item.tag, nameKey(item.hour, item.tag)])), [flagWanted]);
  const flags = useCountryFlagsRead(client, track, flagWanted, active);
  const heldKey = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, content]) : '';
  const shown = useRef<{ key: string; client: DashboardClient; looks: Map<string, Look>; segments: Map<string, number>; revision: number }>({ key: heldKey, client, looks: new Map(), segments: new Map(), revision: 0 });
  if (shown.current.key !== heldKey || shown.current.client !== client) shown.current = { key: heldKey, client, looks: new Map(), segments: new Map(), revision: 0 };
  const held = shown.current;
  const heldTags = useMemo(() => JSON.stringify([...new Set([...held.looks.keys(), ...set])].sort()), [held.looks, set]);
  const segments = useMemo(() => new Map(lookPlan(track?.branch, JSON.parse(heldTags) as string[], moment, segmentAt, hours, exact).samples.map(item => [item.tag, item.stretch])),
    [heldTags, track?.branch, moment, segmentAt, hours, exact]);
  const matches = useMemo(() => (tag: string, at: number) => matchesLookSegment(segments, tag,
    track ? lookHour(track.branch, at, segmentAt?.(tag, Math.floor(at)))?.stretch ?? null : null), [segments, track, segmentAt]);
  return useMemo(() => {
    const next = sampledLooks(held, samples, sample => {
      const key = nameKey(sample.hour, sample.tag);
      const flag = flags.get(key);
      return { style: styles.get(key), flag: flag === undefined ? undefined : isFailedFact(flag) ? flag : flag?.url ?? null };
    }, segments);
    held.looks = next.looks;
    held.segments = next.segments;
    const looks = held.looks;
    const look = (tag: string, at?: number) => at === undefined || matches(tag, at) ? looks.get(tag) : undefined;
    const flagKeyAt = (tag: string, at?: number) => {
      if (at === undefined) return flagKeys.get(tag);
      const sample = track ? lookHour(track.branch, at, segmentAt?.(tag, Math.floor(at)), exact) : null;
      const key = sample ? nameKey(sample.hour, tag) : undefined;
      return key === flagKeys.get(tag) ? key : undefined;
    };
    const warningReplies = new Set([...styles.values()].flatMap(style => style && !isFailedFact(style) && style.warnings ? [style.warnings] : []));
    const sampleKeys = new Map(samples.map(sample => [sample.tag, nameKey(sample.hour, sample.tag)]));
    const reading = (tag: string, at?: number) => active && !!track && (at === undefined || matches(tag, at)) && sampleKeys.has(tag) && !styles.has(sampleKeys.get(tag)!);
    return {
      warnings: [...new Set([...warningReplies].flat().concat([...styles.values(), ...flags.values(), ...[...looks.values()].flatMap(look => [look.nameFailure, look.flagFailure])].filter(isFailedFact).map(error => error.message)))],
      name: (tag, at) => look(tag, at)?.name ?? (look(tag, at)?.nameFailure ? failureText(look(tag, at)?.nameFailure, 'Couldn’t load country name.') : reading(tag, at) ? NAME_LOADING : 'Country name unavailable'),
      nameFailure: (tag, at) => look(tag, at)?.nameFailure,
      color: (tag, at) => look(tag, at)?.color ?? null,
      flag: (tag, at) => look(tag, at)?.flag ?? undefined,
      flagPending: (tag, at) => { const key = flagKeyAt(tag, at); return active && !!track && key !== undefined && flagReadMetadata(flags.get(key)).flagPending; },
      flagDetail: (tag, at) => { const key = flagKeyAt(tag, at); return flagReadMetadata(key === undefined ? undefined : flags.get(key)).flagDetail; },
      flagReason: (tag, at) => { const key = flagKeyAt(tag, at); return flagReadMetadata(key === undefined ? undefined : flags.get(key)).flagReason; },
      matches,
      revision: ++held.revision,
    };
  }, [samples, styles, flags, flagKeys, held, segments, track, segmentAt, matches, active]);
}
