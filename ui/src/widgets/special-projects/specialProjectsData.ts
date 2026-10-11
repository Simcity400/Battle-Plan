import { contentTitle } from '../../dashboard/contentTitles.ts';
import type { CountryGroupReply, CountryIdentityRun, HourSpan, MembershipRun, ProjectDate, ProjectEpisode, ProjectPresentation, SpecialProjectRun, SpecialProjectsReply } from '../../generated/dashboard';
import type { TimelineLane, TimelineSpan } from '../../charts/timeline/layout.ts';
import { withoutHours } from '../../charts/timeline/runState.ts';
import { clipToSpans, runAt, spansCovered } from '../../charts/series.ts';
import { dayLabel } from '../../charts/draw.ts';
import type { TooltipContent } from '../../charts/tooltip.ts';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { CountrySelection } from '../../dashboard/model';
import { countryIdentityIndex, validCountryIdentityRuns } from '../../dashboard/countryIdentity.ts';
import { countryMatch, countryMatchFilter, worldRequestTags } from '../../dashboard/groups.ts';

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
export const scientistName = (data: SpecialProjectsReply, info: ProjectPresentation) => {
  const scientist = data.people[info.scientist];
  const name = scientist.name ?? (scientist.assigned === false ? 'Unassigned' : 'Unknown scientist');
  return info.scientistAsOf === null ? name : `Last assigned · ${name} (as of ${dayLabel(info.scientistAsOf)})`;
};
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

/** Step lookup, with fixed game-day retention. A missing hour never carries a
 * card from an older observation; future completion is never exposed. */
export function liveCards(data: SpecialProjectsReply, hour: number, retentionDays: number): ProjectCard[] {
  if (!observedAt(data, hour)) return [];
  return data.projects.flatMap(project => {
    const run = runAt(project.runs, hour);
    if (!run || run.state === 'cancelled' || run.state === 'completed' && run.completion && hour >= run.completion.hour + retentionDays * 24) return [];
    return [{ project, run, info: data.presentations[run.presentation] }];
  });
}

/** One lane per observed episode; equal state changes do not fragment bars,
 * missing intervals do. The completed end is the game's history date, not
 * the later daily read. The latest known name is dated to the view's hour. */
export function projectLanes(data: SpecialProjectsReply, hour: number, membership?: readonly { firstHour: number; lastHour: number }[]): TimelineLane[] {
  return data.projects.flatMap(project => {
    const shown = shownRuns(project, hour, membership);
    if (!shown.length) return [];
    const latest = shown.at(-1)!;
    const info = data.presentations[latest.presentation];
    const completion = latest.completion;
    const cancellation = latest.cancellation;
    const endHour = completion?.hour ?? cancellation?.hour;
    const spans: TimelineSpan[] = [];
    for (const run of shown) {
      const lastHour = Math.min(run.lastHour, hour, endHour === undefined ? Infinity : endHour - 1);
      if (lastHour < run.firstHour) continue;
      const last = spans.at(-1);
      if (last && last.lastHour + 1 === run.firstHour) last.lastHour = lastHour;
      else spans.push({ id: JSON.stringify([project.id, run.firstHour]), firstHour: run.firstHour, lastHour, label: info.name, emphasis: latest.state === 'running', positive: !!completion });
    }
    if (completion && spans.length) {
      // Mark only the stretch containing the completion date, never the edge
      // before a gap. A pre-recording completion is named, without a fake bar.
      const end = spans.find(span => span.firstHour <= completion.hour && completion.hour === span.lastHour + 1);
      if (end) end.endLabel = '✓';
    }
    return [{ id: project.id, label: scientistName(data, info), identity: { label: info.name, icon: info.icon ?? undefined, tile: 'blueprint' }, muted: !!cancellation, spans }];
  });
}

export function projectTooltip(data: SpecialProjectsReply, card: ProjectCard, history = false): TooltipContent {
  const { project, run, info } = card;
  const rows = [
    ...(!history ? [{ label: 'Type', value: contentTitle(info.specializationName, info.specialization, 'Type name unavailable') }] : []),
    { label: 'Scientist', value: scientistName(data, info) },
    { label: 'Started', value: projectDate(project.start) },
    run.state === 'completed' && run.completion && history ? { label: 'Completed', value: projectDate(run.completion) } : { label: 'Progress', value: `${run.progress}%` },
  ];
  if (run.state !== 'running' && run.state !== 'completed') rows.push({ label: 'State', value: run.state === 'paused' ? 'Paused' : run.state === 'cancelled' ? 'Cancelled' : 'Not recorded' });
  if (run.cancellation) rows.push({ label: 'Cancelled', value: projectDate(run.cancellation) });
  return { title: info.name, icon: info.icon ?? undefined, rows };
}

/** The hovered observation, bounded by both the playhead and the drawn span. */
export function historyTooltip(data: SpecialProjectsReply, id: string, span: TimelineSpan | null, hour: number, playhead: number, membership?: readonly HourSpan[]): TooltipContent | null {
  const project = data.projects.find(project => project.id === id);
  if (!project) return null;
  const runs = shownRuns(project, playhead, membership);
  const latest = runs.at(-1);
  const target = Math.min(hour, playhead, span?.lastHour ?? Infinity);
  const run = span && runAt(runs, target);
  if (!run) return { title: latest ? data.presentations[latest.presentation].name : 'Special project', rows: [{ label: 'Started', value: projectDate(project.start) }], note: 'This project was not recorded at this hour.' };
  const tip = projectTooltip(data, { project, run, info: data.presentations[run.presentation] }, true);
  const rows = [...tip.rows];
  // The span's observed end is separate from the value at the hovered read.
  if (span.positive && latest?.completion && !run.completion) rows.push({ label: 'Completed', value: projectDate(latest.completion) });
  if (latest?.cancellation && !run.cancellation) rows.push({ label: 'Cancelled', value: projectDate(latest.cancellation) });
  return { ...tip, rows };
}
