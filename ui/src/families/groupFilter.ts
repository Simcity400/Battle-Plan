import { countryIdentityIndex, countryIdentityKey, validCountryIdentityHistories, type CountryIdentityIndex } from '../dashboard/countryIdentity.ts';
import { EVER_UNRECORDED, everKnownBefore, groupRuns, groupUnobserved, membershipUnrecordedAt, noMembersAt, worldFilter, type ResolvedCountries } from '../dashboard/groups.ts';
import type { CountrySelection, GroupMembership } from '../dashboard/model';
import { NO_MEMBERS } from '../dashboard/noMembers.ts';
import type { CountryGroupReply, CountryIdentityHistory, MembershipRun } from '../generated/dashboard';
import type { FamilyBodyStatus } from './plan.ts';
import { inHourRanges, nextMembershipChange, nextRangesChange, type HourRanges } from './holders.ts';

export const GROUP_UNAVAILABLE = 'Group membership is unavailable at this time.';
const MEMBERS_UNAVAILABLE = 'Group members are unavailable at this time.';
const NO_TAGS: readonly string[] = [];

export interface DatedMembershipOptions {
  readonly membership?: Readonly<Record<string, HourRanges>> | null;
  readonly unobserved?: HourRanges;
  readonly membershipMode?: GroupMembership | null;
}

export type ActiveGroupMembers =
  | { readonly kind: 'unfiltered' }
  | { readonly kind: 'unavailable'; readonly reason: string }
  | { readonly kind: 'empty'; readonly reason: string }
  | { readonly kind: 'filtered'; readonly active: ReadonlySet<string> };

const firstMemberHourCache = new WeakMap<object, number>();

export function firstMemberHour(membership: Readonly<Record<string, HourRanges>>): number {
  const cached = firstMemberHourCache.get(membership);
  if (cached !== undefined) return cached;
  let first = Infinity;
  for (const tag in membership) {
    const ranges = membership[tag]!;
    for (let i = 0; i < ranges.length; i++) {
      const lo = ranges[i]![0];
      if (lo < first) first = lo;
    }
  }
  firstMemberHourCache.set(membership, first);
  return first;
}

export function activeGroupMembers(options: DatedMembershipOptions | undefined, hour: number): ActiveGroupMembers {
  if (!options) return { kind: 'unfiltered' };
  if (inHourRanges(options.unobserved, hour)) {
    return { kind: 'unavailable', reason: options.membershipMode === 'ever' ? EVER_UNRECORDED : GROUP_UNAVAILABLE };
  }
  const membership = options.membership;
  if (!membership) return { kind: 'unfiltered' };
  const active = new Set<string>();
  for (const tag in membership) {
    if (inHourRanges(membership[tag], hour)) active.add(tag);
  }
  if (!active.size) {
    if (options.membershipMode === 'ever') {
      if (hour < firstMemberHour(membership)) return { kind: 'empty', reason: NO_MEMBERS.yet };
      return { kind: 'filtered', active };
    }
    return { kind: 'empty', reason: NO_MEMBERS.hour };
  }
  return { kind: 'filtered', active };
}

export function nextDatedMembershipChange(options: DatedMembershipOptions | undefined, hour: number): number {
  if (!options) return Infinity;
  return Math.min(nextMembershipChange(options.membership, hour), nextRangesChange(options.unobserved, hour));
}

export function ambiguousHolderRanges(
  histories: readonly CountryIdentityHistory[] | undefined,
): Readonly<Record<string, HourRanges>> | null {
  if (!histories?.length) return null;
  const byHolder = new Map<string, { readonly tag: string; readonly firstHour: number }[]>();
  for (let i = 0; i < histories.length; i++) {
    const history = histories[i]!;
    for (let j = 0; j < history.identities.length; j++) {
      const run = history.identities[j]!;
      const key = countryIdentityKey(run);
      let list = byHolder.get(key);
      if (!list) byHolder.set(key, list = []);
      list.push({ tag: history.tag, firstHour: run.firstHour });
    }
  }
  const eventsByTag = new Map<string, { readonly hour: number; readonly delta: 1 | -1 }[]>();
  for (const runs of byHolder.values()) {
    runs.sort((a, b) => a.firstHour - b.firstHour);
    for (let i = 0; i < runs.length; i++) {
      const cur = runs[i]!;
      if (i > 0 && runs[i - 1]!.tag === cur.tag) continue;
      if (i > 0) {
        const prevTag = runs[i - 1]!.tag;
        let prevEvents = eventsByTag.get(prevTag);
        if (!prevEvents) eventsByTag.set(prevTag, prevEvents = []);
        prevEvents.push({ hour: cur.firstHour, delta: -1 });
      }
      let curEvents = eventsByTag.get(cur.tag);
      if (!curEvents) eventsByTag.set(cur.tag, curEvents = []);
      curEvents.push({ hour: cur.firstHour, delta: 1 });
    }
  }
  let out: Record<string, HourRanges> | null = null;
  for (const [tag, events] of eventsByTag) {
    if (events.length < 2) continue;
    events.sort((a, b) => a.hour - b.hour);
    const ranges: [number, number][] = [];
    let active = 0;
    let start: number | null = null;
    let i = 0;
    while (i < events.length) {
      const hour = events[i]!.hour;
      while (i < events.length && events[i]!.hour === hour) {
        active += events[i]!.delta;
        i++;
      }
      if (active >= 2 && start === null) {
        start = hour;
      } else if (active < 2 && start !== null) {
        ranges.push([start, hour - 1]);
        start = null;
      }
    }
    if (start !== null) ranges.push([start, Number.MAX_SAFE_INTEGER]);
    if (ranges.length) (out ??= {})[tag] = ranges;
  }
  return out;
}

const mergedRangesCache = new WeakMap<object, readonly (readonly [number, number])[]>();

function mergedRanges(ranges: HourRanges): readonly (readonly [number, number])[] {
  let merged = mergedRangesCache.get(ranges);
  if (merged) return merged;
  const valid: [number, number][] = [];
  for (let i = 0; i < ranges.length; i++) {
    const [first, last] = ranges[i]!;
    if (first <= last) valid.push([first, last]);
  }
  valid.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  const out: [number, number][] = [];
  for (let i = 0; i < valid.length; i++) {
    const [first, last] = valid[i]!;
    const prev = out[out.length - 1];
    if (prev && prev[1] + 1 >= first) {
      if (last > prev[1]) prev[1] = last;
    } else {
      out.push([first, last]);
    }
  }
  merged = out;
  mergedRangesCache.set(ranges, merged);
  return merged;
}

export function cumulativeEverMembership(
  selection: CountrySelection,
  groups: CountryGroupReply | undefined,
  lastHour: number,
): Readonly<Record<string, HourRanges>> | null {
  if (selection.kind !== 'group' || selection.membership !== 'ever' || !groups) return null;
  const runs = groupRuns(groups, selection.group);
  if (!runs) return null;
  const until = Math.min(lastHour, everKnownBefore(groupUnobserved(groups, selection.group)) - 1);
  if (until < -0x8000_0000) return {};
  const index = validCountryIdentityHistories(groups.countryIdentities) ? countryIdentityIndex(groups.countryIdentities) : null;
  const byHolder = new Map<string, { readonly tag: string; readonly firstHour: number }[]>();
  for (let i = 0; i < groups.countryIdentities.length; i++) {
    const history = groups.countryIdentities[i]!;
    for (let j = 0; j < history.identities.length; j++) {
      const run = history.identities[j]!;
      const key = countryIdentityKey(run);
      let list = byHolder.get(key);
      if (!list) byHolder.set(key, list = []);
      list.push({ tag: history.tag, firstHour: run.firstHour });
    }
  }
  const raw = new Map<string, [number, number][]>();
  const add = (tag: string, lo: number, hi: number) => {
    if (lo > hi) return;
    let list = raw.get(tag);
    if (!list) raw.set(tag, list = []);
    list.push([lo, hi]);
  };
  for (let i = 0; i < runs.length; i++) {
    const run = runs[i]!;
    if (run.firstHour > until || index?.keyAt(run.tag, run.firstHour) != null) continue;
    add(run.tag, run.firstHour, Math.min(run.lastHour, until));
  }
  if (index) {
    for (const [key, joinHour] of index.firstHoursDuring(runs)) {
      if (joinHour > until) continue;
      const holderRuns = byHolder.get(key);
      if (!holderRuns?.length) continue;
      holderRuns.sort((a, b) => a.firstHour - b.firstHour);
      for (let i = 0; i < holderRuns.length; i++) {
        const cur = holderRuns[i]!;
        const nextStart = i + 1 < holderRuns.length ? holderRuns[i + 1]!.firstHour - 1 : until;
        const lo = Math.max(joinHour, cur.firstHour);
        const hi = Math.min(until, nextStart);
        if (lo <= hi) add(cur.tag, lo, hi);
      }
    }
  }
  const out: Record<string, HourRanges> = {};
  for (const [tag, spans] of raw) {
    out[tag] = mergedRanges(spans);
  }
  return out;
}

export interface IncompleteHourState {
  readonly kind: 'empty' | 'unrecorded';
  readonly from: number;
  readonly until: number;
}

export function incompleteHourStateAt(unobserved: HourRanges, hour: number): IncompleteHourState {
  const spans = mergedRanges(unobserved);
  let low = 0;
  let high = spans.length;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (spans[mid]![1] < hour) low = mid + 1;
    else high = mid;
  }
  const current = spans[low];
  if (current && current[0] <= hour) {
    return { kind: 'unrecorded', from: current[0], until: current[1] };
  }
  const prev = low > 0 ? spans[low - 1]! : null;
  return {
    kind: 'empty',
    from: prev ? prev[1] + 1 : -Infinity,
    until: current ? current[0] - 1 : Infinity,
  };
}

export function incompleteGroupBodyStatus(
  kind: IncompleteHourState['kind'] | null,
  following: boolean,
): FamilyBodyStatus | null {
  if (kind === 'unrecorded') return { kind: 'not-recorded', reason: GROUP_UNAVAILABLE };
  if (kind === 'empty') return { kind: 'no-members', reason: following ? NO_MEMBERS.hour : NO_MEMBERS.date };
  return null;
}

export function staticDatedGroupBodyStatus(
  selection: CountrySelection,
  effectiveMulti: ResolvedCountries,
  groups: CountryGroupReply | undefined,
  membership: Readonly<Record<string, HourRanges>> | null,
  moment: number,
  rankedAtMoment: readonly string[],
): FamilyBodyStatus | null {
  const unrecorded = groups ? membershipUnrecordedAt(selection, groups, moment) : null;
  if (unrecorded) return { kind: 'not-recorded', reason: unrecorded };
  const none = noMembersAt(selection, effectiveMulti, groups, moment);
  if (none) return { kind: 'no-members', reason: none.reason };
  if (effectiveMulti.top == null && !rankedAtMoment.length) {
    if (selection.kind === 'group' && groups) {
      if (selection.membership === 'ever' && membership !== null && firstMemberHour(membership) <= moment) {
        return null;
      }
      const why = worldFilter(selection, [], groups, moment).reason;
      if (why === null) return null;
      return { kind: 'refused', reason: why };
    }
    return { kind: 'refused', reason: selection.kind === 'group' ? NO_MEMBERS.date : 'No countries available for this selection.' };
  }
  return null;
}

export interface TimelineSelectionOptions {
  readonly selection?: CountrySelection;
  readonly fallback?: readonly string[];
  readonly selected: readonly string[] | null;
  readonly membership?: Readonly<Record<string, HourRanges>> | null;
  readonly unobserved?: HourRanges;
}

export type ResolvedTimelineSelection =
  | { readonly kind: 'unfiltered' }
  | { readonly kind: 'empty'; readonly reason: string; readonly emptyKind: 'empty' | 'unavailable' }
  | { readonly kind: 'filtered'; readonly countries: ReadonlySet<string>; readonly countryIds: ReadonlySet<string> };

export function nextTimelineSelectionChange(
  opt: TimelineSelectionOptions,
  hour: number,
): number {
  return Math.min(nextMembershipChange(opt.membership, hour), nextRangesChange(opt.unobserved, hour));
}

export function resolveTimelineSelection(
  opt: TimelineSelectionOptions,
  groups: CountryGroupReply,
  identities: CountryIdentityIndex,
  hour: number,
): ResolvedTimelineSelection {
  if (opt.selection) {
    const selection = opt.selection;
    if (selection.kind === 'all') return { kind: 'unfiltered' };
    if (selection.kind === 'factions') {
      return { kind: 'empty', reason: 'This widget does not show faction totals.', emptyKind: 'unavailable' };
    }
    if (selection.kind === 'group') {
      const unrecorded = membershipUnrecordedAt(selection, groups, hour);
      if (unrecorded !== null) return { kind: 'empty', reason: unrecorded, emptyKind: 'unavailable' };
      if (inHourRanges(opt.unobserved, hour)) {
        return { kind: 'empty', reason: selection.membership === 'ever' ? EVER_UNRECORDED : GROUP_UNAVAILABLE, emptyKind: 'unavailable' };
      }
      const runs = groupRuns(groups, selection.group);
      if (runs === null) {
        return selection.group.type === 'faction'
          ? { kind: 'empty', reason: selection.membership === 'ever' ? NO_MEMBERS.yet : NO_MEMBERS.hour, emptyKind: 'empty' }
          : { kind: 'empty', reason: 'This group is unavailable in this campaign.', emptyKind: 'unavailable' };
      }
      if (selection.membership === 'ever') {
        const seen: MembershipRun[] = [];
        for (let i = 0; i < runs.length; i++) {
          const run = runs[i]!;
          if (run.firstHour <= hour) seen.push(run.lastHour > hour ? { ...run, lastHour: hour } : run);
        }
        if (!seen.length) return { kind: 'empty', reason: NO_MEMBERS.yet, emptyKind: 'empty' };
        const countryIds = identities.keysDuring(seen);
        if (!countryIds.size) return { kind: 'empty', reason: MEMBERS_UNAVAILABLE, emptyKind: 'unavailable' };
        return { kind: 'filtered', countries: new Set(identities.aliasesAt(countryIds, hour)), countryIds };
      }
      const activeTags = new Set<string>();
      for (let i = 0; i < runs.length; i++) {
        const run = runs[i]!;
        if (run.firstHour <= hour && hour <= run.lastHour) activeTags.add(run.tag);
      }
      if (!activeTags.size) return { kind: 'empty', reason: NO_MEMBERS.hour, emptyKind: 'empty' };
      const atHour: MembershipRun[] = [];
      for (const tag of activeTags) atHour.push({ tag, firstHour: hour, lastHour: hour });
      const countryIds = identities.keysDuring(atHour);
      if (!countryIds.size) return { kind: 'empty', reason: MEMBERS_UNAVAILABLE, emptyKind: 'unavailable' };
      return { kind: 'filtered', countries: new Set(identities.aliasesAt(countryIds, hour)), countryIds };
    }
    const tags = selection.kind === 'tags' ? selection.tags : (opt.fallback ?? opt.selected ?? NO_TAGS);
    if (!tags.length) return { kind: 'empty', reason: 'Choose a country in the widget settings.', emptyKind: 'empty' };
    const countryIds = identities.selected(tags);
    return { kind: 'filtered', countries: new Set(identities.aliasesAt(countryIds, hour)), countryIds };
  }
  if (inHourRanges(opt.unobserved, hour)) {
    return { kind: 'empty', reason: GROUP_UNAVAILABLE, emptyKind: 'unavailable' };
  }
  if (!opt.selected) return { kind: 'unfiltered' };
  if (opt.membership) {
    const membership = opt.membership;
    const active = opt.selected.filter(tag => inHourRanges(membership[tag], hour));
    if (!active.length) return { kind: 'empty', reason: NO_MEMBERS.hour, emptyKind: 'empty' };
    const countryIds = identities.keysDuring(active.map(tag => ({ tag, firstHour: hour, lastHour: hour })));
    if (!countryIds.size) {
      return { kind: 'empty', reason: MEMBERS_UNAVAILABLE, emptyKind: 'unavailable' };
    }
    return { kind: 'filtered', countries: new Set(active), countryIds };
  }
  return { kind: 'filtered', countries: new Set(opt.selected), countryIds: identities.selected(opt.selected) };
}
