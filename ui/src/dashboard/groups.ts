import type { CountryGroupReply, HourSpan, MembershipRun } from '../generated/dashboard';
import type { CountryGroup, GroupMembership } from './model';

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
    case 'faction': return reply.unobserved.factions;
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

/** The `count` countries with the highest values, highest first; a country with
 * no value this hour is left out rather than ranked as zero. */
export function topCountries(values: ReadonlyMap<string, number | null>, count: number): string[] {
  return [...values].filter((entry): entry is [string, number] => entry[1] != null && Number.isFinite(entry[1]))
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).slice(0, count).map(([tag]) => tag);
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
