import { contentTitle } from '../../dashboard/contentTitles.ts';
import type { CountryGroupReply, CountryIdentityRun, HourSpan, MembershipRun, ProjectEntry, ProjectPresentation, ProjectRecord, ProjectRuntimeDetails, ProjectStatus, SpecialProjectDetails,
  SpecialProjectsReply } from '../../generated/dashboard';
import type { TimelineLane, TimelineSpan } from '../../charts/timeline/layout.ts';
import { withoutHours } from '../../charts/timeline/runState.ts';
import { clipToSpans, runAt, spansCovered } from '../../charts/series.ts';
import { dayLabel } from '../../charts/draw.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { usNumber } from '../../charts/numberFormat.ts';
import type { TooltipContent } from '../../charts/tooltip.ts';
import type { CountryLabelValue } from '../../dashboard/countryLabelData.ts';
import type { CountrySelection } from '../../dashboard/model';
import { validCountryIdentityRuns } from '../../dashboard/countryIdentity.ts';
import { countryMatch, countryMatchFilter, worldRequestTags } from '../../dashboard/groups.ts';

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
    || !Array.isArray(shown.completed) || !shown.completed.every(item => object(item) && index(item.project) && nullableInteger(item.date))
    || !integer(shown.undated) || shown.undated < 0 || shown.undated > shown.completed.length) return bad();
  return input as unknown as SpecialProjectsReply;
}

export const completions = (project: ProjectRecord, hour: number) => project.entries.filter(entry => entry.counted && entry.readHour <= hour);
export const statusRead = (data: SpecialProjectsReply, project: ProjectRecord, hour: number): number | null =>
  data.reads.find(read => read.owner === project.owner && read.firstHour <= hour && hour <= read.lastHour)?.readHour ?? null;
export const readLabel = (read: number | null, hour: number) => read !== null && read < hour ? `read ${formatDateTime(read)}` : null;
export const observedAt = (data: SpecialProjectsReply, hour: number) => runAt(data.available, hour) !== null;
export const unobservedReason = (data: SpecialProjectsReply, hour: number) =>
  runAt(data.partial, hour) !== null ? 'Projects were read only in part at this time.' : 'Projects were not recorded at this time.';

export function historyUnrecorded(data: readonly SpecialProjectsReply[], first: number, last: number,
  membership?: (reply: SpecialProjectsReply) => readonly HourSpan[] | undefined): string | null {
  const meets = (spans: readonly HourSpan[], from: number, to: number) => spans.some(span => span.firstHour <= to && from <= span.lastHour);
  let seen = false, gap = false, inPart = false;
  for (const reply of data) {
    const held = membership?.(reply);
    const asked = (spans: readonly HourSpan[]) => spans.flatMap(run => {
      const firstHour = Math.max(run.firstHour, first), lastHour = Math.min(run.lastHour, last);
      return firstHour <= lastHour ? [{ firstHour, lastHour }] : [];
    });
    const within = (spans: readonly HourSpan[]) => asked(held ? clipToSpans(spans, held) : spans);
    seen ||= within(reply.available).length > 0;
    for (const hole of [...withoutHours(within(reply.identities), reply.available), ...asked(reply.unresolved)]) {
      gap = true;
      inPart ||= meets(reply.partial, hole.firstHour, hole.lastHour);
    }
  }
  if (!seen) return inPart ? 'Projects were read only in part over this time.' : 'Projects were not recorded over this time.';
  return gap ? 'No projects in complete reads. Other hours are unknown.' : null;
}

export function projectCountries(selection: CountrySelection, fallback: string[], groups: CountryGroupReply | undefined, first: number, last: number) {
  const requested = worldRequestTags(selection, fallback, groups, last);
  const match = countryMatch(selection, fallback, groups, last);
  const filter = countryMatchFilter(selection, fallback, groups, last, undefined, first);
  const memberships = new Map<string, HourSpan[]>();
  const spans: MembershipRun[] = filter.membership ?? groups?.countryIdentities.flatMap(history => history.identities.map(run => ({ tag: history.tag, firstHour: run.firstHour, lastHour: run.lastHour }))) ?? [];
  for (const span of spans) {
    const history = memberships.get(span.tag);
    if (history) history.push(span); else memberships.set(span.tag, [span]);
  }
  const unobserved = selection.kind === 'group' && selection.membership === 'hour' ? filter.unobserved : [];
  if (unobserved.length) {
    const holes = [...unobserved].sort((a, b) => a.firstHour - b.firstHour);
    for (const [tag, runs] of memberships) {
      const known = withoutHours([...runs].sort((a, b) => a.firstHour - b.firstHour), holes);
      if (known.length) memberships.set(tag, known); else memberships.delete(tag);
    }
  }
  return { tags: [...memberships.keys()], roots: requested.tags, memberships, match,
    ready: requested.ready && match.ready && groups !== undefined, reason: requested.reason ?? match.reason, unobserved };
}

export function projectHistoryExtent(first: number, last: number, playhead: number | null, grow: boolean): [number, number] {
  return [first, Math.max(first + 1, grow && playhead !== null ? playhead : last + 1)];
}

export function projectCountryHour(data: SpecialProjectsReply, hour: number, membership?: readonly HourSpan[]): number | null {
  const runs = membership ? clipToSpans(data.identities, membership) : data.identities;
  let low = 0, high = runs.length;
  while (low < high) {
    const middle = (low + high) >> 1;
    if (runs[middle].firstHour <= hour) low = middle + 1; else high = middle;
  }
  return low === 0 ? null : Math.min(hour, runs[low - 1].lastHour);
}

export function projectHeading(id: string, label: CountryLabelValue, message: string): TimelineLane {
  return { id, label: message, identity: { label: label.name, countryFlag: {
    ...(label.nameFailure ? { nameFailure: label.nameFailure } : {}),
    flag: label.flag, flagPending: label.flagPending, flagReason: label.flagReason, ...(label.flagDetail !== undefined ? { flagDetail: label.flagDetail } : {}),
  } }, spans: [], heading: true };
}

export const scientistName = (data: SpecialProjectsReply, info: ProjectPresentation) => {
  const scientist = data.people[info.scientist];
  return scientist.name ?? (scientist.assigned === false ? 'No scientist assigned' : 'Scientist not recorded');
};
export const lastScientist = (data: SpecialProjectsReply, info: ProjectPresentation, run: ProjectStatus) =>
  info.lastScientist === null || run.lastScientistRead === null ? null : `${data.people[info.lastScientist].name ?? 'Name unavailable'} (read ${formatDateTime(run.lastScientistRead)})`;
export const iterationCount = (iterations: number | null) =>
  iterations === null ? 'Iterations not recorded' : `${usNumber.format(iterations)} ${iterations === 1 ? 'iteration' : 'iterations'}`;
export const endedCount = (run: ProjectStatus) => iterationCount(run.iterations);
export const iterationMark = (iterations: number | null) => iterations === null ? '—' : `×${usNumber.format(iterations)}`;
export interface ProjectCard { project: ProjectRecord; run: ProjectStatus; info: ProjectPresentation }

export function iterationTooltip(data: SpecialProjectsReply, card: ProjectCard): TooltipContent {
  const { iterations } = card.run;
  const text = iterations === null ? null : data.iterationTexts[iterations] ?? null;
  return { title: 'Iterations', subtitle: card.info.name,
    rows: [{ label: 'Prototype iterations', value: iterations === null ? 'Not recorded' : usNumber.format(iterations) }], ...(text ? { description: text } : {}) };
}

export function shownCard(data: SpecialProjectsReply, hour: number, id: string): ProjectCard | null {
  const { running, completed, earlier } = currentProjects(data, hour);
  return running.find(card => card.project.id === id) ?? completed.find(card => card.project.id === id) ?? earlier.find(card => card.project.id === id) ?? null;
}

export interface EpisodeBound { through: number; open: boolean }

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
  const found = joined(details, card);
  if (listStale(found, bound)) return `Project rewards were read ${dayLabel(found!.readHour)}, after the project list is known current (${dayLabel(bound.through)}).`;
  if (details.hour > hour) return null;
  if (!details.projects) return 'Project rewards and output were not read at this time.';
  if (!found) return 'Project rewards and output were not recorded for this project at this time.';
  if (found.readHour < card.project.firstRead) return 'Project rewards and output were last read before this project was first recorded.';
  if (found.readHour > bound.through) return 'Project rewards and output were read after this project was last recorded.';
  if (!inStatuses(card.project, bound, found.readHour)) return 'Project rewards and output were read while this project was not recorded.';
  return { project: found, choices: recordedChoices(card.project, bound, found) };
}

export function joinDue(details: SpecialProjectDetails, card: ProjectCard, hour: number, bound: EpisodeBound): { hour: number; list: boolean } | null {
  const found = joined(details, card);
  if (listStale(found, bound)) return { hour: found!.readHour, list: true };
  return details.hour > hour ? { hour: details.hour, list: false } : null;
}

const shownRuns = (project: ProjectRecord, hour: number, membership?: readonly HourSpan[]) => {
  const shown = membership ? clipToSpans(project.statuses, membership) : project.statuses;
  return shown.filter(run => run.firstHour <= hour);
};

export function historyCard(data: SpecialProjectsReply, id: string, hour: number, membership?: readonly HourSpan[]): ProjectCard | null {
  const project = data.projects.find(project => project.id === id);
  const run = project && shownRuns(project, hour, membership).at(-1);
  return project && run ? { project, run, info: data.presentations[run.presentation] } : null;
}

const shownCompletions = (project: ProjectRecord, hour: number, membership?: readonly HourSpan[]) =>
  completions(project, hour).filter(entry => !membership || runAt(membership, entry.date) !== null);

const completionSpan = (project: ProjectRecord, entry: ProjectEntry): TimelineSpan =>
  ({ id: JSON.stringify([project.id, 'completed', entry.ordinal, entry.date]), firstHour: entry.date, lastHour: entry.date, label: 'Completed', positive: true });

const completionInfo = (data: SpecialProjectsReply, dated: readonly { presentation: number | null }[]) => {
  const named = dated.map(entry => entry.presentation).find(at => at !== null);
  return named === undefined || named === null ? null : data.presentations[named];
};

export function historyProject(data: SpecialProjectsReply, id: string, hour: number, membership?: readonly HourSpan[]): ProjectRecord | null {
  const card = historyCard(data, id, hour, membership);
  if (card) return card.project;
  const project = data.projects.find(project => project.id === id);
  return project && shownCompletions(project, hour, membership).length ? project : null;
}

export const completedWhenFirstRecorded = (project: ProjectRecord) => project.firstState === 'completed';

export interface CurrentProjects { running: ProjectCard[]; completed: ProjectCard[]; earlier: ProjectCard[] }

const byName = (a: ProjectCard, b: ProjectCard) => a.info.name.localeCompare(b.info.name) || (a.project.id < b.project.id ? -1 : a.project.id > b.project.id ? 1 : 0);

export function currentProjects(data: SpecialProjectsReply, hour: number): CurrentProjects {
  const groups: CurrentProjects = { running: [], completed: [], earlier: [] };
  if (!observedAt(data, hour)) return groups;
  for (const project of data.projects) {
    const run = runAt(project.statuses, hour);
    if (!run) continue;
    const card = { project, run, info: data.presentations[run.presentation] };
    if (run.state === 'completed') (completedWhenFirstRecorded(project) ? groups.earlier : groups.completed).push(card);
    else if (run.state !== 'cancelled') groups.running.push(card);
  }
  groups.running.sort((a, b) => b.run.progress - a.run.progress || a.project.firstRead - b.project.firstRead || byName(a, b));
  groups.completed.sort((a, b) => b.run.firstHour - a.run.firstHour || byName(a, b));
  groups.earlier.sort(byName);
  return groups;
}

export function currentKey(data: SpecialProjectsReply, hour: number): string {
  if (!observedAt(data, hour)) return `unobserved${unobservedReason(data, hour)}`;
  const { running, completed, earlier } = currentProjects(data, hour);
  const part = (cards: readonly ProjectCard[], extra: (card: ProjectCard) => string) =>
    JSON.stringify(cards.map(card => [card.project.id, card.info, data.people[card.info.scientist], extra(card)]));
  const shown = (read: number | null) => `${read}\u0001${readLabel(read, hour) !== null}`;
  const read = (card: ProjectCard) => [card.run.state, card.run.iterations, shown(statusRead(data, card.project, hour)), shown(card.run.scientistRead), card.run.lastScientistRead].join('\u0001');
  return [part(running, read), part(completed, read), part(earlier, read)].join('\u0003');
}

export function projectStructure(reply: SpecialProjectsReply, hour: number) {
  return JSON.stringify([currentKey(reply, hour), reply.unavailable, reply.warnings, reply.projects.map(project => completions(project, hour).map(entry => entry.date))]);
}

export function runningValues(project: ProjectRecord, hour: number): { progress: string; fill: number; iterations: string } | null {
  const run = runAt(project.statuses, hour);
  return run ? { progress: `${run.progress}%`, fill: Math.max(0, run.progress) / 100, iterations: iterationMark(run.iterations) } : null;
}

export const earlierBound = (cards: readonly ProjectCard[]) => cards.reduce((bound, card) => Math.max(bound, card.project.firstRead), -Infinity);

export interface ProjectHistory {
  lanes: TimelineLane[];
  completed: number; cancelled: number;
  by: number | null;
}

export function projectHistory(data: SpecialProjectsReply, hour: number, membership?: readonly HourSpan[], from = -Infinity, openTo?: number): ProjectHistory {
  const history: ProjectHistory = { lanes: [], completed: 0, cancelled: 0, by: null };
  const order: { start: number; name: string; id: string; lane: TimelineLane }[] = [];
  for (const project of data.projects) {
    const shown = shownRuns(project, hour, membership);
    if (!shown.length) {
      const dated = shownCompletions(project, hour, membership);
      if (!dated.length) continue;
      const info = completionInfo(data, dated);
      order.push({ start: project.firstRead, name: info?.name ?? '', id: project.id, lane: { id: project.id, label: 'Status not recorded',
        identity: { label: info?.name ?? 'Project name not recorded', icon: info?.icon ?? undefined, tile: 'blueprint' }, muted: true, names: true, icons: false,
        spans: dated.map(entry => completionSpan(project, entry)) } });
      continue;
    }
    const latest = shown.at(-1)!;
    const info = data.presentations[latest.presentation];
    const spans: TimelineSpan[] = [];
    for (const run of shown) {
      const lastHour = openTo !== undefined && run === latest && run.lastHour >= hour ? openTo : Math.min(run.lastHour, hour);
      const label = `${run.progress}%`, positive = run.state === 'completed';
      const last = spans.at(-1);
      if (last && last.lastHour + 1 === run.firstHour && last.label === label && !!last.positive === positive) { last.lastHour = lastHour; continue; }
      spans.push({ id: JSON.stringify([project.id, run.firstHour]), firstHour: run.firstHour, lastHour, label, positive,
        ...(run.state === 'running' ? { shade: Math.min(100, Math.max(0, run.progress)) / 100 } : {}) });
    }
    const ended = Math.min(spans.at(-1)!.lastHour, hour);
    if ((latest.state === 'completed' || latest.state === 'cancelled') && ended < from) {
      if (latest.state === 'completed') history.completed++; else history.cancelled++;
      history.by = Math.max(history.by ?? -Infinity, ended);
      continue;
    }
    const marked = shownCompletions(project, hour, membership).filter(entry => !shown.some(run => run.firstHour <= entry.date && entry.date <= run.lastHour));
    if (marked.length) {
      spans.push(...marked.map(entry => completionSpan(project, entry)));
      spans.sort((a, b) => a.firstHour - b.firstHour);
    }
    order.push({ start: project.firstRead, name: info.name, id: project.id, lane: { id: project.id, label: scientistName(data, info),
      identity: { label: info.name, icon: info.icon ?? undefined, tile: 'blueprint' }, muted: latest.state === 'cancelled', names: true, icons: false, spans } });
  }
  order.sort((a, b) => a.start - b.start || a.name.localeCompare(b.name) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  history.lanes = order.map(item => item.lane);
  return history;
}

export const HISTORY_PANEL = 'special-projects-history';

export function historyStructure(reply: SpecialProjectsReply, hour: number): string {
  const end = (span: { lastHour: number }) => span.lastHour >= hour ? 'open' : span.lastHour;
  const spans = (runs: readonly HourSpan[]) => runs.map(run => [run.firstHour, end(run)]);
  return JSON.stringify([reply.tag, reply.unavailable, reply.warnings, spans(reply.available), spans(reply.partial), spans(reply.unresolved),
    reply.identities.map(run => [run.firstHour, end(run), run.slot, run.countryId]), reply.reads.map(read => [read.owner, read.firstHour, end(read), read.readHour]),
    reply.projects.map(project => [project.id, project.firstRead, project.firstState, project.entries, project.attempts,
      project.statuses.map(run => [run.firstHour, end(run), run.readHour, run.presentation, run.progress, run.state, run.iterations, run.scientistRead, run.lastScientistRead])]),
    reply.presentations, reply.people, reply.iterationTexts]);
}

export function heldThrough(reply: SpecialProjectsReply, hour: number, through: number): SpecialProjectsReply {
  if (!(through > hour)) return reply;
  const extend = <T extends { lastHour: number }>(runs: readonly T[]) => runs.map(run => run.lastHour >= hour ? { ...run, lastHour: through } : run);
  return { ...reply, available: extend(reply.available), partial: extend(reply.partial), unresolved: extend(reply.unresolved), identities: extend(reply.identities),
    reads: extend(reply.reads), projects: reply.projects.map(project => ({ ...project, statuses: extend(project.statuses) })) };
}

export function historyPicture(data: readonly SpecialProjectsReply[], hour: number, membership: (reply: SpecialProjectsReply) => readonly HourSpan[] | undefined,
  from = -Infinity, openTo?: number): string {
  return JSON.stringify(data.map(reply => {
    const held = membership(reply);
    return [reply.unavailable, reply.warnings, projectCountryHour(reply, hour, held) !== null, historyUnrecorded([reply], from, hour, () => held), projectHistory(reply, hour, held, from, openTo)];
  }));
}

export function closedLane(id: string, history: ProjectHistory): TimelineLane | null {
  if (history.by === null) return null;
  const parts = [history.completed ? `${usNumber.format(history.completed)} completed` : '', history.cancelled ? `${usNumber.format(history.cancelled)} cancelled` : ''];
  return { id, label: `${parts.filter(Boolean).join(', ')} by ${dayLabel(history.by)}`, compact: true, spans: [] };
}

export function projectTooltip(data: SpecialProjectsReply, card: ProjectCard, hour: number): TooltipContent {
  const { project, run, info } = card;
  const scientistRead = readLabel(run.scientistRead, hour);
  const rows = [{ label: 'Scientist', value: scientistRead ? `${scientistName(data, info)} (${scientistRead})` : scientistName(data, info) }];
  const last = lastScientist(data, info, run);
  if (last) rows.push({ label: 'Last observed', value: last });
  rows.push({ label: 'First recorded', value: dayLabel(project.firstRead) });
  if (run.state === 'running') rows.push({ label: 'Progress', value: `${run.progress}%` });
  else rows.push({ label: 'State', value: { completed: 'Completed', cancelled: 'Cancelled', paused: 'Paused', unknown: 'Unknown' }[run.state] });
  if (run.state === 'paused') rows.push({ label: 'Progress', value: `${run.progress}%` });
  const read = readLabel(statusRead(data, project, hour), hour);
  if (read) rows.push({ label: 'Status', value: read });
  const text = run.iterations === null ? null : data.iterationTexts[run.iterations] ?? null;
  return { title: info.name, subtitle: contentTitle(info.specializationName, info.specialization, 'Type name unavailable'), icon: info.icon ?? undefined, rows,
    ...(text ? { description: text } : {}) };
}

export function historyTooltip(data: SpecialProjectsReply, id: string, span: TimelineSpan | null, hour: number, playhead: number, membership?: readonly HourSpan[]): TooltipContent | null {
  const project = data.projects.find(project => project.id === id);
  if (!project) return null;
  const runs = shownRuns(project, playhead, membership);
  const latest = runs.at(-1);
  const at = Math.min(hour, playhead, span?.lastHour ?? Infinity);
  const dated = shownCompletions(project, playhead, membership);
  const hovered = span ? dated.find(entry => completionSpan(project, entry).id === span.id) : undefined;
  if (!latest || hovered) {
    if (!dated.length) return null;
    const entry = hovered ?? dated.filter(entry => entry.date <= at).at(-1) ?? dated[0];
    const info = entry.presentation !== null ? data.presentations[entry.presentation] : completionInfo(data, dated);
    return { title: info?.name ?? 'Project name not recorded', ...(info ? { subtitle: contentTitle(info.specializationName, info.specialization, 'Type name unavailable'), icon: info.icon ?? undefined } : {}),
      rows: [{ label: 'Completed', value: dayLabel(entry.date) }, ...latest ? [] : [{ label: 'State', value: 'Not recorded' }]],
      ...latest ? {} : { note: 'Only the history of this project was recorded, so its status is unknown.' } };
  }
  const run = runAt(runs, at);
  if (!run) return { title: latest ? data.presentations[latest.presentation].name : 'Special project', rows: [{ label: 'First recorded', value: dayLabel(project.firstRead) }], note: 'This project was not recorded at this hour.' };
  return projectTooltip(data, { project, run, info: data.presentations[run.presentation] }, at);
}

export const progressStyle = (progress: number, painted: boolean): Readonly<Record<string, number>> | undefined =>
  painted ? undefined : { '--project-progress': Math.max(0, progress) / 100 };
