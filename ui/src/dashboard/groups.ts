import type { CountryGroupReply, HourSpan, MembershipRun } from '../generated/dashboard';
import { clipToSpans } from '../charts/series.ts';
import { compareTags } from './tagOrder.ts';
import type { Run } from '../charts/series';
import type { CountryGroup, CountrySelection, GroupMembership } from './model';
import type { Lineage } from './identity';

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

/** The `count` countries with the highest values, highest first; a country with
 * no value this hour is left out rather than ranked as zero. */
export function topCountries(values: ReadonlyMap<string, number | null>, count: number): string[] {
  return [...values].filter((entry): entry is [string, number] => entry[1] != null && Number.isFinite(entry[1]))
    .sort((a, b) => b[1] - a[1] || compareTags(a[0], b[0])).slice(0, count).map(([tag]) => tag);
}

/** A group's name for headers and pickers. Faction names are the game's own. */
export function groupLabel(group: CountryGroup, reply: CountryGroupReply | undefined, countryName: (tag: string) => string): string {
  switch (group.type) {
    case 'humans': return 'Human players';
    case 'faction': return reply?.factions.find(faction => faction.id === group.id)?.name ?? 'Unnamed faction';
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
    case 'hour': return 'This group’s members were not recorded at this moment.';
    case 'end': return 'This group’s members were not recorded at the end of this campaign branch.';
    case 'ever': return 'No member of this group was recorded, and its membership was not recorded for part of this campaign branch.';
  }
}

export interface WorldFilter {
  tags: ReadonlySet<string> | null;
  reason: string | null;
  /** When a lineage is supplied: countries resolved at the hours membership counts. */
  countryIds?: ReadonlySet<string> | null;
}

/** The countries a widget about the whole world (`countries.all`) narrows to: null for
 * every country; else the chosen tags (the dashboard's country by default), or the
 * chosen group's members under its membership setting (`groupMembers`): at `hour`,
 * the moment shown; at the branch's `lastHour`; or anyone ever a member. `reason`
 * says why the set is empty or not known: a membership not recorded at the hour it is
 * counted is unknown, never an empty group. With `lineage`, `countryIds` resolves
 * each member at the hour counted, or through its own runs for anyone ever. */
export function worldFilter(selection: CountrySelection, fallback: readonly string[], groups: CountryGroupReply | undefined,
  hour: number, lastHour: number, lineage?: Lineage): WorldFilter {
  const result = (tags: ReadonlySet<string> | null, reason: string | null, at = hour, runs?: readonly MembershipRun[]): WorldFilter => {
    if (!lineage) return { tags, reason };
    const countryIds = tags && new Set<string>();
    if (countryIds) {
      if (runs) for (const run of runs) {
        countryIds.add(lineage.countryAt(run.tag, run.firstHour));
        for (const changed of lineage.changeHours(run.tag, run.firstHour + 1, run.lastHour)) countryIds.add(lineage.countryAt(run.tag, changed));
      }
      else for (const tag of tags!) countryIds.add(lineage.countryAt(tag, at));
    }
    return { tags, reason, countryIds };
  };
  if (selection.kind === 'all') return result(null, null);
  if (selection.kind !== 'group') {
    const tags = selection.kind === 'tags' ? selection.tags : fallback;
    return result(new Set(tags), tags.length ? null : 'Choose a country in the widget settings.');
  }
  if (!groups) return result(new Set(), null);
  const runs = groupRuns(groups, selection.group);
  if (!runs) return result(new Set(), 'This group is not in this campaign branch.');
  const members = groupMembers(runs, selection.membership, hour, lastHour, Infinity, groupUnobserved(groups, selection.group));
  if (!members) return result(new Set(), unrecorded(selection.membership));
  return result(new Set(members), members.length ? null
    : selection.membership === 'ever' ? 'This group has no members in this campaign branch.'
      : selection.membership === 'end' ? 'This group has no members at the end of this campaign branch.' : 'This group has no members at this moment.',
  selection.membership === 'hour' ? hour : lastHour, selection.membership === 'ever' ? runs : undefined);
}

/** Whether a dated row of a country passes a widget's country choice, for logs, whose
 * rows are each about one country at one hour: every country for "all", the chosen
 * countries, or a group's members under its rule, at the row's own hour, else as
 * `worldFilter` counts them at the branch end or ever. `ready` is false while the
 * group history loads; a row whose group membership at its hour was not recorded
 * does not pass. */
export interface CountryMatch { ready: boolean; matches: (tag: string, hour: number) => boolean; reason: string | null }

export function countryMatch(selection: CountrySelection, fallback: readonly string[], groups: CountryGroupReply | undefined, lastHour: number): CountryMatch {
  if (selection.kind === 'group' && !groups) return { ready: false, matches: () => false, reason: null };
  if (selection.kind === 'group' && selection.membership === 'hour') {
    const runs = groupRuns(groups!, selection.group);
    if (!runs) return { ready: true, matches: () => false, reason: 'This group is not in this campaign branch.' };
    const unobserved = groupUnobserved(groups!, selection.group);
    const byTag = new Map<string, MembershipRun[]>();
    for (const run of runs) byTag.set(run.tag, [...(byTag.get(run.tag) ?? []), run]);
    const hidden = (hour: number) => unobserved.some(span => span.firstHour <= hour && hour <= span.lastHour);
    return { ready: true, matches: (tag, hour) => !hidden(hour) && (byTag.get(tag) ?? []).some(run => run.firstHour <= hour && hour <= run.lastHour), reason: null };
  }
  const { tags, reason } = worldFilter(selection, fallback, groups, lastHour, lastHour);
  return { ready: true, matches: tag => tags === null || tags.has(tag), reason };
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
export function resolveCountries(selection: CountrySelection, fallback: string[], groups: CountryGroupReply | undefined, lastHour: number, max: number): ResolvedCountries {
  if (selection.kind !== 'group') {
    const tags = selection.kind === 'tags' ? selection.tags : fallback;
    return { tags: tags.slice(0, max), membership: null, unobserved: [], dropped: Math.max(0, tags.length - max), reason: tags.length ? null : 'Choose a country in the widget settings.' };
  }
  // Top N needs no group history: the widget ranks by its own value.
  if (selection.group.type === 'top') return { tags: [], membership: null, unobserved: [], dropped: 0, reason: null, top: Math.max(1, Math.min(max, selection.group.count)) };
  if (!groups) return { tags: [], membership: null, unobserved: [], dropped: 0, reason: null };
  const runs = groupRuns(groups, selection.group);
  if (!runs) return { tags: [], membership: null, unobserved: [], dropped: 0, reason: 'This group is not in this campaign branch.' };
  const unobserved = groupUnobserved(groups, selection.group);
  const members = groupMembers(runs, selection.membership === 'end' ? 'end' : 'ever', lastHour, lastHour, Infinity, unobserved);
  // Unknown is not empty: the membership counted was not recorded.
  if (!members) return { tags: [], membership: null, unobserved, dropped: 0, reason: unrecorded(selection.membership === 'end' ? 'end' : 'ever') };
  const tags = members.slice(0, max);
  const membership = selection.membership === 'hour'
    ? new Map(tags.map(tag => [tag, runs.filter(run => run.tag === tag)])) : null;
  return { tags, membership, unobserved, dropped: Math.max(0, members.length - max), reason: members.length ? null : 'This group has no members in this campaign branch.' };
}

export interface WorldRequestTags { tags: string[]; ready: boolean; reason: string | null }
/** A whole-branch world's request: [] means all, chosen countries stay chosen,
 * and an hourly group requests its branch-wide union before local filtering.
 * Pending or empty choices never accidentally request the entire world. */
export function worldRequestTags(selection: CountrySelection, fallback: string[], groups: CountryGroupReply | undefined, lastHour: number): WorldRequestTags {
  if (selection.kind === 'all' || (selection.kind === 'group' && selection.group.type === 'top')) return { tags: [], ready: true, reason: null };
  if (selection.kind === 'group' && !groups) return { tags: [], ready: false, reason: null };
  const resolved = resolveCountries(selection, fallback, groups, lastHour, Number.MAX_SAFE_INTEGER);
  return { tags: resolved.tags, ready: resolved.reason === null && resolved.tags.length > 0, reason: resolved.reason };
}
