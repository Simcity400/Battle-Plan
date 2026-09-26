import type { BalanceDrop, CountryGroupReply, FixedRun, HourSpan, MembershipRun, PoliticalPowerReply, PoliticalPowerSeries } from '../../generated/dashboard';
import type { Run } from '../../charts/series';
import { fixedNumber, formatFixed } from '../../charts/valueAxis.ts';
import { groupMembers, groupRuns, groupUnobserved } from '../../dashboard/groups.ts';
import type { CountrySelection } from '../../dashboard/model';

/** Most countries one Political power widget draws. */
export const MAX_COUNTRIES = 16;
const RAW = /^-?\d{1,20}$/;

const isRun = (value: unknown): value is FixedRun => typeof value === 'object' && value != null
  && Number.isInteger((value as FixedRun).firstHour) && Number.isInteger((value as FixedRun).lastHour)
  && typeof (value as FixedRun).startsSegment === 'boolean'
  && (value as FixedRun).firstHour <= (value as FixedRun).lastHour && RAW.test((value as FixedRun).valueRaw);
const isDrop = (value: unknown): value is BalanceDrop => typeof value === 'object' && value != null
  && Number.isInteger((value as BalanceDrop).hour) && RAW.test((value as BalanceDrop).beforeRaw) && RAW.test((value as BalanceDrop).afterRaw);

/** Checks the reply's shape; runs must be in hour order and never overlap. */
export function validatePoliticalPowerReply(data: unknown): PoliticalPowerReply {
  const reply = data as PoliticalPowerReply;
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string'
    && Number.isInteger(reply.decimals) && reply.decimals >= 0 && reply.decimals <= 12
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string')
    && Array.isArray(reply.series) && reply.series.every(series => typeof series.tag === 'string'
      && Array.isArray(series.runs) && series.runs.every(isRun)
      && series.runs.every((run, index) => index === 0 || series.runs[index - 1].lastHour < run.firstHour)
      && Array.isArray(series.drops) && series.drops.every(isDrop));
  if (!ok) throw new Error('The political power reply is malformed.');
  return reply;
}

/** Runs as plotting numbers; labels use the exact raw values. */
export function toRuns(series: PoliticalPowerSeries, decimals: number): Run[] {
  return series.runs.map(run => ({ firstHour: run.firstHour, lastHour: run.lastHour, value: fixedNumber(run.valueRaw, decimals), breakBefore: run.startsSegment }));
}

/** Whether `hour` falls in one of the membership runs; always true without them. */
export function isMember(membership: readonly MembershipRun[] | undefined, hour: number): boolean {
  return !membership || membership.some(run => run.firstHour <= hour && hour <= run.lastHour);
}

/** The exact recorded balance at an hour, or null when it was not recorded. */
export function rawAt(series: PoliticalPowerSeries, hour: number): string | null {
  let low = 0;
  let high = series.runs.length - 1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    const run = series.runs[middle];
    if (hour < run.firstHour) high = middle - 1;
    else if (hour > run.lastHour) low = middle + 1;
    else return run.valueRaw;
  }
  return null;
}

export const formatBalance = (raw: string, decimals: number) => formatFixed(raw, decimals, 2);

/** A drop's net change, negative, formatted exactly. */
export function netSpend(drop: BalanceDrop, decimals: number): string {
  return formatFixed(BigInt(drop.afterRaw) - BigInt(drop.beforeRaw), decimals, 2);
}

/** The drop nearest `hour` within `tolerance` hours, for snapping hover to a tick. */
export function nearestDrop(drops: readonly BalanceDrop[], hour: number, tolerance: number): BalanceDrop | null {
  let best: BalanceDrop | null = null;
  for (const drop of drops) {
    const distance = Math.abs(drop.hour - hour);
    if (distance <= tolerance && (!best || distance < Math.abs(best.hour - hour))) best = drop;
  }
  return best;
}

/** Runs cut to the hours a country was a group member, so a line starts when it
 * joins and stops when it leaves. */
export function clipRuns(runs: readonly Run[], membership: readonly MembershipRun[]): Run[] {
  const clipped: Run[] = [];
  for (const run of runs) {
    for (const member of membership) {
      const first = Math.max(run.firstHour, member.firstHour);
      const last = Math.min(run.lastHour, member.lastHour);
      if (first <= last) clipped.push({ firstHour: first, lastHour: last, value: run.value });
    }
  }
  return clipped.sort((a, b) => a.firstHour - b.firstHour);
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

/** The countries to draw: chosen tags, or a group's members under its rule. */
export function resolveCountries(selection: CountrySelection, fallback: string[], groups: CountryGroupReply | undefined, lastHour: number): ResolvedCountries {
  if (selection.kind !== 'group') {
    const tags = selection.kind === 'tags' ? selection.tags : fallback;
    return { tags: tags.slice(0, MAX_COUNTRIES), membership: null, unobserved: [], dropped: Math.max(0, tags.length - MAX_COUNTRIES), reason: tags.length ? null : 'Choose a country in the widget settings.' };
  }
  if (!groups) return { tags: [], membership: null, unobserved: [], dropped: 0, reason: null };
  const runs = groupRuns(groups, selection.group);
  if (!runs) return { tags: [], membership: null, unobserved: [], dropped: 0, reason: 'This group is not in this campaign branch.' };
  const unobserved = groupUnobserved(groups, selection.group);
  const members = groupMembers(runs, selection.membership === 'end' ? 'end' : 'ever', lastHour, lastHour, Infinity, unobserved);
  // Unknown is not empty: the group's membership at the branch end was not recorded.
  if (!members) return { tags: [], membership: null, unobserved, dropped: 0, reason: 'This group’s members were not recorded at the end of this campaign branch.' };
  const tags = members.slice(0, MAX_COUNTRIES);
  const membership = selection.membership === 'hour'
    ? new Map(tags.map(tag => [tag, runs.filter(run => run.tag === tag)])) : null;
  return { tags, membership, unobserved, dropped: Math.max(0, members.length - MAX_COUNTRIES), reason: members.length ? null : 'This group has no members in this campaign branch.' };
}
