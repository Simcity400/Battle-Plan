import type { CountryIdentityRun, HourSpan, ProjectPresentation, ProjectRecord, ProjectRuntimeDetails, ProjectStatus, SpecialProjectDetails,
  SpecialProjectsReply } from '../../generated/dashboard';
import { runAt, spansCovered } from '../../charts/series.ts';
import { dayLabel } from '../../charts/draw.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { usNumber } from '../../charts/numberFormat.ts';
import { validCountryIdentityRuns } from '../../dashboard/countryIdentity.ts';

const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
const integer = (value: unknown): value is number => typeof value === 'number' && Number.isSafeInteger(value);
const nullableString = (value: unknown) => value === null || typeof value === 'string';
const nullableInteger = (value: unknown) => value === null || integer(value);
const span = (value: unknown) => object(value) && integer(value.firstHour) && integer(value.lastHour) && value.firstHour <= value.lastHour;
const STATES = ['running', 'paused', 'cancelled', 'completed', 'unknown'];

export function validateSpecialProjectsReply(input: unknown): SpecialProjectsReply {
  const bad = () => { throw new Error('Invalid special-projects reply'); };
  if (!object(input) || typeof input.branchId !== 'string' || typeof input.tag !== 'string' || !nullableString(input.unavailable)
    || !Array.isArray(input.identities) || !validCountryIdentityRuns(input.identities as CountryIdentityRun[])
    || !Array.isArray(input.available) || !input.available.every(span) || !Array.isArray(input.partial) || !input.partial.every(span)
    || !Array.isArray(input.unresolved) || !input.unresolved.every(span)
    || !object(input.iterationTexts) || !Object.entries(input.iterationTexts).every(([count, text]) => /^-?(0|[1-9][0-9]*)$/.test(count) && integer(Number(count)) && nullableString(text))
    || !Array.isArray(input.projects) || !Array.isArray(input.presentations) || !Array.isArray(input.people) || !Array.isArray(input.reads)
    || !Array.isArray(input.warnings) || !input.warnings.every(item => typeof item === 'string')) return bad();
  const ordered = (values: readonly { firstHour: number; lastHour: number }[]) => values.every((value, i) => !i || values[i - 1].lastHour < value.firstHour);
  if (!ordered(input.available as HourSpan[]) || !spansCovered(input.available as HourSpan[], input.identities as CountryIdentityRun[]) || !ordered(input.unresolved as HourSpan[])
    || !ordered(input.partial as HourSpan[]) || !spansCovered(input.partial as HourSpan[], input.identities as CountryIdentityRun[])
    || (input.partial as HourSpan[]).some(span => runAt(input.available as HourSpan[], span.firstHour) !== null || runAt(input.available as HourSpan[], span.lastHour) !== null)) return bad();
  for (const person of input.people) if (!object(person) || !(person.id === null || integer(person.id) && person.id >= 0)
    || !(person.assigned === null || typeof person.assigned === 'boolean') || !nullableString(person.name) || !nullableString(person.portrait) || !nullableString(person.missing)) return bad();
  const people = (input.people as unknown[]).length;
  for (const info of input.presentations) if (!object(info) || typeof info.name !== 'string'
    || !nullableString(info.specialization) || !nullableString(info.specializationName) || !nullableString(info.icon) || !nullableString(info.iconMissing)
    || !integer(info.scientist) || info.scientist < 0 || info.scientist >= people
    || !(info.lastScientist === null || integer(info.lastScientist) && info.lastScientist >= 0 && info.lastScientist < people)) return bad();
  const presentation = (value: unknown) => integer(value) && value >= 0 && value < (input.presentations as unknown[]).length;
  for (const [at, read] of (input.reads as unknown[]).entries()) {
    const before = at ? (input.reads as Record<string, number>[])[at - 1] : null;
    if (!span(read) || !object(read) || !integer(read.owner) || read.owner < 0 || !integer(read.readHour) || read.readHour > (read.firstHour as number)
      || before && (before.owner > read.owner || before.owner === read.owner && before.lastHour >= (read.firstHour as number))) return bad();
  }
  const ids = new Set<string>();
  for (const project of input.projects) {
    if (!object(project) || typeof project.id !== 'string' || ids.has(project.id) || !integer(project.owner) || project.owner < 0 || !integer(project.local) || project.local < 0
      || !integer(project.firstRead) || !STATES.includes(project.firstState as string)
      || !Array.isArray(project.statuses) || !Array.isArray(project.entries) || !Array.isArray(project.attempts)) return bad();
    ids.add(project.id);
    for (const run of project.statuses) if (!span(run) || !object(run) || !presentation(run.presentation) || !integer(run.readHour) || run.readHour > (run.firstHour as number)
      || !integer(run.progress) || run.progress < -100 || run.progress > 100 || !STATES.includes(run.state as string) || run.state === 'completed' && run.progress !== 100
      || !nullableInteger(run.iterations) || run.iterations !== null && !(String(run.iterations) in input.iterationTexts)
      || !nullableInteger(run.scientistRead) || !nullableInteger(run.lastScientistRead)
      || (run.lastScientistRead === null) !== ((input.presentations as ProjectPresentation[])[run.presentation as number].lastScientist === null)
      || (run.firstHour as number) < (project.firstRead as number)) return bad();
    if (!ordered(project.statuses as ProjectStatus[]) || !spansCovered(project.statuses as ProjectStatus[], input.available as HourSpan[])) return bad();
    for (const entry of project.entries) if (!object(entry) || !integer(entry.ordinal) || !integer(entry.date) || typeof entry.completion !== 'boolean' || !integer(entry.readHour)
      || typeof entry.counted !== 'boolean' || entry.counted && (!entry.completion || entry.date > entry.readHour)
      || !(entry.presentation === null || entry.counted && presentation(entry.presentation))) return bad();
    for (const attempt of project.attempts) if (!object(attempt) || !integer(attempt.ordinal) || !integer(attempt.date) || !integer(attempt.readHour) || !nullableInteger(attempt.sinceRead)) return bad();
  }
  const shown = input.shown;
  const index = (value: unknown) => integer(value) && value >= 0 && value < (input.projects as unknown[]).length;
  if (!object(shown) || !nullableInteger(shown.through) || !Array.isArray(shown.running) || !shown.running.every(index)
    || !Array.isArray(shown.unknown) || !shown.unknown.every(index)
    || !Array.isArray(shown.completed) || !shown.completed.every(item => object(item) && index(item.project) && nullableInteger(item.date))
    || !integer(shown.undated) || shown.undated < 0 || shown.undated > shown.completed.length) return bad();
  return input as unknown as SpecialProjectsReply;
}

export const statusRead = (data: SpecialProjectsReply, project: ProjectRecord, hour: number): number | null =>
  data.reads.find(read => read.owner === project.owner && read.firstHour <= hour && hour <= read.lastHour)?.readHour ?? null;
export const readLabel = (read: number | null, hour: number) => read !== null && read < hour ? `read ${formatDateTime(read)}` : null;
export const unobservedReason = (data: SpecialProjectsReply, hour: number) =>
  runAt(data.partial, hour) !== null ? 'Projects were read only in part at this time.' : 'Projects were not recorded at this time.';

export const scientistName = (data: SpecialProjectsReply, info: ProjectPresentation) => {
  const scientist = data.people[info.scientist];
  return scientist.name ?? (scientist.assigned === false ? 'No scientist assigned' : 'Scientist not recorded');
};
export const lastScientist = (data: SpecialProjectsReply, info: ProjectPresentation, run: ProjectStatus) =>
  info.lastScientist === null || run.lastScientistRead === null ? null : `${data.people[info.lastScientist].name ?? 'Name unavailable'} (read ${formatDateTime(run.lastScientistRead)})`;
export const iterationCount = (iterations: number | null) =>
  iterations === null ? 'Iterations not recorded' : `${usNumber.format(iterations)} ${iterations === 1 ? 'iteration' : 'iterations'}`;
export interface ProjectCard { project: ProjectRecord; run: ProjectStatus; info: ProjectPresentation }

export function shownCard(data: SpecialProjectsReply, hour: number, id: string): ProjectCard | null {
  const project = data.projects.find(item => item.id === id);
  const run = project ? runAt(project.statuses, hour) : null;
  if (!project || !run || run.state === 'cancelled') return null;
  return { project, run, info: data.presentations[run.presentation]! };
}

interface EpisodeBound { through: number; open: boolean }

export function episodeBound(reply: SpecialProjectsReply, project: ProjectRecord, through: number | null): EpisodeBound {
  const last = project.statuses[project.statuses.length - 1].lastHour;
  const open = last === reply.available.at(-1)?.lastHour;
  return { through: open && through !== null && through > last ? through : last, open };
}

const ended = (card: ProjectCard): EpisodeBound => ({ through: card.project.statuses[card.project.statuses.length - 1].lastHour, open: false });
const joined = (details: SpecialProjectDetails, card: ProjectCard) =>
  details.projects?.find(project => project.owner === card.project.owner && project.id === card.project.local);
const listStale = (found: ProjectRuntimeDetails | undefined, bound: EpisodeBound) => !!found && bound.open && found.readHour > bound.through;
const inStatuses = (project: ProjectRecord, bound: EpisodeBound, read: number) => runAt(project.statuses, read) !== null
  || project.statuses.some(run => run.readHour === read)
  || bound.open && read > project.statuses[project.statuses.length - 1].lastHour && read <= bound.through;

export interface RuntimeMatch { project: ProjectRuntimeDetails; choices: ProjectRuntimeDetails['choices'] | string }

function recordedChoices(project: ProjectRecord, bound: EpisodeBound, found: ProjectRuntimeDetails): RuntimeMatch['choices'] {
  const read = found.historyReadHour;
  if (found.choices === null || read === null) return null;
  if (read < project.firstRead) return 'Iterations were last read before this project was first recorded.';
  return inStatuses(project, bound, read) ? found.choices : 'Iterations were read while this project was not recorded.';
}

export function runtimeProject(details: SpecialProjectDetails, card: ProjectCard, hour: number, bound: EpisodeBound = ended(card)): RuntimeMatch | string | null {
  if (details.hour > hour) return null;
  const found = joined(details, card);
  if (listStale(found, bound)) return `Project rewards were read ${dayLabel(found!.readHour)}, after the project list is known current (${dayLabel(bound.through)}).`;
  if (!details.projects) return 'Project rewards and output were not read at this time.';
  if (!found) return 'Project rewards and output were not recorded for this project at this time.';
  if (found.readHour < card.project.firstRead) return 'Project rewards and output were last read before this project was first recorded.';
  if (found.readHour > bound.through) return 'Project rewards and output were read after this project was last recorded.';
  if (!inStatuses(card.project, bound, found.readHour)) return 'Project rewards and output were read while this project was not recorded.';
  return { project: found, choices: recordedChoices(card.project, bound, found) };
}
