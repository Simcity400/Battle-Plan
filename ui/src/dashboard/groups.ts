import type { CountryGroupReply, HourSpan, MembershipRun } from '../generated/dashboard';
import { clipToSpans } from '../charts/series.ts';
import { withoutHours } from '../charts/timeline/runState.ts';
import { factionNameLabel } from './factionNames.ts';
import { compareTags } from './tagOrder.ts';
import type { Run } from '../charts/series';
import type { CountryGroup, CountrySelection, GroupMembership } from './model';
import { countryIdentityIndex } from './countryIdentity.ts';
import type { CountryIdentityIndex } from './countryIdentity.ts';

/** Smart country groups, resolved from the recording's own history: human players,
 * any recorded faction (including ones founded or dissolved partway through),
 * everyone at war with a country, and the top N by the widget's own measure. */

/** Membership runs for a group; null for top N, which the widget ranks itself,
 * and for a faction the branch never recorded. */
export function groupRuns(reply: CountryGroupReply, group: CountryGroup): MembershipRun[] | null {
  switch (group.type) {
    case 'humans': return reply.humans;
    case 'faction': return reply.factions.find(faction => faction.id === group.id)?.members ?? null;
    case 'war': return reply.wars.flatMap(pair => pair.first === group.tag ? [{ tag: pair.second, firstHour: pair.firstHour, lastHour: pair.lastHour }]
      : pair.second === group.tag ? [{ tag: pair.first, firstHour: pair.firstHour, lastHour: pair.lastHour }] : []);
    case 'top': return null;
  }
}

/** The hours this group's own history was not observed. */
export function groupUnobserved(reply: CountryGroupReply, group: CountryGroup): HourSpan[] {
  switch (group.type) {
    case 'humans': return reply.unobserved.humans;
    // A faction's own incomplete rosters as well as the hours no faction was read.
    case 'faction': return [...reply.unobserved.factions, ...(reply.factions.find(faction => faction.id === group.id)?.incomplete ?? [])]
      .sort((a, b) => a.firstHour - b.firstHour);
    case 'war': return reply.unobserved.wars;
    case 'top': return [];
  }
}

/** The group's members under a membership rule, in order of first membership:
 * at `hour`, at the branch's last hour, or anyone ever a member. At most `max`,
 * the widget's own country limit, when given. Null when membership was not
 * recorded at the hour counted (for anyone ever: no member was recorded and some
 * hours were not), so a missing hour never reads as an empty group. */
export function groupMembers(runs: readonly MembershipRun[], membership: GroupMembership, hour: number, lastHour: number,
  max = Infinity, unobserved: readonly HourSpan[] = []): string[] | null {
  const at = membership === 'hour' ? hour : lastHour;
  if (membership !== 'ever' && unobserved.some(span => span.firstHour <= at && at <= span.lastHour)) return null;
  const ordered = [...runs].sort((a, b) => a.firstHour - b.firstHour || a.tag.localeCompare(b.tag));
  const tags = ordered.filter(run => membership === 'ever' || (run.firstHour <= at && at <= run.lastHour)).map(run => run.tag);
  if (membership === 'ever' && !tags.length && unobserved.length) return null;
  return [...new Set(tags)].slice(0, max);
}

export interface MembershipChange { hour: number; tag: string; change: 'joined' | 'left' }

/** Joins and leaves as timeline markers. A run that starts at the first recorded
 * hour or ends at the last is where the recording starts or stops, and a run that
 * starts or ends next to an unobserved hour is a gap in the recording; neither is
 * a change. */
export function membershipChanges(runs: readonly MembershipRun[], firstHour: number, lastHour: number, unobserved: readonly HourSpan[] = []): MembershipChange[] {
  const hidden = (hour: number) => unobserved.some(span => span.firstHour <= hour && hour <= span.lastHour);
  const changes: MembershipChange[] = [];
  for (const run of runs) {
    if (run.firstHour > firstHour && !hidden(run.firstHour - 1)) changes.push({ hour: run.firstHour, tag: run.tag, change: 'joined' });
    if (run.lastHour < lastHour && !hidden(run.lastHour + 1)) changes.push({ hour: run.lastHour + 1, tag: run.tag, change: 'left' });
  }
  return changes.sort((a, b) => a.hour - b.hour || a.tag.localeCompare(b.tag) || a.change.localeCompare(b.change));
}

/** How many countries a top-N group holds unless the user chooses. */
export const DEFAULT_TOP = 10;

/** Shared selection choices for ranked country and faction widgets. */
export function countrySelectionModes(limits: { factions?: boolean; all?: boolean }): { value: string; label: string }[] {
  return [...(limits.all ? [{ value: 'all', label: 'All' }] : []), { value: 'countries', label: 'Countries' }, { value: 'group', label: 'Group' },
    ...(limits.factions ? [{ value: 'factions', label: 'Factions' }] : [])];
}

export function countrySelectionMode(selection: CountrySelection): string {
  return selection.kind === 'all' ? 'all' : selection.kind === 'factions' ? 'factions' : selection.kind === 'group' ? 'group' : 'countries';
}

/** New choices use the same hourly membership rule and top count everywhere. */
export function countrySelectionFor(mode: string, max: number): CountrySelection {
  const count = Math.min(DEFAULT_TOP, max);
  return mode === 'all' ? { kind: 'all' } : mode === 'factions' ? { kind: 'factions', count, membership: 'hour' }
    : mode === 'group' ? { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false } : { kind: 'selected' };
}

/** The `count` countries with the highest values, highest first; a country with
 * no value this hour is left out rather than ranked as zero. */
export function topCountries(values: ReadonlyMap<string, number | null>, count: number): string[] {
  return [...values].filter((entry): entry is [string, number] => entry[1] != null && Number.isFinite(entry[1]))
    .sort((a, b) => b[1] - a[1] || compareTags(a[0], b[0])).slice(0, count).map(([tag]) => tag);
}

/** A group's name for headers and pickers. Faction names are the game's own. */
export function groupLabel(group: CountryGroup, reply: CountryGroupReply | undefined, countryName: (tag: string) => string, hour?: number,
  overrides?: Readonly<Record<string, { name?: string }>>): string {
  switch (group.type) {
    case 'humans': return 'Human players';
    case 'faction': {
      const faction = reply?.factions.find(faction => faction.id === group.id);
      return faction ? factionNameLabel(faction, hour, overrides?.[faction.id]?.name) : 'Unnamed faction';
    }
    case 'war': return `At war with ${countryName(group.tag)}`;
    case 'top': return `Top ${group.count}`;
  }
}

/** Whether `hour` falls in one of the membership runs; always true without them. */
export function isMember(membership: readonly HourSpan[] | undefined, hour: number): boolean {
  return !membership || membership.some(run => run.firstHour <= hour && hour <= run.lastHour);
}

/** Runs cut to the hours a country was a group member, so a line starts when it
 * joins and stops when it leaves (`clipToSpans`). */
export function clipRuns(runs: readonly Run[], membership: readonly MembershipRun[]): Run[] {
  return clipToSpans(runs, membership);
}

/** Why a group's members are unknown under a membership rule (`groupMembers` null). */
function unrecorded(membership: GroupMembership): string {
  switch (membership) {
    case 'hour': return 'Group membership is unavailable at this time.';
    case 'end': return 'Group membership is unavailable at this time.';
    case 'ever': return 'Group membership is incomplete for this campaign.';
  }
}

export interface WorldFilter {
  tags: ReadonlySet<string> | null;
  reason: string | null;
  /** Native identity keys resolved at the hours membership counts. */
  countryIds?: ReadonlySet<string> | null;
}

function identitiesFor(groups: CountryGroupReply | undefined, index: CountryIdentityIndex | null | undefined): CountryIdentityIndex | null {
  return index === undefined ? groups && Array.isArray(groups.countryIdentities) ? countryIdentityIndex(groups.countryIdentities) : null : index;
}

/** The countries a widget about the whole world (`countries.all`) narrows to: null for
 * every country; else the chosen tags (the dashboard's country by default), or the
 * chosen group's members under its membership setting (`groupMembers`): at `hour`,
 * the moment shown; at the branch's `lastHour`; or anyone ever a member. `reason`
 * says why the set is empty or not known: a membership not recorded at the hour it is
 * counted is unknown, never an empty group. Native keys resolve smart members at
 * the counted dates; manual picks include all holders ever carrying their tags. */
export function worldFilter(selection: CountrySelection, fallback: readonly string[], groups: CountryGroupReply | undefined,
  hour: number, lastHour: number, index?: CountryIdentityIndex | null): WorldFilter {
  if (selection.kind === 'all') return { tags: null, reason: null };
  const identities = identitiesFor(groups, index);
  const empty = (reason: string | null): WorldFilter => ({ tags: new Set(), countryIds: new Set(), reason });
  if (selection.kind !== 'group') {
    const tags = selection.kind === 'tags' ? selection.tags : fallback;
    if (!tags.length) return empty('Choose a country in the widget settings.');
    if (!identities) return empty(null);
    const countryIds = identities.selected(tags);
    return { tags: new Set(identities.aliasesAt(countryIds, hour)), countryIds,
      reason: countryIds.size ? null : 'Selected countries are unavailable in this campaign.' };
  }
  if (!groups || !identities) return empty(null);
  const runs = groupRuns(groups, selection.group);
  if (!runs) return empty('This group is unavailable in this campaign.');
  const members = groupMembers(runs, selection.membership, hour, lastHour, Infinity, groupUnobserved(groups, selection.group));
  if (!members) return empty(unrecorded(selection.membership));
  const at = selection.membership === 'hour' ? hour : lastHour;
  const countryIds = selection.membership === 'ever' ? identities.keysDuring(runs)
    : identities.keysDuring(members.map(tag => ({ tag, firstHour: at, lastHour: at })));
  return { tags: new Set(identities.aliasesAt(countryIds, hour)), countryIds, reason: members.length
    ? countryIds.size ? null : 'Group members are unavailable at this time.'
    : selection.membership === 'ever' ? 'This group has no members in this campaign.'
      : selection.membership === 'end' ? 'This group has no members at the end of this campaign.' : 'This group has no members at this moment.' };
}

/** Whether a dated row of a country passes a widget's country choice, for logs, whose
 * rows are each about one country at one hour: every country for "all", the chosen
 * countries, or a group's members under its rule, at the row's own hour, else as
 * `worldFilter` counts them at the branch end or ever. `ready` is false while the
 * group history loads; a row whose group membership at its hour was not recorded
 * does not pass. Manual picks include all native holders of their literal tags;
 * rows resolve at their own hours. Null means identity history is still loading. */
export interface CountryMatch { ready: boolean; matches: (tag: string, hour: number) => boolean; reason: string | null }

export function countryMatch(selection: CountrySelection, fallback: readonly string[], groups: CountryGroupReply | undefined, lastHour: number,
  index?: CountryIdentityIndex | null): CountryMatch {
  if (selection.kind === 'all') return { ready: true, matches: () => true, reason: null };
  const identities = identitiesFor(groups, index);
  if (!identities) return { ready: false, matches: () => false, reason: null };
  if (selection.kind === 'group' && !groups) return { ready: false, matches: () => false, reason: null };
  if (selection.kind === 'group' && selection.membership === 'hour') {
    const runs = groupRuns(groups!, selection.group);
    if (!runs) return { ready: true, matches: () => false, reason: 'This group is unavailable in this campaign.' };
    const unobserved = groupUnobserved(groups!, selection.group);
    const byTag = new Map<string, MembershipRun[]>();
    for (const run of runs) {
      const history = byTag.get(run.tag);
      if (history) history.push(run); else byTag.set(run.tag, [run]);
    }
    const hidden = (hour: number) => unobserved.some(span => span.firstHour <= hour && hour <= span.lastHour);
    return { ready: true, matches: (tag, hour) => identities.keyAt(tag, hour) !== null && !hidden(hour)
      && (byTag.get(tag) ?? []).some(run => run.firstHour <= hour && hour <= run.lastHour), reason: null };
  }
  const { countryIds, reason } = worldFilter(selection, fallback, groups, lastHour, lastHour, identities);
  return { ready: true, matches: (tag, hour) => {
    const key = identities.keyAt(tag, hour);
    return key !== null && countryIds?.has(key) === true;
  }, reason };
}

export interface ResolvedCountries {
  tags: string[];
  /** For a group counted at each hour: each country's membership runs. */
  membership: Map<string, MembershipRun[]> | null;
  /** Hours the group's own history was not recorded: breaks there are not joins or leaves. */
  unobserved: HourSpan[];
  /** Countries left out beyond the widget's limit. */
  dropped: number;
  /** Why no countries are shown, if none are. */
  reason: string | null;
  /** For a top-N group: N, at most the widget's limit. The widget ranks every
   * country itself at each hour (`rankedCandidates`), so `tags` is empty. */
  top?: number;
  /** A summed faction reads the shared server total instead of country lines. */
  factionId?: string;
}

/** A complete world total cannot prove a smart group's missing membership. */
export function groupShareReason(resolved: ResolvedCountries, membership: GroupMembership, hour: number): string | null {
  if (resolved.reason) return resolved.reason;
  if (membership === 'ever' && resolved.unobserved.length) return 'Group membership is incomplete for this campaign.';
  if (membership === 'hour' && resolved.unobserved.some(span => span.firstHour <= hour && hour <= span.lastHour)) return unrecorded(membership);
  return null;
}

/** The countries a ranked widget (a race) may show at `hour`, before it ranks them
 * and keeps its count: every country (`all`) for a top-N group; else the chosen
 * countries, or a group's members, those counted at each hour only while members. */
export function rankedCandidates(resolved: ResolvedCountries, hour: number, all: readonly string[]): string[] {
  if (resolved.top != null) return [...all];
  if (!resolved.membership) return resolved.tags;
  return resolved.tags.filter(tag => isMember(resolved.membership!.get(tag), hour));
}

/** The countries a multi-country widget draws: chosen tags, or a group's members
 * under its rule, at most `max` (the widget's own limit). */
export function resolveCountries(selection: CountrySelection, fallback: string[], groups: CountryGroupReply | undefined, lastHour: number, max: number, factionTotals = false): ResolvedCountries {
  if (selection.kind === 'factions') return { tags: [], membership: null, unobserved: [], dropped: 0, reason: 'This widget does not show faction totals.' };
  if (factionTotals && selection.kind === 'group' && selection.aggregate && selection.group.type === 'faction') {
    return { tags: [], membership: null, unobserved: [], dropped: 0, reason: null, factionId: selection.group.id };
  }
  if (selection.kind !== 'group') {
    const tags = selection.kind === 'tags' ? selection.tags : fallback;
    return { tags: tags.slice(0, max), membership: null, unobserved: [], dropped: Math.max(0, tags.length - max), reason: tags.length ? null : 'Choose a country in the widget settings.' };
  }
  // Top N needs no group history: the widget ranks by its own value.
  if (selection.group.type === 'top') return { tags: [], membership: null, unobserved: [], dropped: 0, reason: null, top: Math.max(1, Math.min(max, selection.group.count)) };
  if (!groups) return { tags: [], membership: null, unobserved: [], dropped: 0, reason: null };
  const runs = groupRuns(groups, selection.group);
  if (!runs) return { tags: [], membership: null, unobserved: [], dropped: 0, reason: 'This group is unavailable in this campaign.' };
  const unobserved = groupUnobserved(groups, selection.group);
  const members = groupMembers(runs, selection.membership === 'end' ? 'end' : 'ever', lastHour, lastHour, Infinity, unobserved);
  // Unknown is not empty: the membership counted was not recorded.
  if (!members) return { tags: [], membership: null, unobserved, dropped: 0, reason: unrecorded(selection.membership === 'end' ? 'end' : 'ever') };
  const tags = members.slice(0, max);
  const membership = selection.membership === 'hour' ? new Map(tags.map(tag => [tag, [] as MembershipRun[]])) : null;
  if (membership) for (const run of runs) membership.get(run.tag)?.push(run);
  return { tags, membership, unobserved, dropped: Math.max(0, members.length - max), reason: members.length ? null : 'This group has no members in this campaign.' };
}

/** Drawing projection only: keep the original resolved tags as backend roots. */
export function projectResolvedCountries(resolved: ResolvedCountries, filter: ReturnType<typeof countryMatchFilter>): ResolvedCountries {
  if (resolved.reason || resolved.top != null || resolved.factionId != null || filter.membership === null) return resolved;
  const membership = new Map<string, MembershipRun[]>();
  const known = filter.unobserved.length ? withoutHours([...filter.membership].sort((a, b) => a.firstHour - b.firstHour),
    [...filter.unobserved].sort((a, b) => a.firstHour - b.firstHour)) : filter.membership;
  for (const run of known) {
    const runs = membership.get(run.tag);
    if (runs) runs.push(run); else membership.set(run.tag, [run]);
  }
  const unobserved = new Map(resolved.unobserved.map(span => [JSON.stringify([span.firstHour, span.lastHour]), span]));
  for (const span of filter.unobserved) unobserved.set(JSON.stringify([span.firstHour, span.lastHour]), span);
  return { ...resolved, tags: [...membership.keys()], membership,
    unobserved: [...unobserved.values()].sort((a, b) => a.firstHour - b.firstHour || a.lastHour - b.lastHour) };
}

/** Sum a resolved group's clipped lines once. Every expected member must have a
 * recorded value, and unknown rosters remain gaps. Events avoid rescanning all
 * members at each change; a member's identity break also breaks the total. */
export function aggregateGroupRuns(lines: readonly { tag: string; runs: readonly Run[] }[], resolved: ResolvedCountries, range: HourSpan): Run[] {
  if (resolved.factionId != null) throw new Error('Summed faction groups must read the faction-totals product.');
  type Change = { expected: number; recorded: number; value: number; unknown: number; breaks: boolean };
  const events = new Map<number, Change>();
  const event = (hour: number) => {
    let change = events.get(hour);
    if (!change) { change = { expected: 0, recorded: 0, value: 0, unknown: 0, breaks: false }; events.set(hour, change); }
    return change;
  };
  event(range.firstHour); event(range.lastHour + 1);
  for (const tag of resolved.tags) {
    for (const span of resolved.membership?.get(tag) ?? [range]) {
      const first = Math.max(range.firstHour, span.firstHour);
      const last = Math.min(range.lastHour, span.lastHour);
      if (first <= last) { event(first).expected++; event(last + 1).expected--; }
    }
  }
  for (const line of lines) for (const run of line.runs) {
    event(run.firstHour).recorded++; event(run.firstHour).value += run.value;
    event(run.firstHour).breaks ||= run.breakBefore === true;
    event(run.lastHour + 1).recorded--; event(run.lastHour + 1).value -= run.value;
  }
  for (const span of resolved.membership ? resolved.unobserved : []) {
    const first = Math.max(range.firstHour, span.firstHour);
    const last = Math.min(range.lastHour, span.lastHour);
    if (first <= last) { event(first).unknown++; event(last + 1).unknown--; }
  }
  let expected = 0, recorded = 0, value = 0, unknown = 0;
  const hours = [...events.keys()].sort((a, b) => a - b);
  const runs: Run[] = [];
  hours.forEach((hour, index) => {
    const change = events.get(hour)!;
    expected += change.expected; recorded += change.recorded; value += change.value; unknown += change.unknown;
    const lastHour = hours[index + 1] - 1;
    if (index + 1 === hours.length || expected === 0 || unknown > 0 || expected !== recorded || !Number.isSafeInteger(value)) return;
    const previous = runs.at(-1);
    if (previous && !change.breaks && previous.lastHour + 1 === hour && previous.value === value) previous.lastHour = lastHour;
    else runs.push({ firstHour: hour, lastHour, value, ...(change.breaks ? { breakBefore: true } : {}) });
  });
  return runs;
}

export interface WorldRequestTags { tags: string[]; ready: boolean; reason: string | null }
/** A whole-branch world's request: [] means all, chosen countries stay chosen,
 * and an hourly group requests its branch-wide union before local filtering.
 * Pending or empty choices never accidentally request the entire world. */
export function worldRequestTags(selection: CountrySelection, fallback: string[], groups: CountryGroupReply | undefined, lastHour: number,
  index?: CountryIdentityIndex | null): WorldRequestTags {
  if (selection.kind === 'all' || (selection.kind === 'group' && selection.group.type === 'top')) return { tags: [], ready: true, reason: null };
  const identities = identitiesFor(groups, index);
  if (!identities || selection.kind === 'group' && !groups) return { tags: [], ready: false, reason: null };
  const resolved = resolveCountries(selection, fallback, groups, lastHour, Number.MAX_SAFE_INTEGER);
  const counted = selection.kind === 'group' && selection.membership === 'hour' ? { ...selection, membership: 'ever' as const } : selection;
  const reason = resolved.reason ?? worldFilter(counted, fallback, groups, lastHour, lastHour, identities).reason;
  // Backends expand these literal roots; an alias is never a new manual pick.
  return { tags: resolved.tags, ready: reason === null && resolved.tags.length > 0, reason };
}

/** Serializable countryMatch predicate. Native alias runs bound every holder. */
export function countryMatchFilter(selection: CountrySelection, fallback: readonly string[], groups: CountryGroupReply | undefined, lastHour: number,
  index?: CountryIdentityIndex | null, firstHour = -2147483648): {
    tags: string[] | null; membership: MembershipRun[] | null; unobserved: HourSpan[];
  } {
  if (selection.kind === 'all') return { tags: null, membership: null, unobserved: [] };
  const identities = identitiesFor(groups, index);
  if (!identities) return { tags: null, membership: [], unobserved: [] };
  if (selection.kind === 'group' && selection.membership === 'hour') {
    return { tags: null, membership: groups ? identities.membershipRuns(groupRuns(groups, selection.group) ?? [], firstHour, lastHour) : [],
      unobserved: groups ? groupUnobserved(groups, selection.group) : [] };
  }
  const { countryIds } = worldFilter(selection, fallback, groups, lastHour, lastHour, identities);
  return { tags: null, membership: identities.aliasRuns(countryIds ?? new Set(), firstHour, lastHour), unobserved: [] };
}
