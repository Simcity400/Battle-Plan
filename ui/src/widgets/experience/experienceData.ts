import type { CountryIdentityRun, ExperienceRead, ExperienceReply } from '../../generated/dashboard';
import { formatDateTime } from '../../dashboard/calendar.ts';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { clipToSpans, lineEndHour, runAt } from '../../charts/series.ts';
import type { Run, Span } from '../../charts/series';
import { fitTooltip } from '../../charts/tooltip.ts';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { formatAmount, formatSigned, rescale } from '../../charts/valueAxis.ts';
import { object, opaqueTag, signed32, signed64 } from '../../dashboard/validation.ts';
import { nameReadText, type NameRead } from '../../dashboard/nameReads.ts';
import { countryIdentityKey, validCountryIdentityRuns } from '../../dashboard/countryIdentity.ts';

/** Aliases expand manual picks; group rosters retain their literal dated tags. */
export function experienceSelection(reply: ExperienceReply | undefined, tags: readonly string[], grouped: boolean): ExperienceReply['series'] {
  if (!reply) return [];
  if (!grouped) return reply.series;
  const selected = new Set(tags);
  return reply.series.filter(series => selected.has(series.tag));
}

/** A missing hourly membership is an empty roster, never unbounded coverage. */
export function experienceMembership(membership: ReadonlyMap<string, readonly Span[]> | null, tag: string): readonly Span[] | null {
  return membership === null ? null : membership.get(tag) ?? [];
}

export function experienceHolderCount(countries: readonly { tag: string; identities?: readonly CountryIdentityRun[] }[]): number {
  return new Set(countries.flatMap(country => country.identities?.length
    ? country.identities.map(countryIdentityKey) : [JSON.stringify(['tag', country.tag])])).size;
}

/** These are the engine's fixed experience branches, not content categories. */
export const BRANCHES = ['army', 'navy', 'air'] as const;
export type ExperienceBranch = typeof BRANCHES[number];
export type ExperienceMeasure = 'banked' | 'gain' | 'change';
export const BRANCH_LABELS = { army: 'Army', navy: 'Navy', air: 'Air' } as const;
const differenceRaw = (raw: unknown): raw is string => typeof raw === 'string' && /^-?\d{1,20}$/.test(raw)
  && BigInt(raw) > -(1n << 64n) && BigInt(raw) < (1n << 64n);
const gainRaw = (raw: unknown, dayHours: number): raw is string => typeof raw === 'string' && /^\d{1,39}$/.test(raw)
  && BigInt(raw) <= BigInt(dayHours) * ((1n << 63n) - 1n);
const isValues = (value: unknown, raw: (value: unknown) => boolean) => object(value)
  && BRANCHES.every(branch => raw(value[`${branch}Raw`]));
const DECIMALS = 2;

const isRead = (value: unknown, dayHours: number): value is ExperienceRead => {
  const read = value as ExperienceRead;
  return isValues(read, signed64) && signed32(read.firstHour) && signed32(read.lastHour) && read.firstHour <= read.lastHour
    && signed32(read.readHour) && read.readHour <= read.firstHour
    && typeof read.startsSegment === 'boolean'
    && (read.change === null ? read.changeUnknown === null || read.changeUnknown === 'noMatchingRead'
      : isValues(read.change, differenceRaw) && read.changeUnknown === null)
    && (read.gain === null ? read.gainUnknown === 'noCompleteDay'
      : isValues(read.gain, raw => gainRaw(raw, dayHours)) && read.gainUnknown === null);
};

/** Checks exact raw strings, scales, and non-overlapping reads in hour order. */
export function validateExperienceReply(data: unknown): ExperienceReply {
  const reply = data as ExperienceReply;
  const positive = (value: unknown) => Number.isSafeInteger(value) && (value as number) > 0;
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string'
    && positive(reply.scale) && positive(reply.dayHours)
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string')
    && Array.isArray(reply.series) && reply.series.every(series => object(series) && opaqueTag(series.tag)
      && validCountryIdentityRuns(series.identities)
      && Array.isArray(series.reads) && series.reads.every(read => isRead(read, reply.dayHours))
      && series.reads.every((read, index) => index === 0 || series.reads[index - 1].lastHour < read.firstHour))
    && new Set(reply.series.map(series => series.tag)).size === reply.series.length;
  if (!ok) throw new Error('The experience reply is malformed.');
  return reply;
}

/** Exact raw at a read, or null when its change is not known. */
export const experienceRaw = (read: ExperienceRead, branch: ExperienceBranch, measure: ExperienceMeasure): string | null =>
  (measure === 'change' ? read.change : measure === 'gain' ? read.gain : read)?.[`${branch}Raw`] ?? null;

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

export type ExperienceNameRead = NameRead | undefined;
/** Names belong to their dated read; unresolved titles use neutral labels. */
export function experienceName(_tag: string, name: ExperienceNameRead): string {
  return nameReadText(name);
}

/** Date an end label to the actual last drawn value, including an earlier holder
 * when the visible line stops before a rename, identity change or missing read. */
export function experienceLineEnd(tag: string, reads: readonly ExperienceRead[], runs: readonly Run[], branch: ExperienceBranch,
  measure: ExperienceMeasure, from: number, to: number, nameAt: (hour: number, tag: string) => ExperienceNameRead) {
  const hour = lineEndHour(runs, from, to);
  const read = hour == null ? null : runAt(reads, hour);
  const raw = read ? experienceRaw(read, branch, measure) : null;
  return hour == null || raw == null ? null : { tag, hour, raw, name: experienceName(tag, nameAt(hour, tag)) };
}

export interface ExperienceTooltipCountry {
  tag: string;
  reads: readonly ExperienceRead[];
  readHours: readonly number[];
  colours: Readonly<Record<ExperienceBranch, string>>;
  /** Whether this country has any reads, even outside the current view/membership. */
  recorded: boolean;
  identities?: readonly CountryIdentityRun[];
  identityKnown?: boolean;
}

export interface ExperienceHolder<T extends ExperienceTooltipCountry = ExperienceTooltipCountry> {
  id: string;
  parts: T[];
}

/** One native holder can have several tags, while a reused tag has several holders. */
export function experienceHolders<T extends ExperienceTooltipCountry>(countries: readonly T[]): ExperienceHolder<T>[] {
  const holders = new Map<string, ExperienceHolder<T>>();
  const append = (id: string, part: T) => {
    const holder = holders.get(id);
    if (holder) holder.parts.push(part); else holders.set(id, { id, parts: [part] });
  };
  for (const country of countries) {
    if (!country.identities?.length) {
      append(JSON.stringify(['tag', country.tag]), country);
      continue;
    }
    const spans = new Map<string, CountryIdentityRun[]>();
    for (const identity of country.identities) {
      const id = countryIdentityKey(identity);
      const runs = spans.get(id);
      if (runs) runs.push(identity); else spans.set(id, [identity]);
    }
    for (const [id, identities] of spans) append(id, { ...country, identities,
      reads: clipToSpans(country.reads, identities) });
  }
  return [...holders.values()];
}

/** Only the current proven alias receives dated names/flags. Gaps remain one unknown holder. */
export function experienceHolderAt<T extends ExperienceTooltipCountry>(holder: ExperienceHolder<T>, hour: number): T & { identityKnown: boolean } {
  const active = holder.parts.filter(part => part.identities === undefined
    ? runAt(part.reads, hour) !== null : runAt(part.identities, hour) !== null);
  if (active.length === 1) return { ...active[0], identityKnown: true };
  let fallback = holder.parts[0];
  let closest = Number.NEGATIVE_INFINITY;
  for (const part of holder.parts) for (const span of part.identities ?? part.reads) {
    if (span.firstHour <= hour && span.lastHour > closest) { closest = span.lastHour; fallback = part; }
  }
  return { ...fallback, reads: [], recorded: holder.parts.some(part => part.recorded), identityKnown: false };
}

/** The latest drawn alias supplies one end label per holder and branch. */
export function experienceHolderEnd<T extends ExperienceTooltipCountry>(parts: readonly { country: T; runs: readonly Run[] }[],
  branch: ExperienceBranch, measure: ExperienceMeasure, from: number, to: number,
  nameAt: (hour: number, tag: string) => ExperienceNameRead) {
  let chosen: ReturnType<typeof experienceLineEnd> = null;
  let country: T | null = null;
  for (const part of parts) {
    const end = experienceLineEnd(part.country.tag, part.country.reads, part.runs, branch, measure, from, to, nameAt);
    if (end && (!chosen || end.hour > chosen.hour)) { chosen = end; country = part.country; }
  }
  return chosen && country ? { ...chosen, country } : null;
}

/** Every shown branch at the held read, with its dating and any unknown change. */
export function experienceTooltip(reply: Pick<ExperienceReply, 'scale' | 'dayHours'>, countries: readonly ExperienceTooltipCountry[],
  branches: readonly ExperienceBranch[], measure: ExperienceMeasure, hour: number, label: CountryLabel, height: number, extraRows: readonly TooltipRow[] = []): TooltipContent {
  const reads = countries.map(country => runAt(country.reads, hour));
  const notes = new Set<string>();
  const rows = countries.flatMap<TooltipRow>((country, index) => {
    const read = reads[index];
    const named: ReturnType<CountryLabel> = country.identityKnown === false
      ? { name: 'Country name unavailable', known: false, flagPending: false, flagReason: 'The country identity was not recorded at this hour.' }
      : label(country.tag);
    if (!read) return [{ label: named.name, countryFlag: named, value: country.recorded ? 'Not recorded at this time' : 'Not in this recording', tone: 'muted' }];
    if (measure === 'change' && read.change == null) notes.add('A full day of data is unavailable.');
    if (measure === 'gain' && read.gain == null) notes.add('A full day of data is unavailable.');
    return branches.map<TooltipRow>((branch, branchIndex) => {
      const raw = experienceRaw(read, branch, measure);
      return { label: countries.length === 1 ? BRANCH_LABELS[branch] : `${named.name}: ${BRANCH_LABELS[branch]}`, countryFlag: named,
        swatch: country.colours[branch], divider: index > 0 && branchIndex === 0,
        value: raw == null ? measure === 'gain' ? 'Not recorded' : 'Not known' : formatExperience(raw, reply.scale, measure), tone: raw == null ? 'muted' : undefined };
    });
  });
  if (measure === 'change') notes.add('Net change includes spending.');
  if (measure === 'gain') notes.add('Gain before spending.');
  return fitTooltip([{ title: formatDateTime(hour),
    rows: [...rows, ...extraRows], note: [...notes].join(' ') }], height);
}
