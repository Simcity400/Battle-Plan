import { countryKey } from '../../dashboard/countryKey.ts';
import type { ChartSummary } from '../chartSummary';
import type { CountryRaceEntry } from './countryRace';
import { CELL, cellNumber, cellState, cellValue, rowName } from '../../dashboard/follow/message.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import type { CountryGroupReply, FactionGroup, FactionRostersReply, HourSpan, SeriesStateReply } from '../../generated/dashboard';
import type { FactionLines } from '../../dashboard/territoryReplies';
import type { ResolvedCountries } from '../../dashboard/groups';

/** A group shown as one total: its name, its roster at an hour, every member and its hours, the hours not read. */
export interface GroupTotal {
  label: string;
  rosterAt: (hour: number) => FactionRostersReply;
  members: readonly { tag: string; spans: readonly (readonly [number, number])[] | null }[];
  unread: readonly (readonly [number, number])[];
}

/** The line id of a group's members summed (`faction-lines` with members). */
export const GROUP_ID = 'group';

/** Every member of every faction, ascending, once: the tags whose values the race follows. */
export const rosterTags = (rosters: FactionRostersReply | undefined) =>
  [...new Set(rosters?.factions.flatMap(faction => faction.members) ?? [])].sort();

/** Which factions and members there are: what renders the race (never a number). */
export const rosterRows = (rosters: FactionRostersReply) =>
  rosters.factions.map(faction => `${faction.id}:${faction.members.join(',')}:${faction.missing ?? ''}`).join('|');

/** Each tag's value in a followed answer of members' states (rows named by tag, or the tags asked in order); a tag
 * without a value is absent. */
export function answerValues(answer: FollowedAnswer | undefined, asked: readonly string[], decimals: number): Map<string, number> {
  const out = new Map<string, number>();
  if (!answer || answer.kind === 'notRecorded') return out;
  const divisor = 10 ** decimals;
  for (let row = 0; row < answer.rows; row++) {
    const at = row * answer.columns;
    if (cellState(answer, at) !== CELL.value) continue;
    const tag = rowName(answer, row) ?? asked[row];
    if (tag) out.set(tag, (cellNumber(answer, at) ?? Number(cellValue(answer, at))) / divisor);
  }
  return out;
}

/** Whether the race paints its members' followed values: once the service follows them and their first answer has
 * landed, as a pushed panel (`headSource`). Until then (an answer waits for the head to take its step) the members'
 * states are asked for the hour, so a paused seek shows its totals, never an empty race. */
export const membersPushed = (status: 'off' | 'pending' | 'followed' | 'unavailable', answer: FollowedAnswer | undefined) =>
  status === 'followed' && answer !== undefined;

/** What renders the race for its members' follow: whether its first answer has landed, and a refusal's reason. */
export const answerShape = (answer: FollowedAnswer | undefined) => answer === undefined ? '' : answer.kind === 'notRecorded' ? `refused:${answer.reason}` : 'rows';

/** Each tag's value at `hour` in a state reply. */
export function stateValues(reply: SeriesStateReply | undefined, hour: number): Map<string, number> {
  const out = new Map<string, number>();
  if (!reply) return out;
  const divisor = 10 ** reply.decimals;
  for (const state of reply.states) if (state.run && state.hour === hour) out.set(countryKey(state.country), Number(state.run.valueRaw) / divisor);
  return out;
}

/** Each faction's total from its members' values, greatest first (ties by id), the first `slots`; a faction whose
 * roster or any member's value is not known is left out and counted in `unknown`, never shown smaller. */
export function factionEntries(rosters: FactionRostersReply | undefined, values: ReadonlyMap<string, number>, slots: number, hour: number,
  format: (value: number) => string): { entries: CountryRaceEntry[]; unknown: number } {
  if (!rosters) return { entries: [], unknown: 0 };
  const known: { id: string; value: number }[] = [];
  let unknown = 0;
  for (const faction of rosters.factions) {
    if (!faction.members.length) continue;
    let sum = 0, whole = faction.missing === null;
    for (const tag of faction.members) {
      const value = values.get(tag);
      if (value === undefined) { whole = false; break; }
      sum += value;
    }
    if (whole) known.push({ id: faction.id, value: sum });
    else unknown++;
  }
  known.sort((a, b) => b.value - a.value || a.id.localeCompare(b.id));
  return { entries: known.slice(0, Math.max(0, slots)).map(({ id, value }) => ({ tag: id, value, label: format(value), stretch: null, hour })), unknown };
}

/** Each faction's line by id; the faction lines a chart draws are the `slots` greatest at the view's end. */
export function factionSummaries(reply: FactionLines | undefined, slots: number): Map<string, ChartSummary> {
  if (!reply) return new Map();
  const end = (summary: ChartSummary) => {
    for (let at = summary.buckets.length - 1; at >= 0; at--) if (summary.buckets[at]) return summary.buckets[at]!.lastValue;
    return Number.NEGATIVE_INFINITY;
  };
  const lines = [...reply.lines].sort((a, b) => end(b.summary) - end(a.summary) || a.id.localeCompare(b.id)).slice(0, Math.max(0, slots));
  return new Map(lines.map(line => [line.id, line.summary]));
}

/** A faction's tooltip note: why its total is not known, or that its least and greatest are bounds. */
export function factionNote(rosters: FactionRostersReply | undefined, id: string, bounds: boolean): string | null {
  const missing = rosters?.factions.find(faction => faction.id === id)?.missing;
  if (missing) return missing;
  return bounds ? 'Over a bin, a faction’s least and greatest are its members’ summed: bounds, not exact.' : null;
}

/** Whose membership a faction sums: at each hour, at the view's last hour, or anyone a member within the view. */
export type RosterMembership = 'hour' | 'end' | 'ever';

/** Rosters fixed over the view `from..to`: the members at its last hour (`end`) or anyone a member within it
 * (`ever`), from the groups' recorded memberships. Where memberships were not read the rosters are not known; a
 * faction whose roster was not read whole then is not known, never smaller. */
export function fixedRosters(groups: CountryGroupReply, membership: Exclude<RosterMembership, 'hour'>, from: number, to: number): FactionRostersReply {
  const [first, last] = membership === 'end' ? [to, to] : [from, to];
  const meets = (spans: readonly { firstHour: number; lastHour: number }[]) => spans.some(span => span.firstHour <= last && first <= span.lastHour);
  if (meets(groups.unobserved.factions)) return { factions: [], missing: 'Faction memberships were not read over these hours.', warnings: [] };
  const factions = groups.factions.filter(faction => faction.firstHour <= last && first <= faction.lastHour).map(faction => ({
    id: faction.id,
    members: [...new Set(faction.members.filter(member => member.firstHour <= last && first <= member.lastHour).map(member => member.tag))].sort(),
    missing: meets(faction.incomplete) ? 'Its roster was not read whole over these hours.' : null,
  }));
  return { factions, missing: null, warnings: [] };
}
/** One summed total of a group's members (`aggregate`, a group that is not a faction): its roster at an hour (the
 * members then, or every member where the group is counted at the end or ever), each member's hours (none: the
 * whole view), and the hours its members were not read, where its total is not known. */
export function groupTotal(resolved: Pick<ResolvedCountries, 'tags' | 'membership' | 'unobserved'>, label: string): GroupTotal {
  const members = resolved.tags.map(tag => ({ tag, spans: resolved.membership ? (resolved.membership.get(tag) ?? []).map(run => [run.firstHour, run.lastHour] as const) : null }));
  const unread = resolved.unobserved.map(span => [span.firstHour, span.lastHour] as const);
  const rosterAt = (hour: number): FactionRostersReply => {
    const missing = unread.some(([first, last]) => first <= hour && hour <= last) ? 'The group’s members were not read at this hour.' : null;
    const tags = members.filter(member => member.spans === null || member.spans.some(([first, last]) => first <= hour && hour <= last)).map(member => member.tag);
    return { factions: [{ id: GROUP_ID, members: tags, missing }], missing: null, warnings: [] };
  };
  return { label, members, unread, rosterAt };
}

/** Each tag ever a member of `faction` and the hours it was one, ascending, joined where they meet. */
export function factionMemberSpans(faction: Pick<FactionGroup, 'members'>): Map<string, HourSpan[]> {
  const out = new Map<string, HourSpan[]>();
  for (const run of [...faction.members].sort((a, b) => a.firstHour - b.firstHour)) {
    const spans = out.get(run.tag) ?? [];
    const last = spans.at(-1);
    if (last && last.lastHour + 1 >= run.firstHour) last.lastHour = Math.max(last.lastHour, run.lastHour);
    else spans.push({ firstHour: run.firstHour, lastHour: run.lastHour });
    out.set(run.tag, spans);
  }
  return out;
}

/** `faction`'s members at `hour`, ascending, once. */
export const factionMembersAt = (faction: Pick<FactionGroup, 'members'>, hour: number) =>
  [...new Set(faction.members.filter(run => run.firstHour <= hour && hour <= run.lastHour).map(run => run.tag))].sort();
