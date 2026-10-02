import type { ConstructionLogReply, ConstructionProject, EndKind, LevelsBuilt, ProjectRun, UnknownEnd } from '../../generated/dashboard';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { CONSTRUCTION_CADENCE, isBuildingInfo, isStateName, placeName, stateName as stateNameIn, typeName } from '../../dashboard/construction.ts';
import { readAt, readNote } from '../../dashboard/reads.ts';
import type { LogRow } from '../../charts/log/logData';
import { newestFirst } from '../../charts/log/logData.ts';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { formatAmount } from '../../charts/valueAxis.ts';

export { typeName };
const RAW = /^-?\d{1,20}$/;
const END_KINDS: readonly EndKind[] = ['finished', 'removed', 'unknown'];
/** Why an ending is not known, as the tooltip says it. */
export const UNKNOWN_END: Readonly<Record<UnknownEnd, string>> = {
  notRecorded: 'Some hours before this read were not recorded, so whether it was finished is not known.',
  sharedTarget: 'Another project was building the same thing there, so which of them the building’s rise belongs to is not known.',
  nothingLeft: 'Its last read showed no levels left to build.',
  routeUnclear: 'No rise of the railway’s last province that its own construction could have made shows when it was finished, or another railway through it changed then.',
  risesUnclear: 'Its building also rose in ways its own construction cannot be told apart from (a focus, decision, event or effect), so whether it was finished is not known.',
};

const int = (value: unknown) => Number.isSafeInteger(value);
const optionalInt = (value: unknown) => value == null || int(value);
const isRun = (run: ProjectRun) => typeof run === 'object' && run != null && int(run.firstHour) && int(run.lastHour) && run.firstHour <= run.lastHour
  && int(run.factories) && int(run.remaining) && int(run.repairLevels);
const isProject = (project: ConstructionProject, types: number) => typeof project === 'object' && project != null && typeof project.id === 'string'
  && (project.kind === 'building' || project.kind === 'railway') && typeof project.tag === 'string'
  && int(project.building) && project.building >= 0 && project.building < types && typeof project.repair === 'boolean'
  && [project.state, project.toState, project.province, project.routeProvinces, project.baseLevel, project.targetLevel].every(optionalInt)
  && RAW.test(project.costRaw) && int(project.createdHour) && int(project.firstRead)
  && typeof project.earlierUnknown === 'boolean'
  && Array.isArray(project.runs) && project.runs.length > 0 && project.runs.every(isRun) && project.runs[0].firstHour === project.firstRead
  && project.runs.every((run, index) => index === 0 || project.runs[index - 1].lastHour < run.firstHour)
  && [project.built, project.unaccounted, project.possible, project.added].every(list => Array.isArray(list) && list.every(levels => typeof levels === 'object' && levels != null
    && optionalInt(levels.hour) && int(levels.knownAt) && int(levels.count) && levels.count > 0 && levels.knownAt >= project.firstRead
    && typeof levels.repair === 'boolean'))
  && (project.end == null || (typeof project.end === 'object' && END_KINDS.includes(project.end.kind) && optionalInt(project.end.hour)
    && int(project.end.knownAt) && project.end.knownAt > project.firstRead
    && (project.end.unknown == null || Object.hasOwn(UNKNOWN_END, project.end.unknown))));

/** Checks the reply's shape; each project names a listed type and its runs are in hour order. */
export function validateConstructionLogReply(data: unknown): ConstructionLogReply {
  const reply = data as ConstructionLogReply;
  const hours = (list: unknown) => Array.isArray(list) && list.every(int) && list.every((hour, index) => index === 0 || list[index - 1] < hour);
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string' && int(reply.costScale) && reply.costScale > 0
    && hours(reply.reads) && hours(reply.railwayReads) && Array.isArray(reply.types) && reply.types.every(isBuildingInfo)
    && (reply.repairIcon == null || typeof reply.repairIcon === 'string')
    && Array.isArray(reply.states) && reply.states.every(isStateName)
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string')
    && Array.isArray(reply.projects) && reply.projects.every(project => isProject(project, reply.types.length));
  if (!ok) throw new Error('The construction log reply is malformed.');
  return reply;
}

/** Where a project stands at an hour, as far as the reads by then show. */
export type Status = 'building' | 'unread' | 'finished' | 'removed' | 'ended';

/** What a project has done by an hour, per stretch: built new levels, repaired, or
 * repaired and then built the new levels queued behind the repair. */
export type ProjectKind = 'new' | 'repair' | 'repairThenNew';

export interface ProjectAt {
  project: ConstructionProject;
  status: Status;
  /** Work before its first read was not recorded; counts are lower bounds. */
  earlierUnknown: boolean;
  /** Levels (railway: segments) its reads by then show built. */
  built: number;
  /** Levels its reads by then show it lost where the recording cannot tell built
   * from dropped (`unaccounted`). */
  unknown: number;
  /** Extra levels it may have both built and had added, known only by the read. */
  possible: number;
  /** All its levels as then known: built, unaccounted, added and still to build or
   * repair. */
  levels: number;
  /** Levels its reads by then show added to it (queued more, or new damage). */
  added: number;
  /** What it did by then, per stretch, and whether it is repairing now. */
  kind: ProjectKind;
  repairing: boolean;
  /** The read whose values hold at the hour; null between a gap and the next read, or once it ended. */
  run: ProjectRun | null;
  /** Its last read at or before the hour. */
  last: ProjectRun;
  /** Its latest change by then: when it ended, a level was built, or it started. */
  changed: number;
  /** That hour is an upper bound: only the read shows it. */
  changedBy: boolean;
  /** The events known by then, so a row that changes gets a new identity. */
  stage: number;
}

/** A project at `hour`: null before its first read. Nothing shows before the read
 * that establishes it (`knownAt`). */
export function projectAt(project: ConstructionProject, hour: number): ProjectAt | null {
  if (hour < project.firstRead) return null;
  const sum = (list: readonly LevelsBuilt[], until: number) => list.reduce((total, levels) => total + (levels.knownAt <= until ? levels.count : 0), 0);
  const runs = project.runs.filter(run => run.firstHour <= hour);
  const last = runs[runs.length - 1];
  const end = project.end && project.end.knownAt <= hour ? project.end : null;
  const run = !end && last.lastHour >= hour ? last : null;
  const status: Status = !end ? run ? 'building' : 'unread' : end.kind === 'finished' ? 'finished' : end.kind === 'removed' ? 'removed' : 'ended';
  const count = sum(project.built, hour);
  const unknown = sum(project.unaccounted, hour);
  const possible = sum(project.possible, hour);
  // Its last read's levels still to build and to repair, with what it had built or
  // lost by that read. Levels known after it (at its ending) came out of those, so
  // they are not counted twice, except what the ending shows added.
  const readHour = Math.min(hour, last.lastHour);
  const before = sum(project.built, readHour) + sum(project.unaccounted, readHour);
  const addedLater = sum(project.added, hour) - sum(project.added, readHour);
  const levels = before + Math.max(0, last.remaining) + Math.max(0, last.repairLevels) + addedLater;
  const events = [...project.built, ...project.unaccounted, ...project.possible].filter(levels => levels.knownAt <= hour);
  // A stretch repairs while it has levels to repair (the engine's repair branch),
  // else builds its remaining new levels.
  const repaired = runs.some(item => item.repairLevels > 0) || events.some(levels => levels.repair);
  // New levels queued behind a repair make it a repair, then new, from the start.
  const builtNew = runs.some(item => item.remaining > 0) || events.some(levels => !levels.repair);
  const kind: ProjectKind = repaired && builtNew ? 'repairThenNew' : repaired ? 'repair' : 'new';
  const repairing = run != null && run.repairLevels > 0;
  // The latest change: the latest read to confirm one, and of the changes one read
  // confirms (several levels, say, after a read that failed), the latest dated.
  const dated = (levels: LevelsBuilt) => levels.hour ?? levels.knownAt;
  const latest = events.reduce<LevelsBuilt | null>((found, levels) => !found || levels.knownAt > found.knownAt
    || (levels.knownAt === found.knownAt && dated(levels) > dated(found)) ? levels : found, null);
  const [changed, changedBy] = end ? [end.hour ?? end.knownAt, end.hour == null]
    : latest ? [latest.hour ?? latest.knownAt, latest.hour == null] : [project.createdHour, false];
  return { project, status, earlierUnknown: project.earlierUnknown, built: count, unknown, possible, levels, added: sum(project.added, hour), kind, repairing, run, last, changed, changedBy,
    stage: events.length + (end ? 1 : 0) };
}

/** A project's building type in the reply. */
export const projectType = (reply: ConstructionLogReply, project: ConstructionProject) => reply.types[project.building];

/** The building types this campaign's projects name, as the game names and pictures
 * them, by name: the choices of the Building types setting and chips. */
export function buildingChoices(data: ConstructionLogReply | undefined): { value: string; label: string; icon?: string | null }[] {
  return (data?.types ?? []).flatMap(type => type.token ? [{ value: type.token, label: typeName(type), icon: type.icon }] : [])
    .sort((a, b) => a.label.localeCompare(b.label));
}

/** A state's name as the game gives it, else its number. */
export const stateName = (reply: ConstructionLogReply, state: number | null) => stateNameIn(reply.states, state);

export interface LogFilter {
  /** Building type tokens to show; empty shows every type. */
  types: readonly string[];
  projects: 'all' | 'new' | 'repair';
  /** Whether the country's project passes the country choice at its first read. */
  country: (tag: string, hour: number) => boolean;
  /** Only rows whose date is at or after this hour (a date range). */
  from: number | null;
}

/** The log's rows at `hour`: every project read by then that passes the filter,
 * newest first by its latest change (or its start). A row is keyed by its project
 * and, ordered by change, by what is known of it, so a project that reaches a new
 * stage enters again at the top. */
export function logRows(reply: ConstructionLogReply, hour: number, filter: LogFilter, order: 'change' | 'start'): LogRow<ProjectAt>[] {
  const types = new Set(filter.types);
  const rows: LogRow<ProjectAt>[] = [];
  for (const project of reply.projects) {
    if (types.size && !types.has(projectType(reply, project)?.token ?? '')) continue;
    if (!filter.country(project.tag, project.firstRead)) continue;
    const at = projectAt(project, hour);
    if (!at) continue;
    // By what it did by then: a repair that went on to build new levels is both.
    if (filter.projects === 'new' && at.kind === 'repair') continue;
    if (filter.projects === 'repair' && at.kind === 'new') continue;
    const date = order === 'start' ? project.createdHour : at.changed;
    if (filter.from != null && date < filter.from) continue;
    rows.push({ key: order === 'start' ? project.id : `${project.id}:${at.stage}`, id: project.id, hour: date, by: order === 'change' && at.changedBy, item: at });
  }
  return newestFirst(rows);
}

/** The status as a row's tag says it. */
export function statusLabel(at: ProjectAt): string {
  switch (at.status) {
    case 'building': return at.repairing ? 'Repairing' : at.kind === 'repairThenNew' ? 'Building, after repair' : 'Building';
    case 'unread': return 'Not recorded';
    case 'finished': return at.kind === 'repair' ? 'Repaired' : at.kind === 'repairThenNew' ? 'Repaired and built' : 'Built';
    case 'removed': return 'Removed';
    case 'ended': return 'Left the queue';
  }
}

/** What a project did, as the log's repair marking says it. */
export function kindLabel(kind: ProjectKind): string {
  return kind === 'repair' ? 'Repair' : kind === 'repairThenNew' ? 'Repair, then new' : 'New';
}

/** The levels part of a row: "×3" for several, "2 of 3" while building several, none
 * for one. Levels the recording cannot tell built from dropped keep the count a range
 * ("1–2 of 2") at every status, as the tooltip gives it: an ending never settles them. */
export function levelsLabel(at: ProjectAt): string | null {
  if (at.earlierUnknown || at.unknown > 0 || at.possible > 0) return builtLabel(at);
  if (at.levels <= 1) return null;
  return (at.status === 'building' || at.status === 'unread') && at.built > 0 ? builtLabel(at) : `×${at.levels}`;
}

/** Where the project is: its state (a railway: from one state to another). */
export const placeLabel = (reply: ConstructionLogReply, project: ConstructionProject) =>
  placeName(reply.states, { state: project.state, toState: project.toState, province: project.province, railway: project.kind === 'railway' });

/** The row read out as one sentence. */
export function describeRow(reply: ConstructionLogReply, row: LogRow<ProjectAt>, country: string): string {
  const at = row.item;
  const levels = levelsLabel(at);
  return `${country}, ${typeName(projectType(reply, at.project))}${levels ? ` ${levels}` : ''} in ${placeLabel(reply, at.project)}, ${statusLabel(at).toLowerCase()}, ${row.by ? 'by ' : ''}${formatDateTime(row.hour)}${at.earlierUnknown ? ', work before its first read was not recorded' : ''}`;
}

/** Levels built as then known, with ranges for work built or dropped unseen and
 * extra levels it may have both built and had added ("1–2 of 2–3"). */
export function builtLabel(at: ProjectAt): string {
  const more = at.earlierUnknown ? '+' : '';
  const extra = at.unknown + at.possible;
  const built = extra ? `${at.built}–${at.built + extra}` : at.built;
  const total = at.possible ? `${at.levels}–${at.levels + at.possible}` : at.levels;
  return `${built}${more} of ${total}${more}`;
}

/** Where a row's Jump to menu can go, in time order: when the project started, its
 * latest change and its ending, as far as the reads by `hour` show them, whatever
 * the row is ordered by. */
export function jumpHours(at: ProjectAt, hour: number): number[] {
  const { end } = at.project;
  const ended = end && end.knownAt <= hour ? [end.hour ?? end.knownAt] : [];
  return [...new Set([at.project.createdHour, at.changed, ...ended])].sort((a, b) => a - b);
}

/** The factories on a project as the row shows them: at the read in view, or at its
 * last read once it ended; null between a gap and the next read. */
export function factoriesShown(at: ProjectAt): number | null {
  return at.run ? at.run.factories : at.status === 'unread' ? null : at.last.factories;
}

/** The read that supplied the factories and levels shown at `hour`: the latest read
 * of the run in view, or of its last run once it ended; null when none is shown. */
export function valuesRead(reply: ConstructionLogReply, at: ProjectAt, hour: number): number | null {
  const reads = at.project.kind === 'railway' ? reply.railwayReads : reply.reads;
  if (at.run) return readAt(reads, Math.min(hour, at.run.lastHour));
  return at.status === 'unread' ? null : readAt(reads, at.last.lastHour);
}

/** The tooltip, fullest first: when it started and ended, its levels, factories and
 * cost, and (fuller) where exactly; its note dates the read behind the values. */
export function projectTooltip(reply: ConstructionLogReply, at: ProjectAt, hour: number, country: string): TooltipContent[] {
  const { project } = at;
  const type = projectType(reply, project);
  const end = project.end && project.end.knownAt <= hour ? project.end : null;
  const pending = at.status === 'unread' ? 'Not recorded' : 'Not yet';
  const rows: TooltipRow[] = [{ label: 'Started', value: formatDateTime(project.createdHour) }];
  // Completions follow what it did per stretch: a repair, then new levels, each
  // with its own date.
  const dated = (risen: number | null, knownAt: number) => risen != null ? formatDateTime(risen) : `by ${formatDateTime(knownAt)}`;
  if (at.kind === 'repairThenNew') {
    const latest = project.built.filter(levels => levels.repair && levels.knownAt <= hour)
      .reduce<LevelsBuilt | null>((found, levels) => !found || (levels.hour ?? levels.knownAt) > (found.hour ?? found.knownAt) ? levels : found, null);
    const repaired = end?.kind === 'finished' || (at.run ? at.run.repairLevels <= 0 : end != null && at.last.repairLevels <= 0);
    if (repaired) rows.push(latest ? { label: 'Repaired', value: dated(latest.hour, latest.knownAt), tone: 'positive' } : { label: 'Repaired', value: 'Not recorded', tone: 'muted' });
    else if (!end) rows.push({ label: 'Repaired', value: pending, tone: 'muted' });
  }
  const completed = at.kind === 'repair' ? 'Repaired' : at.kind === 'repairThenNew' ? 'Built' : 'Finished';
  if (end) {
    const label = end.kind === 'finished' ? completed : end.kind === 'removed' ? 'Removed' : 'Left the queue';
    rows.push({ label, value: dated(end.hour, end.knownAt), tone: end.kind === 'finished' ? 'positive' : end.kind === 'removed' ? 'negative' : 'muted' });
  } else rows.push({ label: at.kind === 'repairThenNew' ? 'Built' : 'Finished', value: pending, tone: 'muted' });
  const unit = project.kind === 'railway' ? 'segment' : 'level';
  const done = at.kind === 'repair' ? 'Levels repaired' : at.kind === 'repairThenNew' ? 'Levels repaired and built'
    : project.kind === 'railway' ? 'Segments built' : 'Levels built';
  rows.push({ label: done, value: builtLabel(at) });
  if (at.added || at.possible) {
    const added = at.possible ? `${at.added}–${at.added + at.possible}` : at.added;
    rows.push({ label: 'Added while under way', value: `${added} ${unit}${at.added + at.possible === 1 ? '' : 's'}`, tone: 'muted' });
  }
  if (at.unknown) rows.push({ label: 'Built or dropped, not recorded', value: `${at.unknown} ${unit}${at.unknown === 1 ? '' : 's'}`, tone: 'muted' });
  const factories = factoriesShown(at);
  rows.push({ label: 'Factories', value: factories == null ? 'Not recorded since a gap' : String(factories) });
  const cost = formatAmount(project.costRaw, Math.round(Math.log10(reply.costScale)));
  rows.push({ label: 'Cost', value: project.kind === 'railway' ? `${cost} for the route` : `${cost} per ${unit}` });
  const detail: TooltipRow[] = [];
  if (project.province != null) detail.push({ label: 'Province', value: String(project.province), divider: true });
  if (project.kind === 'railway') {
    detail.push({ label: 'Railway level', value: `${project.baseLevel ?? '?'} → ${project.targetLevel ?? '?'}`, divider: true });
    if (project.routeProvinces != null) detail.push({ label: 'Provinces on the route', value: String(project.routeProvinces) });
  }
  const notes = [end?.unknown ? UNKNOWN_END[end.unknown] : null, end?.kind === 'removed' ? 'It left the queue unfinished: cancelled, or its builder lost the place. The recording does not say which.' : null,
    at.earlierUnknown ? 'Work before its first read was not recorded.' : null,
    at.possible ? 'Extra possible completions may have been built with more levels added, or supplied by a focus, decision, event or effect.' : null,
    readNote(valuesRead(reply, at, hour), hour, CONSTRUCTION_CADENCE)].filter(Boolean).join(' ');
  const base = { title: typeName(type), icon: type?.icon ?? undefined, subtitle: `${country} · ${placeLabel(reply, project)}${at.kind === 'new' ? '' : ` · ${kindLabel(at.kind).toLowerCase()}`}`, note: notes || undefined };
  return [{ ...base, rows: [...rows, ...detail] }, { ...base, rows }];
}
