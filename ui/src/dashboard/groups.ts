import type { CountryGroupReply, HourSpan, MembershipRun } from '../generated/dashboard';
import { compareTags } from './tagOrder.ts';
import type { CountryGroup, CountrySelection, GroupMembership } from './model';
import { countryIdentityIndex } from './countryIdentity.ts';
import type { CountryIdentityIndex } from './countryIdentity.ts';
import { PlaybackIntervals, spanChanges } from '../charts/playbackIntervals.ts';
import { NO_MEMBERS } from './noMembers.ts';
import { countryFromKey } from './countryKey.ts';

export function groupRuns(reply: CountryGroupReply, group: CountryGroup): MembershipRun[] | null {
  switch (group.type) {
    case 'humans': return reply.humans;
    case 'faction': return reply.factions.find(faction => faction.id === group.id)?.members ?? null;
    case 'war': return warEnemies(reply, group.tag);
    case 'top': return null;
  }
}

function warEnemies(reply: CountryGroupReply, chosen: string): MembershipRun[] {
  const slot = countryFromKey(chosen);
  const held = typeof slot === 'number'
    ? reply.countryIdentities.flatMap(history => history.identities.filter(run => run.slot === slot).map(run => ({ tag: history.tag, firstHour: run.firstHour, lastHour: run.lastHour })))
    : [{ tag: chosen, firstHour: -Infinity, lastHour: Infinity }];
  return reply.wars.flatMap(pair => held.flatMap(own => {
    const enemy = pair.first === own.tag ? pair.second : pair.second === own.tag ? pair.first : null;
    const firstHour = Math.max(pair.firstHour, own.firstHour), lastHour = Math.min(pair.lastHour, own.lastHour);
    return enemy !== null && firstHour <= lastHour ? [{ tag: enemy, firstHour, lastHour }] : [];
  }));
}

export function groupUnobserved(reply: CountryGroupReply, group: CountryGroup): HourSpan[] {
  switch (group.type) {
    case 'humans': return reply.unobserved.humans;
    case 'faction': return [...reply.unobserved.factions, ...(reply.factions.find(faction => faction.id === group.id)?.incomplete ?? [])]
      .sort((a, b) => a.firstHour - b.firstHour);
    case 'war': return reply.unobserved.wars;
    case 'top': return [];
  }
}

export function groupMembers(runs: readonly MembershipRun[], membership: GroupMembership, hour: number,
  max = Infinity, unobserved: readonly HourSpan[] = []): string[] | null {
  if (membership === 'hour' && unobserved.some(span => span.firstHour <= hour && hour <= span.lastHour)) return null;
  const ordered = [...runs].sort((a, b) => a.firstHour - b.firstHour || a.tag.localeCompare(b.tag));
  const tags = ordered.filter(run => membership === 'ever' || (run.firstHour <= hour && hour <= run.lastHour)).map(run => run.tag);
  if (membership === 'ever' && !tags.length && unobserved.length) return null;
  return [...new Set(tags)].slice(0, max);
}

export const DEFAULT_TOP = 10;

export function countrySelectionModes(limits: { factions?: boolean; all?: boolean }): { value: string; label: string }[] {
  return [...(limits.all ? [{ value: 'all', label: 'All' }] : []), { value: 'countries', label: 'Countries' }, { value: 'group', label: 'Group' },
    ...(limits.factions ? [{ value: 'factions', label: 'Factions' }] : [])];
}

export function countrySelectionMode(selection: CountrySelection): string {
  return selection.kind === 'all' ? 'all' : selection.kind === 'factions' ? 'factions' : selection.kind === 'group' ? 'group' : 'countries';
}

export function countrySelectionFor(mode: string, max: number): CountrySelection {
  const count = Math.min(DEFAULT_TOP, max);
  return mode === 'all' ? { kind: 'all' } : mode === 'factions' ? { kind: 'factions', count, membership: 'hour' }
    : mode === 'group' ? { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false } : { kind: 'selected' };
}

export function isMember(membership: readonly HourSpan[] | undefined, hour: number): boolean {
  return !membership || membership.some(run => run.firstHour <= hour && hour <= run.lastHour);
}

function unrecorded(membership: GroupMembership): string {
  switch (membership) {
    case 'hour': return 'Group membership is unavailable at this time.';
    case 'ever': return 'Group membership is incomplete for this campaign.';
  }
}

export type MembershipKnown = (firstHour: number, lastHour?: number) => boolean;

const ALWAYS: MembershipKnown = () => true;
const NEVER: MembershipKnown = () => false;

const choiceLoaded = (selection: CountrySelection, groups: CountryGroupReply | undefined) => selection.kind !== 'group' || groups !== undefined;

export interface WorldFilter {
  readonly tags: ReadonlySet<string> | null;
  readonly reason: string | null;
  readonly known: boolean;
  readonly countryIds?: ReadonlySet<string> | null;
}

function identitiesFor(groups: CountryGroupReply | undefined, index: CountryIdentityIndex | null | undefined): CountryIdentityIndex | null {
  return index === undefined ? groups && Array.isArray(groups.countryIdentities) ? countryIdentityIndex(groups.countryIdentities) : null : index;
}

function readWorldFilter(selection: CountrySelection, fallback: readonly string[], groups: CountryGroupReply | undefined,
  hour: number, index?: CountryIdentityIndex | null): WorldFilter {
  const identities = identitiesFor(groups, index);
  const filter = settledWorldFilter(selection, fallback, groups, hour, identities);
  if (selection.kind === 'all' || filter.reason === CHOOSE) return { ...filter, known: true };
  const known = !!identities && choiceLoaded(selection, groups);
  return known ? { ...filter, known } : { ...filter, reason: null, known };
}

const CHOOSE = 'Choose a country in the widget settings.';
const UNAVAILABLE = 'This group is unavailable in this campaign.';

type FilterQueries = Map<string, PlaybackIntervals<WorldFilter>>;
const groupFilters = new WeakMap<CountryGroupReply, Map<CountryIdentityIndex | null, FilterQueries>>();
const identityFilters = new WeakMap<CountryIdentityIndex, FilterQueries>();
const unrecordedFilters: FilterQueries = new Map();
const filterHours = new WeakMap<CountryGroupReply, ReadonlyArray<number>>();

function worldFilterHours(groups: CountryGroupReply): ReadonlyArray<number> {
  let hours = filterHours.get(groups);
  if (!hours) {
    hours = [...spanChanges([...groups.humans, ...groups.wars, ...groups.factions.flatMap(faction => [...faction.members, ...faction.incomplete ?? []]),
      ...groups.unobserved.humans, ...groups.unobserved.factions, ...groups.unobserved.wars])];
    filterHours.set(groups, hours);
  }
  return hours;
}

export function selectionChangeHours(selection: CountrySelection, fallback: readonly string[], groups: CountryGroupReply | undefined,
  index: CountryIdentityIndex | null): readonly number[] {
  if (selection.kind === 'all' || !index) return [];
  if (selection.kind !== 'group') return index.changeHoursOf(selection.kind === 'tags' ? selection.tags : fallback);
  const runs = groups ? groupRuns(groups, selection.group) : null;
  if (!groups || !runs) return [];
  return [...new Set([...spanChanges([...runs, ...groupUnobserved(groups, selection.group)]), ...index.changeHoursOf([...new Set(runs.map(run => run.tag))])])]
    .sort((a, b) => a - b);
}

export function worldFilter(selection: CountrySelection, fallback: readonly string[], groups: CountryGroupReply | undefined,
  hour: number, index?: CountryIdentityIndex | null): WorldFilter {
  return worldFilterQuery(selection, fallback, groups, index).at(hour);
}

export function worldFilterQuery(selection: CountrySelection, fallback: readonly string[], groups: CountryGroupReply | undefined,
  index?: CountryIdentityIndex | null): PlaybackIntervals<WorldFilter> {
  const identities = identitiesFor(groups, index);
  let queries: FilterQueries;
  if (groups) {
    let byIndex = groupFilters.get(groups);
    if (!byIndex) groupFilters.set(groups, byIndex = new Map());
    queries = byIndex.get(identities) ?? new Map();
    byIndex.set(identities, queries);
  } else if (identities) {
    queries = identityFilters.get(identities) ?? new Map();
    identityFilters.set(identities, queries);
  } else queries = unrecordedFilters;
  const key = JSON.stringify([selection, fallback]);
  let intervals = queries.get(key);
  if (!intervals) {
    const sets = new Map<string, ReadonlySet<string>>();
    const intern = (set: ReadonlySet<string> | null | undefined) => {
      if (set == null) return set;
      const key = JSON.stringify([...set].sort(compareTags));
      const held = sets.get(key);
      if (held) return held;
      sets.set(key, set); return set;
    };
    intervals = new PlaybackIntervals<WorldFilter>([...(groups ? worldFilterHours(groups) : []), ...identities?.changeHours ?? []], at => {
      const filter = readWorldFilter(selection, fallback, groups, at, identities);
      return { ...filter, tags: intern(filter.tags) ?? null, ...(filter.countryIds === undefined ? {} : { countryIds: intern(filter.countryIds) }) };
    });
    queries.set(key, intervals);
  }
  return intervals;
}

function settledWorldFilter(selection: CountrySelection, fallback: readonly string[], groups: CountryGroupReply | undefined,
  hour: number, identities: CountryIdentityIndex | null): Omit<WorldFilter, 'known'> {
  if (selection.kind === 'all') return { tags: null, reason: null };
  const empty = (reason: string | null) => ({ tags: new Set<string>(), countryIds: new Set<string>(), reason });
  if (selection.kind !== 'group') {
    const tags = selection.kind === 'tags' ? selection.tags : fallback;
    if (!tags.length) return empty(CHOOSE);
    if (!identities) return empty(null);
    const countryIds = identities.selected(tags);
    return { tags: new Set(identities.aliasesAt(countryIds, hour)), countryIds,
      reason: countryIds.size ? null : 'Selected countries are unavailable in this campaign.' };
  }
  if (!groups || !identities) return empty(null);
  const runs = groupRuns(groups, selection.group);
  if (!runs) return empty(UNAVAILABLE);
  const members = groupMembers(runs, selection.membership, hour, Infinity, groupUnobserved(groups, selection.group));
  if (!members) return empty(unrecorded(selection.membership));
  const countryIds = selection.membership === 'ever' ? identities.keysDuring(runs)
    : identities.keysDuring(members.map(tag => ({ tag, firstHour: hour, lastHour: hour })));
  return { tags: new Set(identities.aliasesAt(countryIds, hour)), countryIds, reason: members.length
    ? countryIds.size ? null : 'Group members are unavailable at this time.'
    : selection.membership === 'ever' ? NO_MEMBERS.ever : NO_MEMBERS.hour };
}

export interface CountryMatch { ready: boolean; matches: (tag: string, hour: number) => boolean; known: MembershipKnown; reason: string | null }

const matchChanges = new WeakMap<CountryMatch['matches'], ReadonlyArray<number>>();
export const countryMatchChangeHours = (matches: CountryMatch['matches']) => matchChanges.get(matches);

export function membershipUnrecordedAt(selection: CountrySelection, groups: CountryGroupReply | undefined, hour: number): string | null {
  if (selection.kind !== 'group' || selection.membership !== 'hour' || !groups) return null;
  return groupUnobserved(groups, selection.group).some(span => span.firstHour <= hour && hour <= span.lastHour) ? unrecorded('hour') : null;
}

export function countryMatch(selection: CountrySelection, fallback: readonly string[], groups: CountryGroupReply | undefined, lastHour: number,
  index?: CountryIdentityIndex | null): CountryMatch {
  const result = readCountryMatch(selection, fallback, groups, lastHour, index);
  const identities = identitiesFor(groups, index);
  matchChanges.set(result.matches, [...(groups ? worldFilterHours(groups) : []), ...identities?.changeHours ?? []]);
  return result;
}

function readCountryMatch(selection: CountrySelection, fallback: readonly string[], groups: CountryGroupReply | undefined, lastHour: number,
  index?: CountryIdentityIndex | null): CountryMatch {
  if (selection.kind === 'all') return { ready: true, matches: () => true, known: ALWAYS, reason: null };
  const identities = identitiesFor(groups, index);
  const loading: CountryMatch = { ready: false, matches: () => false, known: NEVER, reason: null };
  if (!identities || selection.kind === 'group' && !groups) return loading;
  if (selection.kind === 'group' && selection.membership === 'hour') {
    const runs = groupRuns(groups!, selection.group);
    if (!runs) return { ready: true, matches: () => false, known: ALWAYS, reason: UNAVAILABLE };
    const unobserved = groupUnobserved(groups!, selection.group);
    if (!runs.length) return { ready: true, matches: () => false, known: ALWAYS, reason: unobserved.length ? unrecorded('ever') : NO_MEMBERS.ever };
    const byTag = new Map<string, MembershipRun[]>();
    for (const run of runs) {
      const history = byTag.get(run.tag);
      if (history) history.push(run); else byTag.set(run.tag, [run]);
    }
    const hidden = (hour: number) => unobserved.some(span => span.firstHour <= hour && hour <= span.lastHour);
    return { ready: true, matches: (tag, hour) => identities.keyAt(tag, hour) !== null && !hidden(hour)
      && (byTag.get(tag) ?? []).some(run => run.firstHour <= hour && hour <= run.lastHour), known: ALWAYS, reason: null };
  }
  const { countryIds, reason } = worldFilter(selection, fallback, groups, lastHour, identities);
  return { ready: true, matches: (tag, hour) => {
    const key = identities.keyAt(tag, hour);
    return key !== null && countryIds?.has(key) === true;
  }, known: ALWAYS, reason };
}

export interface ResolvedCountries {
  tags: string[];
  membership: Map<string, MembershipRun[]> | null;
  unobserved: HourSpan[];
  dropped: number;
  reason: string | null;
  known: MembershipKnown;
  top?: number;
  factionId?: string;
}

export function rankedCandidates(resolved: ResolvedCountries, hour: number, all: readonly string[]): string[] {
  if (resolved.top != null) return [...all];
  if (!resolved.membership) return resolved.tags;
  return resolved.tags.filter(tag => isMember(resolved.membership!.get(tag), hour));
}

export function resolveCountries(selection: CountrySelection, fallback: string[], groups: CountryGroupReply | undefined, lastHour: number, max: number, factionTotals = false): ResolvedCountries {
  const none = { tags: [], membership: null, unobserved: [], dropped: 0, known: ALWAYS };
  if (selection.kind === 'factions') return { ...none, reason: 'This widget does not show faction totals.' };
  if (factionTotals && selection.kind === 'group' && selection.aggregate && selection.group.type === 'faction') {
    return { ...none, reason: null, factionId: selection.group.id };
  }
  if (selection.kind !== 'group') {
    const tags = selection.kind === 'tags' ? selection.tags : fallback;
    return { ...none, tags: tags.slice(0, max), dropped: Math.max(0, tags.length - max), reason: tags.length ? null : CHOOSE };
  }
  if (selection.group.type === 'top') return { ...none, reason: null, top: Math.max(1, Math.min(max, selection.group.count)) };
  if (!groups) return { ...none, reason: null, known: NEVER };
  const known = ALWAYS;
  const runs = groupRuns(groups, selection.group);
  if (!runs) return { ...none, reason: UNAVAILABLE, known };
  const unobserved = groupUnobserved(groups, selection.group);
  const members = groupMembers(runs, 'ever', lastHour, Infinity, unobserved);
  if (!members) return { ...none, unobserved, reason: unrecorded('ever'), known };
  const tags = members.slice(0, max);
  const membership = selection.membership === 'hour' ? new Map(tags.map(tag => [tag, [] as MembershipRun[]])) : null;
  if (membership) for (const run of runs) membership.get(run.tag)?.push(run);
  return { tags, membership, unobserved, dropped: Math.max(0, members.length - max), known,
    reason: members.length ? null : NO_MEMBERS.ever };
}

export function noMembersAt(selection: CountrySelection, resolved: ResolvedCountries, groups: CountryGroupReply | undefined,
  hour: number): { reason: string } | null {
  if (selection.kind !== 'group' || resolved.top != null || resolved.factionId != null || resolved.tags.length || resolved.reason || !groups) return null;
  const { reason } = worldFilter(selection, [], groups, hour);
  return reason === null ? null : { reason };
}

export interface WorldRequestTags { tags: string[]; ready: boolean; reason: string | null }
export function worldRequestTags(selection: CountrySelection, fallback: string[], groups: CountryGroupReply | undefined, lastHour: number,
  index?: CountryIdentityIndex | null): WorldRequestTags {
  if (selection.kind === 'all' || (selection.kind === 'group' && selection.group.type === 'top')) return { tags: [], ready: true, reason: null };
  const identities = identitiesFor(groups, index);
  if (!identities || selection.kind === 'group' && !groups) return { tags: [], ready: false, reason: null };
  const resolved = resolveCountries(selection, fallback, groups, lastHour, Number.MAX_SAFE_INTEGER);
  const counted = selection.kind === 'group' && selection.membership === 'hour' ? { ...selection, membership: 'ever' as const } : selection;
  const reason = resolved.reason ?? worldFilter(counted, fallback, groups, lastHour, identities).reason;
  return { tags: resolved.tags, ready: reason === null && resolved.tags.length > 0, reason };
}
