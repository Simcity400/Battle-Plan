import type { ExperienceRead, ExperienceReply } from '../../generated/dashboard';
import { formatDateTime } from '../../dashboard/calendar.ts';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { readAt, readNote } from '../../dashboard/reads.ts';
import { runAt } from '../../charts/series.ts';
import type { Run } from '../../charts/series';
import { fitTooltip } from '../../charts/tooltip.ts';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { formatAmount, formatSigned, rescale } from '../../charts/valueAxis.ts';

/** These are the engine's fixed experience branches, not content categories. */
export const BRANCHES = ['army', 'navy', 'air'] as const;
export type ExperienceBranch = typeof BRANCHES[number];
export type ExperienceMeasure = 'banked' | 'change';
export const BRANCH_LABELS = { army: 'Army', navy: 'Navy', air: 'Air' } as const;
const RAW = /^-?\d{1,20}$/;
const isRaw = (raw: unknown): raw is string => typeof raw === 'string' && RAW.test(raw);
const isValues = (value: unknown) => typeof value === 'object' && value != null
  && BRANCHES.every(branch => isRaw((value as ExperienceRead)[`${branch}Raw`]));
const CADENCE = 'experience is read once a day.';
const DECIMALS = 2;

const isRead = (value: unknown): value is ExperienceRead => {
  const read = value as ExperienceRead;
  return isValues(read) && Number.isInteger(read.firstHour) && Number.isInteger(read.lastHour) && read.firstHour <= read.lastHour
    && typeof read.startsSegment === 'boolean'
    && (read.change === null ? read.changeUnknown === null || read.changeUnknown === 'noMatchingRead'
      : isValues(read.change) && read.changeUnknown === null);
};

/** Checks exact raw strings, scales, and non-overlapping reads in hour order. */
export function validateExperienceReply(data: unknown): ExperienceReply {
  const reply = data as ExperienceReply;
  const positive = (value: unknown) => Number.isSafeInteger(value) && (value as number) > 0;
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string'
    && positive(reply.scale) && positive(reply.dayHours)
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string')
    && Array.isArray(reply.series) && reply.series.every(series => typeof series === 'object' && series != null && typeof series.tag === 'string'
      && Array.isArray(series.reads) && series.reads.every(isRead)
      && series.reads.every((read, index) => index === 0 || series.reads[index - 1].lastHour < read.firstHour));
  if (!ok) throw new Error('The experience reply is malformed.');
  return reply;
}

/** Exact raw at a read, or null when its change is not known. */
export const experienceRaw = (read: ExperienceRead, branch: ExperienceBranch, measure: ExperienceMeasure): string | null =>
  (measure === 'change' ? read.change : read)?.[`${branch}Raw`] ?? null;

/** Plotting numbers only; unknown changes are left out, never filled or interpolated. */
export function experienceRuns(reads: readonly ExperienceRead[], scale: number, branch: ExperienceBranch, measure: ExperienceMeasure): Run[] {
  return reads.flatMap(read => {
    const raw = experienceRaw(read, branch, measure);
    return raw == null ? [] : [{ firstHour: read.firstHour, lastHour: read.lastHour, value: Number(raw) / scale, breakBefore: read.startsSegment }];
  });
}

/** Rescale the raw once, then use the kit's exact amount formatting. */
export const formatExperience = (raw: string, scale: number, measure: ExperienceMeasure = 'banked') => measure === 'change'
  ? formatSigned(rescale(raw, scale, DECIMALS), DECIMALS, DECIMALS)
  : formatAmount(rescale(raw, scale, DECIMALS), DECIMALS, DECIMALS);

export interface ExperienceTooltipCountry {
  tag: string;
  reads: readonly ExperienceRead[];
  readHours: readonly number[];
  colours: Readonly<Record<ExperienceBranch, string>>;
  /** Whether this country has any reads, even outside the current view/membership. */
  recorded: boolean;
}

/** Every shown branch at the held read, with its dating and any unknown change. */
export function experienceTooltip(reply: Pick<ExperienceReply, 'scale' | 'dayHours'>, countries: readonly ExperienceTooltipCountry[],
  branches: readonly ExperienceBranch[], measure: ExperienceMeasure, hour: number, label: CountryLabel, height: number, extraRows: readonly TooltipRow[] = []): TooltipContent {
  const reads = countries.map(country => runAt(country.reads, hour));
  const dates = new Set(reads.flatMap((read, index) => {
    const date = read ? readAt(countries[index].readHours, hour) : null;
    return date == null ? [] : [date];
  }));
  const date = dates.size === 1 ? [...dates][0] : hour;
  const notes = new Set<string>(['Experience is read once a day.']);
  const rows = countries.flatMap<TooltipRow>((country, index) => {
    const read = reads[index];
    const named = label(country.tag);
    if (!read) return [{ label: named.name, icon: named.flag, value: country.recorded ? 'Not recorded at this time' : 'Not in this recording', tone: 'muted' }];
    const dating = readNote(readAt(country.readHours, hour), hour, CADENCE);
    if (dating) notes.add(countries.length === 1 ? dating : `${named.name}: ${dating}`);
    if (measure === 'change' && read.change == null) notes.add('No read exactly a day earlier in the same unbroken recorded stretch of this country.');
    return branches.map<TooltipRow>((branch, branchIndex) => {
      const raw = experienceRaw(read, branch, measure);
      return { label: countries.length === 1 ? BRANCH_LABELS[branch] : `${named.name}: ${BRANCH_LABELS[branch]}`, icon: named.flag,
        swatch: country.colours[branch], divider: index > 0 && branchIndex === 0,
        value: raw == null ? 'Not known' : formatExperience(raw, reply.scale, measure), tone: raw == null ? 'muted' : undefined };
    });
  });
  if (measure === 'change') notes.add(`Net change since the read ${reply.dayHours} hours earlier, including experience spent and the game’s cap.`);
  return fitTooltip([{ title: formatDateTime(date), subtitle: date < hour ? `Shown at ${formatDateTime(hour)}` : undefined,
    rows: [...rows, ...extraRows], note: [...notes].join(' ') }], height);
}
