import type { CountryGroupReply, HourSpan, MembershipRun } from '../generated/dashboard';
import { compareTags } from './tagOrder.ts';
import type { CountryGroup, CountrySelection, GroupMembership } from './model';
import { countryIdentityIndex, validCountryIdentityHistories } from './countryIdentity.ts';
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
  if (membership === 'ever' && hour >= everKnownBefore(unobserved)) return null;
  const ordered = [...runs].sort((a, b) => a.firstHour - b.firstHour || a.tag.localeCompare(b.tag));
  const tags = ordered.filter(run => membership === 'ever' ? run.firstHour <= hour : run.firstHour <= hour && hour <= run.lastHour).map(run => run.tag);
  return [...new Set(tags)].slice(0, max);
}

export function everKnownBefore(unobserved: readonly HourSpan[]): number {
  let first = Infinity;
  for (const span of unobserved) if (span.firstHour < first) first = span.firstHour;
  return first;
}

export interface EverBounds {
  readonly firstJoin: number;
  readonly unknownFrom: number;
}

export interface EverState {
  readonly kind: 'empty' | 'unrecorded' | null;
  readonly from: number;
  readonly until: number;
}

export function everBounds(selection: CountrySelection, groups: CountryGroupReply | undefined): EverBounds | null {
  if (selection.kind !== 'group' || selection.membership !== 'ever' || !groups) return null;
  const runs = groupRuns(groups, selection.group);
  if (!runs) return null;
  const unobserved = groupUnobserved(groups, selection.group);
  if (!runs.length && !unobserved.length) return null;
  let firstJoin = Infinity;
  for (const run of runs) if (run.firstHour < firstJoin) firstJoin = run.firstHour;
  return { firstJoin, unknownFrom: everKnownBefore(unobserved) };
}

export function everStateAt(bounds: EverBounds, hour: number): EverState {
  if (hour >= bounds.unknownFrom) return { kind: 'unrecorded', from: bounds.unknownFrom, until: Infinity };
  if (hour < bounds.firstJoin) return { kind: 'empty', from: -Infinity, until: Math.min(bounds.firstJoin, bounds.unknownFrom) - 1 };
  return { kind: null, from: bounds.firstJoin, until: bounds.unknownFrom - 1 };
}

export const EVER_UNRECORDED = 'Group membership was not recorded at this time.';

function soFarMembership(groups: CountryGroupReply, runs: readonly MembershipRun[], until: number, max: number): { membership: Map<string, MembershipRun[]>; countries: number } {
  const index = validCountryIdentityHistories(groups.countryIdentities) ? countryIdentityIndex(groups.countryIdentities) : null;
  const countries: { id: string; from: number; spans: MembershipRun[] }[] = [];
  const loose = new Map<string, { id: string; from: number; spans: MembershipRun[] }>();
  for (const run of runs) {
    if (run.firstHour > until || index?.keyAt(run.tag, run.firstHour) != null) continue;
    const span = { tag: run.tag, firstHour: run.firstHour, lastHour: Math.min(run.lastHour, until) };
    const held = loose.get(run.tag);
    if (held) { held.spans.push(span); held.from = Math.min(held.from, span.firstHour); } else loose.set(run.tag, { id: `tag:${run.tag}`, from: span.firstHour, spans: [span] });
  }
  countries.push(...loose.values());
  if (index) for (const [key, from] of index.firstHoursDuring(runs)) if (from <= until) countries.push({ id: key, from, spans: index.aliasRuns(new Set([key]), from, until) });
  countries.sort((a, b) => a.from - b.from || a.id.localeCompare(b.id));
  const spans = countries.slice(0, max).flatMap(country => country.spans);
  spans.sort((a, b) => a.firstHour - b.firstHour || a.tag.localeCompare(b.tag));
  const membership = new Map<string, MembershipRun[]>();
  for (const span of spans) {
    const own = membership.get(span.tag);
    const previous = own?.at(-1);
    if (!own) membership.set(span.tag, [{ ...span }]);
    else if (previous && span.firstHour <= previous.lastHour + 1) own[own.length - 1] = { ...previous, lastHour: Math.max(previous.lastHour, span.lastHour) };
    else own.push({ ...span });
  }
  return { membership, countries: countries.length };
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
    case 'ever': return EVER_UNRECORDED;
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
const INCOMPLETE = 'Group membership is incomplete for this campaign.';
const MEMBERS_UNAVAILABLE = 'Group members are unavailable at this time.';
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
  const countryIds = selection.membership === 'ever'
    ? identities.keysDuring(runs.filter(run => run.firstHour <= hour).map(run => ({ ...run, lastHour: Math.min(run.lastHour, hour) })))
    : identities.keysDuring(members.map(tag => ({ tag, firstHour: hour, lastHour: hour })));
  return { tags: new Set(identities.aliasesAt(countryIds, hour)), countryIds, reason: members.length
    ? countryIds.size ? null : MEMBERS_UNAVAILABLE
    : selection.membership === 'ever' ? NO_MEMBERS.yet : NO_MEMBERS.hour };
}

export interface CountryMatch { ready: boolean; matches: (tag: string, hour: number) => boolean; known: MembershipKnown; reason: string | null }

const matchChanges = new WeakMap<CountryMatch['matches'], ReadonlyArray<number>>();
export const countryMatchChangeHours = (matches: CountryMatch['matches']) => matchChanges.get(matches);

export function membershipUnrecordedAt(selection: CountrySelection, groups: CountryGroupReply | undefined, hour: number): string | null {
  if (selection.kind !== 'group' || !groups) return null;
  const unobserved = groupUnobserved(groups, selection.group);
  if (selection.membership === 'ever') return hour >= everKnownBefore(unobserved) ? unrecorded('ever') : null;
  return unobserved.some(span => span.firstHour <= hour && hour <= span.lastHour) ? unrecorded('hour') : null;
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
    if (!runs.length) return { ready: true, matches: () => false, known: ALWAYS, reason: unobserved.length ? INCOMPLETE : NO_MEMBERS.ever };
    const byTag = new Map<string, MembershipRun[]>();
    for (const run of runs) {
      const history = byTag.get(run.tag);
      if (history) history.push(run); else byTag.set(run.tag, [run]);
    }
    const hidden = (hour: number) => unobserved.some(span => span.firstHour <= hour && hour <= span.lastHour);
    return { ready: true, matches: (tag, hour) => identities.keyAt(tag, hour) !== null && !hidden(hour)
      && (byTag.get(tag) ?? []).some(run => run.firstHour <= hour && hour <= run.lastHour), known: ALWAYS, reason: null };
  }
  if (selection.kind === 'group' && selection.membership === 'ever') {
    const runs = groupRuns(groups!, selection.group);
    if (!runs) return { ready: true, matches: () => false, known: ALWAYS, reason: UNAVAILABLE };
    const unobserved = groupUnobserved(groups!, selection.group);
    if (!runs.length && !unobserved.length) return { ready: true, matches: () => false, known: ALWAYS, reason: NO_MEMBERS.ever };
    const until = everKnownBefore(unobserved);
    const first = identities.firstHoursDuring(runs);
    return { ready: true, matches: (tag, hour) => {
      if (hour >= until) return false;
      const key = identities.keyAt(tag, hour);
      return key !== null && (first.get(key) ?? Infinity) <= hour;
    }, known: ALWAYS, reason: null };
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
  if (selection.membership === 'ever') {
    const { membership, countries } = soFarMembership(groups, runs, Math.min(lastHour, everKnownBefore(unobserved) - 1), max);
    if (!countries && !unobserved.length) return { ...none, reason: NO_MEMBERS.ever, known };
    return { tags: [...membership.keys()], membership, unobserved, dropped: Math.max(0, countries - max), known, reason: null };
  }
  const members = [...new Set([...runs].sort((a, b) => a.firstHour - b.firstHour || a.tag.localeCompare(b.tag)).map(run => run.tag))];
  if (!members.length && unobserved.length) return { ...none, unobserved, reason: INCOMPLETE, known };
  const tags = members.slice(0, max);
  const membership = new Map(tags.map(tag => [tag, [] as MembershipRun[]]));
  for (const run of runs) membership.get(run.tag)?.push(run);
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
  if (selection.kind !== 'group') {
    const resolved = resolveCountries(selection, fallback, groups, lastHour, Number.MAX_SAFE_INTEGER);
    const reason = resolved.reason ?? worldFilter(selection, fallback, groups, lastHour, identities).reason;
    return { tags: resolved.tags, ready: reason === null && resolved.tags.length > 0, reason };
  }
  const runs = groupRuns(groups!, selection.group);
  if (!runs) return { tags: [], ready: false, reason: UNAVAILABLE };
  const tags = [...new Set([...runs].sort((a, b) => a.firstHour - b.firstHour || a.tag.localeCompare(b.tag)).map(run => run.tag))];
  const reason = !tags.length ? groupUnobserved(groups!, selection.group).length ? INCOMPLETE : NO_MEMBERS.ever
    : identities.keysDuring(runs).size ? null : MEMBERS_UNAVAILABLE;
  return { tags, ready: reason === null, reason };
}
