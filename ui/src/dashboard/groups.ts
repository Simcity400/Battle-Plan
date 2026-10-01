import type { CountryGroupReply, HourSpan, MembershipRun } from '../generated/dashboard';
import { clipToSpans } from '../charts/series.ts';
import type { Run } from '../charts/series';
import type { CountryGroup, CountrySelection, GroupMembership } from './model';

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
 * recorded at the hour counted, so a missing hour never reads as an empty group. */
export function groupMembers(runs: readonly MembershipRun[], membership: GroupMembership, hour: number, lastHour: number,
  max = Infinity, unobserved: readonly HourSpan[] = []): string[] | null {
  const at = membership === 'hour' ? hour : lastHour;
  if (membership !== 'ever' && unobserved.some(span => span.firstHour <= at && at <= span.lastHour)) return null;
  const ordered = [...runs].sort((a, b) => a.firstHour - b.firstHour || a.tag.localeCompare(b.tag));
  const tags = ordered.filter(run => membership === 'ever' || (run.firstHour <= at && at <= run.lastHour)).map(run => run.tag);
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
export function isMember(membership: readonly MembershipRun[] | undefined, hour: number): boolean {
  return !membership || membership.some(run => run.firstHour <= hour && hour <= run.lastHour);
}

/** Runs cut to the hours a country was a group member, so a line starts when it
 * joins and stops when it leaves (`clipToSpans`). */
export function clipRuns(runs: readonly Run[], membership: readonly MembershipRun[]): Run[] {
  return clipToSpans(runs, membership);
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
}

/** The countries a multi-country widget draws: chosen tags, or a group's members
 * under its rule, at most `max` (the widget's own limit). */
export function resolveCountries(selection: CountrySelection, fallback: string[], groups: CountryGroupReply | undefined, lastHour: number, max: number): ResolvedCountries {
  if (selection.kind !== 'group') {
    const tags = selection.kind === 'tags' ? selection.tags : fallback;
    return { tags: tags.slice(0, max), membership: null, unobserved: [], dropped: Math.max(0, tags.length - max), reason: tags.length ? null : 'Choose a country in the widget settings.' };
  }
  if (!groups) return { tags: [], membership: null, unobserved: [], dropped: 0, reason: null };
  const runs = groupRuns(groups, selection.group);
  if (!runs) return { tags: [], membership: null, unobserved: [], dropped: 0, reason: 'This group is not in this campaign branch.' };
  const unobserved = groupUnobserved(groups, selection.group);
  const members = groupMembers(runs, selection.membership === 'end' ? 'end' : 'ever', lastHour, lastHour, Infinity, unobserved);
  // Unknown is not empty: the group's membership at the branch end was not recorded.
  if (!members) return { tags: [], membership: null, unobserved, dropped: 0, reason: 'This group’s members were not recorded at the end of this campaign branch.' };
  const tags = members.slice(0, max);
  const membership = selection.membership === 'hour'
    ? new Map(tags.map(tag => [tag, runs.filter(run => run.tag === tag)])) : null;
  return { tags, membership, unobserved, dropped: Math.max(0, members.length - max), reason: members.length ? null : 'This group has no members in this campaign branch.' };
}
