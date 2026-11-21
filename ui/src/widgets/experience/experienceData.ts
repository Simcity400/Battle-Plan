import type { OptionValue } from '../../dashboard/options';
import type { ChartSummary } from '../../charts/chartSummary';
import { scaleSummary } from '../../charts/chartSummary.ts';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { formatDateTime } from '../../dashboard/calendar.ts';
import type { ExperienceReply, ExperienceValues } from '../../generated/dashboard';
import { rowAmount } from '../../dashboard/follow/painters.ts';
import type { FollowEntry } from '../../dashboard/follow/store';
import { followAtHead } from '../../dashboard/tiles/head.ts';

export type ExperienceMeasure = 'banked' | 'gain' | 'change';
export const BRANCHES = ['army', 'navy', 'air'] as const;
export type ExperienceBranch = typeof BRANCHES[number];
export const BRANCH_LABELS: Readonly<Record<ExperienceBranch, string>> = { army: 'Army', navy: 'Navy', air: 'Air' };
export const MEASURE_LABELS: Readonly<Record<ExperienceMeasure, string>> = { banked: 'Banked', gain: 'Daily gain', change: 'Change per day' };
export const SCALE = 32_768;


export const experienceMeasure = (options: Readonly<Record<string, OptionValue>>): ExperienceMeasure =>
  options.measure === 'gain' || options.measure === 'change' ? options.measure : 'banked';
export const experienceBranches = (options: Readonly<Record<string, OptionValue>>): ExperienceBranch[] => BRANCHES.filter(branch => options[branch] !== false);

export function experienceLines(tags: readonly string[], branches: readonly ExperienceBranch[], single: boolean,
  countries: ReadonlyMap<string, string>, categories: readonly string[]): { tag: string; branch: ExperienceBranch; color: string }[] {
  return tags.flatMap(tag => branches.map(branch => ({ tag, branch,
    color: single ? categories[BRANCHES.indexOf(branch) % categories.length] : countries.get(tag) ?? categories[0] })));
}

export const pointsSummary = (summary: ChartSummary): ChartSummary => scaleSummary(summary, 1 / SCALE);

export function followExperienceHeads(entries: Readonly<Record<ExperienceBranch, Pick<FollowEntry, 'answer'> | null>>, tags: readonly string[],
  headOf: (branch: ExperienceBranch, tag: string) => string): () => void {
  const stops = BRANCHES.flatMap(branch => {
    const entry = entries[branch];
    return entry ? tags.map((tag, row) => followAtHead(headOf(branch, tag), () => rowAmount(entry.answer, row, 0))) : [];
  });
  return () => stops.forEach(stop => stop());
}

export function experienceFamily(measure: ExperienceMeasure, branch: ExperienceBranch): { kind: string; measure: string } {
  if (measure === 'banked') return { kind: 'country-experience', measure: branch };
  return { kind: 'country-experience-daily', measure: `${branch}${measure === 'gain' ? 'Gain' : 'Change'}` };
}

const isText = (value: unknown) => value === null || typeof value === 'string';
const values = (value: unknown) => value === null || (!!value && typeof value === 'object'
  && ['armyRaw', 'navyRaw', 'airRaw'].every(key => typeof (value as Record<string, unknown>)[key] === 'string'));

export function validateExperienceReply(value: unknown): ExperienceReply {
  const reply = value as ExperienceReply;
  if (!reply || typeof reply.tag !== 'string' || reply.scale !== SCALE || !Number.isSafeInteger(reply.dayHours)
    || !(reply.readHour === null || Number.isSafeInteger(reply.readHour)) || !values(reply.banked) || !values(reply.change) || !values(reply.gain)
    || !isText(reply.bankedMissing) || !isText(reply.changeMissing) || !isText(reply.gainMissing)
    || (reply.banked === null) === (reply.bankedMissing === null) || !Array.isArray(reply.warnings)) {
    throw new Error('The experience reply is malformed.');
  }
  return reply;
}

export function formatExperience(raw: string | number, signed = false): string {
  const points = Number(raw) / SCALE;
  const text = points.toLocaleString('en-US', { maximumFractionDigits: 2 });
  return signed && points > 0 ? `+${text}` : text;
}

export function experienceCard(reply: ExperienceReply, branch: ExperienceBranch): TooltipContent {
  const key = `${branch}Raw` as keyof ExperienceValues;
  const row = (label: string, held: ExperienceValues | null, missing: string | null, signed: boolean): TooltipRow =>
    held ? { label, value: formatExperience(held[key], signed) } : { label, value: 'Not recorded', detail: missing ?? undefined, tone: 'muted' };
  return { title: `${BRANCH_LABELS[branch]} experience`, subtitle: reply.readHour === null ? 'Read hour not recorded' : `Read ${formatDateTime(reply.readHour)}`,
    rows: [row('Banked', reply.banked, reply.bankedMissing, false), row('Daily gain', reply.gain, reply.gainMissing, true),
      row('Change per day', reply.change, reply.changeMissing, true)] };
}

export const experienceStructure = (reply: ExperienceReply) => JSON.stringify([reply.readHour, reply.banked, reply.change, reply.gain,
  reply.bankedMissing, reply.changeMissing, reply.gainMissing]);
