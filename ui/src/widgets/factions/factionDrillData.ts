import type { FactionDetailReply, FactionGoalDefinition, FactionGoalRewardsReply, FactionHeldGoal, FactionResourceDetail } from '../../generated/dashboard';
import type { LiveValues } from '../../charts/live';
import type { TooltipRow } from '../../charts/tooltip';
import { HOUR_EPOCH } from '../../dashboard/calendar.ts';
import { goalProgress as liveProgress, resourceKey } from '../../dashboard/factionDetails.ts';
import type { FactionView } from './factionsData';

const HOURS_PER_YEAR = 365 * 24;
/** The game's own goal terms, in the order its interface lists them; other categories follow. */
const TERMS = ['short_term', 'medium_term', 'long_term'];
const STATUS = ['In progress', 'Completed', 'Cancelled', 'Completing', 'Cancelling'];

/** A reward line as the game writes it: colour markup removed and country markers
 * reduced to their names; a line ending in a colon heads the lines under it. */
export interface RewardLine { text: string; heading: boolean }

// Colour runs open with 0x11 and a colour letter and close with 0x11 '!'; a country
// marker is 0x12 and its three-letter tag, followed by the country's name.
const COLOUR = /\u0011[^\u0011]?/g;
const COUNTRY = /\u0012[^\s,()]{3}/g;
const CONTROL = /[\u0000-\u0009\u000b-\u001f]/g;

/** The game's description of what an effect gives, as lines. A heading with nothing
 * under it (the scope's own effects were not described) is left out. */
export function rewardLines(text: string): RewardLine[] {
  const lines = text.replace(COUNTRY, '').replace(COLOUR, '').replace(CONTROL, '').split('\n')
    .map(line => line.trim());
  const out: RewardLine[] = [];
  for (let index = 0; index < lines.length; index++) {
    const line = lines[index];
    if (!line) continue;
    const heading = line.endsWith(':');
    if (heading) {
      let next = index + 1;
      while (next < lines.length && lines[next].endsWith(':')) next++;
      if (next >= lines.length || !lines[next]) continue;
    }
    out.push({ text: heading ? line.slice(0, -1).replace(/^\((.*)\)$/, '$1') : line, heading });
  }
  return out;
}

/** A goal's completion rewards as its tooltip knows them: described, still being asked, or not to be had. */
export type RewardState = { kind: 'described'; reply: FactionGoalRewardsReply } | { kind: 'loading' } | { kind: 'failed'; reason: string };

/** What completing a goal gives, as tooltip rows under one heading; none where the goal gives nothing described. */
export function rewardRows(state: RewardState, heading: string): TooltipRow[] {
  const head: TooltipRow = { label: heading, strong: true, divider: true };
  if (state.kind === 'loading') return [head, { label: 'Loading rewards…', tone: 'muted' }];
  if (state.kind === 'failed') return [head, { label: `Rewards unavailable: ${state.reason}`, tone: 'muted' }];
  if (state.reply.reason) return [head, { label: `Rewards not described: ${state.reply.reason}`, tone: 'muted' }];
  const lines = state.reply.text ? rewardLines(state.reply.text) : [];
  if (!lines.length) return [];
  return [head, ...lines.map(line => line.heading ? { label: line.text, tone: 'muted' as const } : { label: line.text, indent: true })];
}

export const goalName = (definition: FactionGoalDefinition) => definition.name || 'Goal name not recorded';
export const goalStatus = (status: number) => STATUS[status] ?? 'Status not recorded';

/** A held goal with its definition. */
export interface HeldGoal { goal: FactionHeldGoal; definition: FactionGoalDefinition }

/** The manifest's held entry: the goal its definition names. */
export function manifestGoal(reply: FactionDetailReply): HeldGoal | null {
  const index = reply.manifest;
  const goal = index == null ? undefined : reply.goals.find(entry => entry.definition === index);
  return goal ? { goal, definition: reply.definitions[goal.definition] } : null;
}

/** Progress as the game holds it at the frame, only where it is a share of the goal (0–100%). */
export function goalPercent(values: LiveValues<number>, goal: Pick<FactionHeldGoal, 'ordinal'>): number | null {
  const progress = liveProgress(values, goal.ordinal);
  return progress.kind === 'percent' && progress.percent >= 0 && progress.percent <= 100 ? progress.percent : null;
}

/** The manifest's thresholds on its bar, and the furthest named one it has reached. */
export function manifestMarks(held: HeldGoal): { marks: { at: number; name: string | null }[]; reached: string | null } {
  const bands = held.definition.bands;
  const marks = bands.flatMap(band => band.min != null && band.min > 0 && band.min < 1 ? [{ at: band.min * 100, name: band.name }] : []);
  const reached = [...held.goal.activeSections].sort((a, b) => b - a).map(index => bands[index]?.name).find(name => name) ?? null;
  return { marks, reached };
}

/** Active goals other than the manifest, grouped under their terms. */
export function goalGroups(reply: FactionDetailReply, manifest: HeldGoal | null): { key: string; name: string; goals: HeldGoal[] }[] {
  const groups = new Map<string, { key: string; name: string; goals: HeldGoal[] }>();
  for (const goal of reply.goals) {
    if (goal === manifest?.goal) continue;
    const definition = reply.definitions[goal.definition];
    const key = definition.category ?? '';
    let group = groups.get(key);
    if (!group) groups.set(key, group = { key, name: definition.categoryName ?? (key ? key.replaceAll('_', ' ') : 'Other goals'), goals: [] });
    group.goals.push({ goal, definition });
  }
  const rank = (key: string) => { const index = TERMS.indexOf(key); return index < 0 ? TERMS.length : index; };
  return [...groups.values()].sort((a, b) => rank(a.key) - rank(b.key));
}

/** A pooled resource's amount at the frame; undefined where its cell holds none. */
export const resourceAmount = (values: LiveValues<number>, resource: Pick<FactionResourceDetail, 'ordinal'>) => values.get(resourceKey(resource.ordinal));

/** Each resource's place, largest amount first, an amount not known last; ties by slot. */
export function resourceOrder(resources: readonly FactionResourceDetail[], values: LiveValues<number>): Map<number, number> {
  const amounts = resources.map(resource => ({ ordinal: resource.ordinal, value: resourceAmount(values, resource) }));
  amounts.sort((a, b) => (b.value ?? -Infinity) - (a.value ?? -Infinity) || a.ordinal - b.ordinal);
  return new Map(amounts.map((entry, place) => [entry.ordinal, place]));
}

/** A fixed-point amount the live cell has scaled to units, as the whole number the game shows. */
export const wholeOf = (value: number | undefined) => value === undefined ? null : Math.trunc(value);

/** Countries joining the faction by `at`, grouped by hour: the chart's flags. Each
 * country is placed where its first membership began. */
export function joinEvents(view: FactionView, at: number): { hour: number; members: FactionView['members'] }[] {
  const byHour = new Map<number, FactionView['members']>();
  for (const member of view.members) {
    const first = member.stretches[0]?.firstHour;
    if (first == null || first > at) continue;
    byHour.set(first, [...byHour.get(first) ?? [], member]);
  }
  return [...byHour].sort((a, b) => a[0] - b[0]).map(([hour, members]) => ({ hour, members }));
}

/** The member count from `from` by the steps begun at `at`, clipped to the range [from, to]: the chart draws them to
 * `to` and shows them to the head. */
export function countSteps(view: FactionView, from: number, at: number, to = at): { firstHour: number; lastHour: number; value: number }[] {
  return view.counts.filter(step => step.lastHour >= from && step.firstHour <= at)
    .map(step => ({ firstHour: Math.max(from, step.firstHour), lastHour: Math.min(to, step.lastHour), value: step.value }));
}

/** The first hour of each game year inside [from, to]. */
export function yearStarts(from: number, to: number): { hour: number; year: number }[] {
  const out: { hour: number; year: number }[] = [];
  for (let year = Math.ceil((from - HOUR_EPOCH) / HOURS_PER_YEAR); HOUR_EPOCH + year * HOURS_PER_YEAR <= to; year++) {
    out.push({ hour: HOUR_EPOCH + year * HOURS_PER_YEAR, year });
  }
  return out;
}
