import { validateProjectDetails } from './projectDetailsData.ts';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import type { CountryGroupReply, CountryIdentityRun, HourSpan, MembershipRun, ProjectDate, ProjectEpisode, ProjectPresentation, SpecialProjectRun, SpecialProjectsReply } from '../../generated/dashboard';
import type { TimelineLane, TimelineSpan } from '../../charts/timeline/layout.ts';
import { withoutHours } from '../../charts/timeline/runState.ts';
import { clipToSpans, runAt, spansCovered } from '../../charts/series.ts';
import { dayLabel } from '../../charts/draw.ts';
import { usNumber } from '../../charts/numberFormat.ts';
import type { TooltipContent } from '../../charts/tooltip.ts';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { CountrySelection } from '../../dashboard/model';
import { countryIdentityIndex, validCountryIdentityRuns } from '../../dashboard/countryIdentity.ts';
import { countryMatch, countryMatchFilter, worldRequestTags } from '../../dashboard/groups.ts';
import { preparedPrefix } from '../../dashboard/preparedPrefix.ts';

const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
const integer = (value: unknown): value is number => typeof value === 'number' && Number.isSafeInteger(value);
const nullableString = (value: unknown) => value === null || typeof value === 'string';
const date = (value: unknown) => object(value) && integer(value.hour) && typeof value.exact === 'boolean';
const span = (value: unknown) => object(value) && integer(value.firstHour) && integer(value.lastHour) && value.firstHour <= value.lastHour;

/** Validate references as well as scalars: corrupt payloads cannot turn unknown
 * scientists or absent reads into someone else's data. */
export function validateSpecialProjectsReply(input: unknown): SpecialProjectsReply {
  const bad = () => { throw new Error('Invalid special-projects reply'); };
  if (!object(input) || typeof input.branchId !== 'string' || typeof input.tag !== 'string' || !nullableString(input.unavailable)
    || !Array.isArray(input.identities) || !validCountryIdentityRuns(input.identities as CountryIdentityRun[])
    || !Array.isArray(input.available) || !input.available.every(span) || !Array.isArray(input.readHours) || !input.readHours.every(integer)
    || !object(input.iterationTexts) || !Object.entries(input.iterationTexts).every(([count, text]) => /^-?(0|[1-9][0-9]*)$/.test(count) && integer(Number(count)) && nullableString(text))
    || !Array.isArray(input.projects) || !Array.isArray(input.presentations) || !Array.isArray(input.people)
    || !Array.isArray(input.warnings) || !input.warnings.every(item => typeof item === 'string')) return bad();
  if (input.details !== undefined) validateProjectDetails(input.details);
  // A reply answered from the prepared prefix names its coverage and the hours still pending.
  try { preparedPrefix(input); } catch { return bad(); }
  if (input.current !== undefined) {
    if (!object(input.current) || input.current.current !== undefined) return bad();
    validateSpecialProjectsReply(input.current);
  }
  const ordered = (values: readonly { firstHour: number; lastHour: number }[]) => values.every((value, i) => !i || values[i - 1].lastHour < value.firstHour);
  if (!ordered(input.available as HourSpan[]) || !spansCovered(input.available as HourSpan[], input.identities as CountryIdentityRun[])
    || !(input.readHours as number[]).every((hour, i, reads) => (!i || reads[i - 1] < hour) && runAt(input.available as HourSpan[], hour) !== null)) return bad();
  for (const person of input.people) if (!object(person) || !(person.id === null || integer(person.id) && person.id >= 0)
    || !(person.assigned === null || typeof person.assigned === 'boolean') || !nullableString(person.name) || !nullableString(person.portrait) || !nullableString(person.missing)) return bad();
  for (const info of input.presentations) if (!object(info) || typeof info.name !== 'string'
    || !nullableString(info.specialization) || !nullableString(info.specializationName) || !nullableString(info.icon) || !nullableString(info.iconMissing)
    || !(info.scientistAsOf === null || integer(info.scientistAsOf)) || !integer(info.scientist) || info.scientist < 0 || info.scientist >= input.people.length) return bad();
  const ids = new Set<string>();
  for (const project of input.projects) {
    if (!object(project) || typeof project.id !== 'string' || ids.has(project.id) || !date(project.start) || !Array.isArray(project.runs)) return bad();
    ids.add(project.id);
    for (const run of project.runs) if (!span(run) || !object(run) || !integer(run.presentation) || run.presentation < 0 || run.presentation >= input.presentations.length
      || !integer(run.progress) || run.progress < -100 || run.progress > 100 || !['running', 'paused', 'cancelled', 'completed', 'unknown'].includes(run.state as string)
      || !(run.completion === null || date(run.completion)) || !(run.iterations === null || integer(run.iterations)) || !(run.cancellation === null || date(run.cancellation))) return bad();
    if (!ordered(project.runs as {firstHour:number;lastHour:number}[])) return bad();
    if (!spansCovered(project.runs as SpecialProjectRun[], input.available as HourSpan[])) return bad();
    for (const raw of project.runs) {
      const run = raw as SpecialProjectRun;
      const info = input.presentations[run.presentation] as ProjectPresentation;
      if ((project.start as ProjectDate).hour > run.firstHour || (run.state === 'completed') !== (run.completion !== null) || (run.state === 'cancelled') !== (run.cancellation !== null)
        || run.completion && run.completion.hour > run.firstHour || run.state === 'completed' && run.progress !== 100
        || run.cancellation && (run.cancellation.exact || run.cancellation.hour > run.firstHour)
        || run.iterations !== null && !(String(run.iterations) in input.iterationTexts)
        || info.scientistAsOf !== null && info.scientistAsOf > run.firstHour) return bad();
    }
  }
  return input as unknown as SpecialProjectsReply;
}

export function projectPrefix(reply: SpecialProjectsReply) {
  return preparedPrefix(reply);
}
export function projectPending(reply: SpecialProjectsReply) {
  return preparedPrefix(reply)?.pending ?? [];
}
export function currentProjectReply(reply: SpecialProjectsReply): SpecialProjectsReply | null {
  return 'current' in reply && reply.current ? reply.current as SpecialProjectsReply : null;
}

export const projectDate = (date: ProjectDate) => `${date.exact ? '' : 'by '}${dayLabel(date.hour)}`;
export const observedAt = (data: SpecialProjectsReply, hour: number) => data.available.some(span => span.firstHour <= hour && hour <= span.lastHour);

/** Request projected aliases while preserving the original choices for the server. */
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
  // Each alias carries only original roots that selected one of its shown
  // holders. Large comparisons never copy the whole choice list per request.
  const rootsFor = new Map<string, string[]>();
  if (groups) {
    const index = countryIdentityIndex(groups.countryIdentities);
    const byKey = new Map<string, string[]>();
    for (const root of requested.tags) for (const key of index.selected([root])) {
      const roots = byKey.get(key);
      if (roots) roots.push(root); else byKey.set(key, [root]);
    }
    for (const [tag, runs] of memberships) {
      const roots = new Set<string>();
      for (const run of runs) {
        const key = index.keyAt(tag, run.firstHour);
        if (key !== null) for (const root of byKey.get(key) ?? []) roots.add(root);
      }
      rootsFor.set(tag, [...roots]);
    }
  }
  return { tags: [...memberships.keys()], roots: requested.tags, rootsFor, memberships, match,
    ready: requested.ready && match.ready && groups !== undefined, reason: requested.reason ?? match.reason,
    unobserved };
}

export function projectHistoryExtent(first: number, last: number, playhead: number | null, grow: boolean): [number, number] {
  return [first, Math.max(first + 1, grow && playhead !== null ? playhead : last + 1)];
}

/** A renamed holder's heading cannot borrow its old tag's later occupant. */
export function projectCountryHour(data: SpecialProjectsReply, hour: number, membership?: readonly HourSpan[]): number | null {
  const runs = membership ? clipToSpans(data.identities, membership) : data.identities;
  let low = 0, high = runs.length;
  while (low < high) {
    const middle = (low + high) >> 1;
    if (runs[middle].firstHour <= hour) low = middle + 1; else high = middle;
  }
  return low === 0 ? null : Math.min(hour, runs[low - 1].lastHour);
}

export function projectHeading(id: string, label: ReturnType<CountryLabel>, message: string): TimelineLane {
  return { id, label: message, identity: { label: label.name, countryFlag: {
    flag: label.flag, flagPending: label.flagPending, flagReason: label.flagReason,
  } }, spans: [], heading: true };
}
/** The scientist as rows name them; the last-assignment date belongs in the tooltip. */
export const scientistName = (data: SpecialProjectsReply, info: ProjectPresentation) => {
  const scientist = data.people[info.scientist];
  return scientist.name ?? (scientist.assigned === false ? 'No scientist assigned' : 'Scientist unknown');
};
export const iterationCount = (iterations: number | null) =>
  iterations === null ? 'Iterations unknown' : `${usNumber.format(iterations)} ${iterations === 1 ? 'iteration' : 'iterations'}`;
export interface ProjectCard { project: ProjectEpisode; run: SpecialProjectRun; info: ProjectPresentation }

const shownRuns = (project: ProjectEpisode, hour: number, membership?: readonly HourSpan[]) => {
  const shown = membership ? clipToSpans(project.runs, membership) : project.runs;
  return shown.filter(run => run.firstHour <= hour);
};

export function historyCard(data: SpecialProjectsReply, id: string, hour: number, membership?: readonly HourSpan[]): ProjectCard | null {
  const project = data.projects.find(project => project.id === id);
  const run = project && shownRuns(project, hour, membership).at(-1);
  return project && run ? { project, run, info: data.presentations[run.presentation] } : null;
}

/** A project the recording first saw already complete: its completion is only
 * bounded by that first read, so it has no recorded run of its own. */
export const completedWhenFirstRecorded = (project: ProjectEpisode, run: SpecialProjectRun) =>
  !!run.completion && !run.completion.exact && run.completion.hour === project.runs[0].firstHour;

export interface CurrentProjects { running: ProjectCard[]; completed: ProjectCard[]; earlier: ProjectCard[]; cancelled: ProjectCard[] }

const byName = (a: ProjectCard, b: ProjectCard) => a.info.name.localeCompare(b.info.name) || (a.project.id < b.project.id ? -1 : a.project.id > b.project.id ? 1 : 0);

/** The projects at one hour, as Current lists them: running (and paused or
 * unknown) closest to completion first, completed newest first, those already
 * complete when first recorded apart, then cancelled. A missing hour never
 * carries an older observation; a future completion is never exposed. */
export function currentProjects(data: SpecialProjectsReply, hour: number): CurrentProjects {
  const groups: CurrentProjects = { running: [], completed: [], earlier: [], cancelled: [] };
  if (!observedAt(data, hour)) return groups;
  for (const project of data.projects) {
    const run = runAt(project.runs, hour);
    if (!run) continue;
    const card = { project, run, info: data.presentations[run.presentation] };
    if (run.state === 'completed') (completedWhenFirstRecorded(project, run) ? groups.earlier : groups.completed).push(card);
    else if (run.state === 'cancelled') groups.cancelled.push(card);
    else groups.running.push(card);
  }
  groups.running.sort((a, b) => b.run.progress - a.run.progress || a.project.start.hour - b.project.start.hour || byName(a, b));
  groups.completed.sort((a, b) => b.run.completion!.hour - a.run.completion!.hour || byName(a, b));
  groups.earlier.sort(byName);
  groups.cancelled.sort((a, b) => b.run.cancellation!.hour - a.run.cancellation!.hour || byName(a, b));
  return groups;
}

/** The latest bound of a folded group, so "completed by" holds for every member. */
export const earlierBound = (cards: readonly ProjectCard[]) => cards.reduce((bound, card) => Math.max(bound, card.run.completion!.hour), -Infinity);

export interface ProjectHistory {
  lanes: TimelineLane[];
  /** Projects that ended before the shown range, or before the recording saw them run: one closing line counts them. */
  completed: number; cancelled: number;
  /** The latest of their end bounds; null when there are none. */
  by: number | null;
}

/** One lane per project run in the shown range, in start order. A bar splits at
 * every recorded progress step (an iteration), labelled with its percent and,
 * while running, shaded deeper as progress rises; equal consecutive reads join
 * and missing intervals stay gaps. The completed end is the game's history date,
 * not the later read. Projects that ended before `from` get no empty lane. */
export function projectHistory(data: SpecialProjectsReply, hour: number, membership?: readonly HourSpan[], from = -Infinity): ProjectHistory {
  const history: ProjectHistory = { lanes: [], completed: 0, cancelled: 0, by: null };
  const order: { start: number; name: string; id: string; lane: TimelineLane }[] = [];
  for (const project of data.projects) {
    const shown = shownRuns(project, hour, membership);
    if (!shown.length) continue;
    const latest = shown.at(-1)!;
    const info = data.presentations[latest.presentation];
    const completion = latest.completion, cancellation = latest.cancellation;
    const endHour = completion?.hour ?? cancellation?.hour;
    const spans: TimelineSpan[] = [], steps: { progress: number; state: SpecialProjectRun['state'] }[] = [];
    for (const run of shown) {
      const lastHour = Math.min(run.lastHour, hour, endHour === undefined ? Infinity : endHour - 1);
      if (lastHour < run.firstHour) continue;
      const last = spans.at(-1), step = steps.at(-1);
      if (last && step && last.lastHour + 1 === run.firstHour && step.progress === run.progress && step.state === run.state) { last.lastHour = lastHour; continue; }
      spans.push({ id: JSON.stringify([project.id, run.firstHour]), firstHour: run.firstHour, lastHour, label: `${run.progress}%`, positive: !!completion,
        ...(!completion && run.state === 'running' ? { shade: Math.min(100, Math.max(0, run.progress)) / 100 } : {}) });
      steps.push({ progress: run.progress, state: run.state });
    }
    if (endHour !== undefined && (!spans.length || spans.at(-1)!.lastHour < from)) {
      if (completion) history.completed++; else history.cancelled++;
      history.by = Math.max(history.by ?? -Infinity, endHour);
      continue;
    }
    if (completion && spans.length) {
      // Mark only the stretch containing the completion date, never the edge before a gap.
      const end = spans.find(span => span.firstHour <= completion.hour && completion.hour === span.lastHour + 1);
      if (end) end.endLabel = '✓';
    }
    order.push({ start: project.start.hour, name: info.name, id: project.id, lane: { id: project.id, label: scientistName(data, info),
      identity: { label: info.name, icon: info.icon ?? undefined, tile: 'blueprint' }, muted: !!cancellation, names: true, icons: false, spans } });
  }
  order.sort((a, b) => a.start - b.start || a.name.localeCompare(b.name) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  history.lanes = order.map(item => item.lane);
  return history;
}

/** The closing line for projects that ended before the shown range, or null. */
export function closedLane(id: string, history: ProjectHistory): TimelineLane | null {
  if (history.by === null) return null;
  const parts = [history.completed ? `${usNumber.format(history.completed)} completed` : '', history.cancelled ? `${usNumber.format(history.cancelled)} cancelled` : ''];
  return { id, label: `${parts.filter(Boolean).join(', ')} by ${dayLabel(history.by)}`, compact: true, spans: [] };
}

/** Details the rows leave out: type, last-assignment date, bounded dates and the
 * game's own iteration text. "By" appears here, never in a row's plain date. */
export function projectTooltip(data: SpecialProjectsReply, card: ProjectCard, preparingStart = false): TooltipContent {
  const { project, run, info } = card;
  const rows = [{ label: 'Scientist', value: scientistName(data, info) }];
  if (info.scientistAsOf !== null && (!run.completion || dayLabel(info.scientistAsOf) !== dayLabel(run.completion.hour))) rows.push({ label: 'Last assigned', value: dayLabel(info.scientistAsOf) });
  rows.push({ label: 'Started', value: preparingStart ? 'Still being prepared' : projectDate(project.start) });
  rows.push(run.completion ? { label: 'Completed', value: projectDate(run.completion) } : { label: 'Progress', value: `${run.progress}%` });
  if (run.state === 'paused' || run.state === 'unknown') rows.push({ label: 'State', value: run.state === 'paused' ? 'Paused' : 'Unknown' });
  if (run.cancellation) rows.push({ label: 'Cancelled', value: projectDate(run.cancellation) });
  const text = run.iterations === null ? null : data.iterationTexts[run.iterations] ?? null;
  return { title: info.name, subtitle: contentTitle(info.specializationName, info.specialization, 'Type name unavailable'), icon: info.icon ?? undefined, rows,
    ...(text ? { description: text } : {}) };
}

/** The hovered observation, bounded by both the playhead and the drawn span. */
export function historyTooltip(data: SpecialProjectsReply, id: string, span: TimelineSpan | null, hour: number, playhead: number, membership?: readonly HourSpan[]): TooltipContent | null {
  const project = data.projects.find(project => project.id === id);
  if (!project) return null;
  const runs = shownRuns(project, playhead, membership);
  const latest = runs.at(-1);
  const target = Math.min(hour, playhead, span?.lastHour ?? Infinity);
  const run = span && runAt(runs, target);
  if (!run) {
    // Past a finished bar the project is recorded as completed or cancelled, not missing.
    const ended = !span ? runAt(runs, Math.min(hour, playhead)) : null;
    if (ended && (ended.completion || ended.cancellation)) return projectTooltip(data, { project, run: ended, info: data.presentations[ended.presentation] });
  }
  if (!run) return { title: latest ? data.presentations[latest.presentation].name : 'Special project', rows: [{ label: 'Started', value: projectDate(project.start) }], note: 'This project was not recorded at this hour.' };
  const tip = projectTooltip(data, { project, run, info: data.presentations[run.presentation] });
  const rows = [...tip.rows];
  // The span's observed end is separate from the value at the hovered read.
  if (span.positive && latest?.completion && !run.completion) rows.push({ label: 'Completed', value: projectDate(latest.completion) });
  if (latest?.cancellation && !run.cancellation) rows.push({ label: 'Cancelled', value: projectDate(latest.cancellation) });
  return { ...tip, rows };
}
