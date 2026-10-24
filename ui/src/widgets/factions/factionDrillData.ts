import type { FactionDetailReply, FactionGoalDefinition, FactionGoalDetail, FactionResourceDetail } from '../../generated/dashboard';
import type { TooltipRow } from '../../charts/tooltip';
import { HOUR_EPOCH } from '../../dashboard/calendar.ts';
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

/** What completing a goal gives, as tooltip rows under one heading. */
export function rewardRows(definition: FactionGoalDefinition, heading: string): TooltipRow[] {
  const lines = definition.rewards.filter(reward => reward.kind === 'complete_effect').flatMap(reward => rewardLines(reward.text));
  if (!lines.length) return [];
  return [{ label: heading, strong: true, divider: true },
    ...lines.map(line => line.heading ? { label: line.text, tone: 'muted' as const } : { label: line.text, indent: true })];
}

export const goalName = (definition: FactionGoalDefinition) => definition.name || 'Goal name not recorded';
export const goalStatus = (status: number) => STATUS[status] ?? 'Status not recorded';

/** The manifest's held entry: the goal its definition names. */
export function manifestGoal(reply: FactionDetailReply): FactionGoalDetail | null {
  const token = reply.manifest?.token;
  return token == null ? null : reply.goals.find(goal => goal.definition.token === token) ?? null;
}

/** Progress as the game holds it, only where it is a share of the goal (0–100%). */
export function goalProgress(goal: FactionGoalDetail): number | null {
  const value = goal.progressPercent;
  return value != null && value >= 0 && value <= 100 ? value : null;
}

/** The manifest's thresholds on its bar, and the furthest named one it has reached. */
export function manifestMarks(goal: FactionGoalDetail): { marks: { at: number; name: string | null }[]; reached: string | null } {
  const bands = goal.definition.bands;
  const marks = bands.flatMap(band => band.min != null && band.min > 0 && band.min < 1 ? [{ at: band.min * 100, name: band.name }] : []);
  const reached = [...goal.activeSections].sort((a, b) => b - a).map(index => bands[index]?.name).find(name => name) ?? null;
  return { marks, reached };
}

/** Active goals other than the manifest, grouped under their terms. */
export function goalGroups(reply: FactionDetailReply, manifest: FactionGoalDetail | null): { key: string; name: string; goals: FactionGoalDetail[] }[] {
  const groups = new Map<string, { key: string; name: string; goals: FactionGoalDetail[] }>();
  for (const goal of reply.goals) {
    if (goal === manifest) continue;
    const key = goal.definition.category ?? '';
    let group = groups.get(key);
    if (!group) groups.set(key, group = { key, name: goal.definition.categoryName ?? (key ? key.replaceAll('_', ' ') : 'Other goals'), goals: [] });
    group.goals.push(goal);
  }
  const rank = (key: string) => { const index = TERMS.indexOf(key); return index < 0 ? TERMS.length : index; };
  return [...groups.values()].sort((a, b) => rank(a.key) - rank(b.key));
}

/** Resources the faction extracts, largest first; the engine's empty slot is not one. */
export function extracted(resources: readonly FactionResourceDetail[] | null): { resource: FactionResourceDetail; value: number }[] {
  return (resources ?? []).filter(resource => resource.token != null)
    .map(resource => ({ resource, value: Number(BigInt(resource.raw)) / 100_000 }))
    .sort((a, b) => b.value - a.value);
}

/** A fixed-point (five places) amount as a whole number. */
export const fixedWhole = (raw: string | null) => raw == null ? null : Number(BigInt(raw) / 100_000n);

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

/** The member count over [from, at], as steps clipped to the range. */
export function countSteps(view: FactionView, from: number, at: number): { firstHour: number; lastHour: number; value: number }[] {
  return view.counts.filter(step => step.lastHour >= from && step.firstHour <= at)
    .map(step => ({ firstHour: Math.max(from, step.firstHour), lastHour: Math.min(at, step.lastHour), value: step.value }));
}

/** The first hour of each game year inside [from, to]. */
export function yearStarts(from: number, to: number): { hour: number; year: number }[] {
  const out: { hour: number; year: number }[] = [];
  for (let year = Math.ceil((from - HOUR_EPOCH) / HOURS_PER_YEAR); HOUR_EPOCH + year * HOURS_PER_YEAR <= to; year++) {
    out.push({ hour: HOUR_EPOCH + year * HOURS_PER_YEAR, year });
  }
  return out;
}
