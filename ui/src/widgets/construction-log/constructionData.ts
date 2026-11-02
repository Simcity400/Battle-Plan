import { dayLabel } from '../../charts/draw.ts';
import type { BattleWindow, BuildingInfo, ConstructionLogManifestReply, ConstructionLogReply, ConstructionOutputRun, ConstructionProject, EndKind, LevelsBuilt, ProjectRun, SpendingGap, UnknownEnd } from '../../generated/dashboard';
import { isBuildingInfo, isSpendingGap, placeName, typeName } from '../../dashboard/construction.ts';
import type { StateNameAt } from '../../dashboard/construction';
import { readAt } from '../../dashboard/reads.ts';
import type { LogRow } from '../../charts/log/logData';
import { logCountryRequests, newestFirst } from '../../charts/log/logData.ts';
import { playbackRows, spanChanges } from '../../charts/playbackIntervals.ts';
import { MAX_NAME_HOURS } from '../../dashboard/countryFacts.ts';
import type { NameAt } from '../../dashboard/countryFacts';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { formatAmount } from '../../charts/valueAxis.ts';
import { battleWindowIndex } from '../battles/battleRows.ts';
import { preparedPrefix } from '../../dashboard/preparedPrefix.ts';

export { typeName };
export type ConstructionManifest = ConstructionLogManifestReply & { newest?: BattleWindow };
export type ConstructionWindow = ConstructionLogReply & { types?: BuildingInfo[]; exactFrom?: number };
const RAW = /^-?\d{1,20}$/;
const END_KINDS: readonly EndKind[] = ['finished', 'removed', 'unknown'];
const UNKNOWN_ENDS: readonly UnknownEnd[] = ['completionNotRecorded', 'notRecorded', 'sharedTarget', 'nothingLeft'];

const int = (value: unknown) => Number.isSafeInteger(value);
const optionalInt = (value: unknown) => value == null || int(value);
const hours = (list: unknown) => Array.isArray(list) && list.every(int) && list.every((hour, index) => index === 0 || list[index - 1] < hour);
const strings = (list: unknown) => Array.isArray(list) && list.every(item => typeof item === 'string');
const isSpan = (span: BattleWindow) => typeof span === 'object' && span != null && int(span.firstHour) && int(span.lastHour) && span.firstHour <= span.lastHour;
const isRun = (run: ProjectRun) => typeof run === 'object' && run != null && int(run.firstHour) && int(run.lastHour) && run.firstHour <= run.lastHour
  && int(run.factories) && int(run.remaining) && int(run.repairLevels);
// A run with no reason holds exact output; an absent row's reason is the table's at each read.
const isOutputRun = (run: ConstructionOutputRun) => typeof run === 'object' && run != null && int(run.from) && int(run.to) && run.from <= run.to
  && [run.constructionRaw, run.repairRaw].every(raw => raw == null || (typeof raw === 'string' && RAW.test(raw)))
  && (run.reason == null || isSpendingGap(run.reason)) && typeof run.beforeObservation === 'boolean' && typeof run.absent === 'boolean'
  && (run.reason == null && !run.absent ? run.constructionRaw != null && run.repairRaw != null : run.constructionRaw == null && run.repairRaw == null)
  && !(run.absent && (run.reason != null || run.beforeObservation));
const isProject = (project: ConstructionProject, types: number) => typeof project === 'object' && project != null && typeof project.id === 'string'
  && (project.kind === 'building' || project.kind === 'railway') && typeof project.tag === 'string'
  && int(project.building) && project.building >= 0 && project.building < types && typeof project.repair === 'boolean'
  && [project.state, project.toState, project.province, project.routeProvinces, project.baseLevel, project.targetLevel].every(optionalInt)
  && RAW.test(project.costRaw) && int(project.createdHour) && int(project.firstRead)
  && typeof project.earlierUnknown === 'boolean'
  && (project.firstRepairTick == null || (int(project.firstRepairTick) && project.firstRepairTick >= project.firstRead))
  && Array.isArray(project.runs) && project.runs.length > 0 && project.runs.every(isRun) && project.runs[0].firstHour === project.firstRead
  && project.runs.every((run, index) => index === 0 || project.runs[index - 1].lastHour < run.firstHour)
  && [project.built, project.unaccounted, project.possible, project.added].every(list => Array.isArray(list) && list.every(levels => typeof levels === 'object' && levels != null
    && optionalInt(levels.hour) && int(levels.knownAt) && int(levels.count) && levels.count > 0 && levels.knownAt >= project.firstRead
    && typeof levels.repair === 'boolean'))
  && (project.end == null || (typeof project.end === 'object' && END_KINDS.includes(project.end.kind) && optionalInt(project.end.hour)
    && int(project.end.knownAt) && project.end.knownAt > project.firstRead
    && (project.end.unknown == null || UNKNOWN_ENDS.includes(project.end.unknown))));

/** Checks the manifest's shape: its windows in hour order and the types its projects name. */
export function validateConstructionLogManifest(data: unknown): ConstructionManifest {
  const reply = data as ConstructionManifest;
  const ok = typeof reply === 'object' && reply != null && int(reply.costScale) && reply.costScale > 0
    && Array.isArray(reply.windows) && reply.windows.every((window, index) => isSpan(window) && (index === 0 || reply.windows[index - 1].firstHour < window.firstHour))
    && typeof reply.recorded === 'boolean' && Array.isArray(reply.types) && reply.types.every(isBuildingInfo)
    && (reply.repairIcon == null || typeof reply.repairIcon === 'string') && strings(reply.warnings)
    && (reply.newest === undefined || isSpan(reply.newest) && reply.windows.some(window => window.firstHour === reply.newest!.firstHour && window.lastHour === reply.newest!.lastHour));
  if (!ok) throw new Error('The construction log manifest is malformed.');
  preparedPrefix(reply);
  return reply;
}

/** Checks a window's shape: each project names one of the manifest's `types`, and its
 * runs and the output reads are in hour order. */
export function validateConstructionLogWindow(data: unknown, types: number): ConstructionWindow {
  const reply = data as ConstructionWindow;
  const ok = typeof reply === 'object' && reply != null && int(reply.window) && reply.window >= 0 && isSpan(reply.interval)
    && hours(reply.outputReads) && Array.isArray(reply.outputUnavailable) && reply.outputUnavailable.every(read => Array.isArray(read)
      && read.length === 2 && reply.outputReads.includes(read[0]) && isSpendingGap(read[1]))
    && (reply.types === undefined || Array.isArray(reply.types) && reply.types.every(isBuildingInfo))
    && (reply.exactFrom === undefined || reply.exactFrom === reply.interval.firstHour && reply.types !== undefined)
    && Array.isArray(reply.projects) && reply.projects.every(project => isProject(project, reply.types?.length ?? types))
    && Array.isArray(reply.outputs) && reply.outputs.every(output => typeof output === 'object' && output != null && typeof output.id === 'string'
      && Array.isArray(output.runs) && output.runs.length > 0 && output.runs.every((run, index) => isOutputRun(run) && (index === 0 || output.runs[index - 1].to < run.from)))
    && strings(reply.warnings);
  if (!ok) throw new Error('The construction log window is malformed.');
  preparedPrefix(reply);
  return reply;
}

/** The window an hour falls in: the last starting at or before it (an hour before the
 * first falls in the first), as the windows list projects by their rows' dates. */
export const windowAt = (windows: readonly BattleWindow[], hour: number) => Math.max(0, battleWindowIndex(windows, hour));

/** The log as the windows held show it. */
export interface ConstructionLog {
  manifest: ConstructionLogManifestReply;
  windows: ReadonlyMap<number, ConstructionLogReply>;
  /** The window of the hour shown, and the oldest window held with every window from
   * it to that one: rows dated before it are not shown yet, as others may be dated there. */
  current: number;
  held: number;
  /** Every project the windows list, once, in the scan's order. */
  projects: readonly ConstructionProject[];
  /** Each project's output runs the windows hold, by its handle, in hour order. */
  output: ReadonlyMap<string, readonly ConstructionOutputRun[]>;
}

/** The scan's order: by first read, building lines before railway lines read at the
 * same hour, then by native handle (`read_buildings`, `read_railways`). */
const scanOrder = (a: ConstructionProject, b: ConstructionProject) => a.firstRead - b.firstRead
  || Number(a.kind === 'railway') - Number(b.kind === 'railway') || a.id.length - b.id.length || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);

/** The windows held at the window `current`, merged: a project several windows list is
 * one project, with every output run any of them holds. */
export function mergeWindows(manifest: ConstructionLogManifestReply, windows: ReadonlyMap<number, ConstructionLogReply>, current: number): ConstructionLog {
  let held = current;
  while (held > 0 && windows.has(held - 1)) held--;
  const projects = new Map<string, { project: ConstructionProject; through: number }>();
  const output = new Map<string, Map<number, { run: ConstructionOutputRun; through: number }>>();
  const types = [...manifest.types];
  const byId = new Map(types.map((type, index) => [type.id, index]));
  for (const original of windows.values()) {
    const reply = original as ConstructionWindow;
    const prefix = preparedPrefix(reply);
    const through = reply.exactFrom != null || prefix?.coverage.complete !== false ? Infinity : prefix.coverage.lastHour;
    const remap = reply.types?.map(type => {
      let index = byId.get(type.id);
      if (index === undefined) { index = types.length; byId.set(type.id, index); types.push(type); }
      return index;
    });
    for (const project of reply.projects) {
      if ((projects.get(project.id)?.through ?? -Infinity) <= through)
        projects.set(project.id, { project: remap ? { ...project, building: remap[project.building] } : project, through });
    }
    for (const { id, runs } of reply.outputs) {
      const known = output.get(id) ?? new Map<number, { run: ConstructionOutputRun; through: number }>();
      for (const run of runs) {
        const before = known.get(run.from);
        if (!before || through > before.through || through === before.through && run.to >= before.run.to) known.set(run.from, { run, through });
      }
      output.set(id, known);
    }
  }
  return { manifest: { ...manifest, types }, windows, current, held, projects: [...projects.values()].map(row => row.project).sort(scanOrder),
    output: new Map([...output].map(([id, runs]) => [id, [...runs.values()].map(row => row.run).sort((a, b) => a.from - b.from)])) };
}

/** Whether the log holds every row: from the branch's start, or from a range's `from`. */
export const logComplete = (log: ConstructionLog, from: number | null) =>
  log.held === 0 || (from != null && log.manifest.windows[log.held].firstHour <= from);

/** The oldest window the log needs: as many rows held below those drawn as are drawn,
 * so scrolling down never meets a missing row. Short of them, it reaches back as far
 * again as it holds, until it holds every row. */
export function logReach(log: ConstructionLog, rows: readonly LogRow<ProjectAt>[], drawn: readonly LogRow<ProjectAt>[], from: number | null): number {
  const last = drawn.at(-1);
  const needed = (last ? rows.findIndex(row => row.key === last.key) + 1 : 0) + Math.max(1, drawn.length);
  if (needed <= rows.length) return Math.min(log.current, windowAt(log.manifest.windows, rows[needed - 1].hour));
  return logComplete(log, from) ? log.held : Math.max(0, log.held - (log.current - log.held + 1));
}

/** The windows to hold at the window `current`: back to the reach the drawn rows asked
 * for at this window or the one before (at first, the window before), and the next
 * during playback, so crossing into it never waits. */
export function constructionWindows(count: number, current: number, live: boolean, reach: { at: number; lower: number } | null): number[] {
  if (current < 0) return [];
  const lower = Math.max(0, reach && reach.lower <= current && reach.at >= current - 1 ? reach.lower : current - 1);
  return Array.from({ length: Math.min(count - 1, live ? current + 1 : current) - lower + 1 }, (_, index) => lower + index);
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
  const repaired = runs.some(item => item.repairLevels > 0) || events.some(levels => levels.repair)
    || (project.firstRepairTick != null && project.firstRepairTick <= hour);
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

/** A project's building type in the manifest. */
export const projectType = (log: ConstructionLog, project: ConstructionProject) => log.manifest.types[project.building];

/** The building types this campaign's projects name, as the game names and pictures
 * them, by name: the choices of the Building types setting and chips. */
export function buildingChoices(data: ConstructionLogManifestReply | undefined): { value: string; label: string; icon?: string | null }[] {
  return (data?.types ?? []).flatMap(type => type.token ? [{ value: type.token, label: typeName(type), icon: type.icon }] : [])
    .sort((a, b) => a.label.localeCompare(b.label));
}

export interface LogFilter {
  /** Building type tokens to show; empty shows every type. */
  types: readonly string[];
  projects: 'all' | 'new' | 'repair';
  /** Only projects still in the queue at the hour (`inQueue`). */
  queue?: boolean;
  /** Whether the country's project passes the country choice at its first read. */
  country: (tag: string, hour: number) => boolean;
  /** Only rows whose date is at or after this hour (a date range). */
  from: number | null;
}

/** Whether a project is still in the queue at its hour, as far as the reads show:
 * building or repairing, or between a gap and its next read. */
export const inQueue = (at: ProjectAt) => at.status === 'building' || at.status === 'unread';

/** The log's rows at `hour`: every project read by then that passes the filter,
 * newest first by its latest change (or its start), back to the windows held. A row
 * is keyed by its project and, ordered by change, by what is known of it, so a
 * project that reaches a new stage enters again at the top. */
export function logRows(log: ConstructionLog, hour: number, filter: LogFilter, order: 'change' | 'start'): ReadonlyArray<LogRow<ProjectAt>> {
  const types = new Set(filter.types);
  const from = Math.max(filter.from ?? -Infinity, log.held > 0 ? log.manifest.windows[log.held].firstHour : -Infinity);
  const rows: LogRow<ProjectAt>[] = [];
  for (const project of log.projects) {
    if (types.size && !types.has(projectType(log, project)?.token ?? '')) continue;
    if (!filter.country(project.tag, project.firstRead)) continue;
    const at = projectAt(project, hour);
    if (!at) continue;
    // By what it did by then: a repair that went on to build new levels is both.
    if (filter.projects === 'new' && at.kind === 'repair') continue;
    if (filter.projects === 'repair' && at.kind === 'new') continue;
    if (filter.queue && !inQueue(at)) continue;
    const date = order === 'start' ? project.createdHour : at.changed;
    if (date < from) continue;
    rows.push({ key: order === 'start' ? project.id : `${project.id}:${at.stage}`, id: project.id, hour: date, by: order === 'change' && at.changedBy, item: at });
  }
  return newestFirst(rows);
}

export function constructionPlayback(log: ConstructionLog, filter: LogFilter, order: 'change' | 'start') {
  const types = new Set(filter.types);
  const selected = { ...log, projects: log.projects.filter(project => (!types.size || types.has(projectType(log, project)?.token ?? ''))
    && filter.country(project.tag, project.firstRead)) };
  const preparedFilter = { ...filter, types: [], country: () => true };
  function* changes() {
    for (const project of selected.projects) {
      yield project.firstRead;
      if (project.firstRepairTick != null) yield project.firstRepairTick;
      if (project.end) yield project.end.knownAt;
      yield* spanChanges(project.runs);
      for (const levels of [project.built, project.unaccounted, project.possible, project.added]) for (const level of levels) yield level.knownAt;
    }
  }
  return playbackRows(changes(), hour => logRows(selected, hour, preparedFilter, order));
}

/** The countries the drawn rows name, each at its project's first read, and as many
 * hours as they ask for: rows read at many hours keep every name, newest included. */
export function rowCountryRequests(drawn: readonly LogRow<ProjectAt>[]): { wanted: NameAt[]; maxHours: number } {
  const wanted = logCountryRequests(drawn, row => ({ hour: row.item.project.firstRead, tags: [row.item.project.tag] }));
  return { wanted, maxHours: Math.max(MAX_NAME_HOURS, new Set(wanted.map(item => item.hour)).size) };
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

/** The status pill's tone: building or repairing, finished, or neither (removed,
 * left the queue, or not recorded since a gap). */
export function statusTone(at: ProjectAt): 'accent' | 'positive' | 'neutral' {
  return at.status === 'building' ? 'accent' : at.status === 'finished' ? 'positive' : 'neutral';
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
export const placeLabel = (project: ConstructionProject, hour: number, stateAt: StateNameAt) =>
  placeName({ state: project.state, toState: project.toState, province: project.province, railway: project.kind === 'railway' }, hour, stateAt).label;

/** Where the project is, as a line of its own: the state's name as the game gives
 * it, and a place without one capitalised ("Province 5007", "An unknown place"). */
export function placeTitle(project: ConstructionProject, hour: number, stateAt: StateNameAt): string {
  const label = placeLabel(project, hour, stateAt);
  return project.state != null || (project.kind === 'railway' && project.toState != null) ? label : label.charAt(0).toUpperCase() + label.slice(1);
}

/** The row read out as one sentence. */
export function describeRow(log: ConstructionLog, row: LogRow<ProjectAt>, country: string, hour: number, stateAt: StateNameAt): string {
  const at = row.item;
  const levels = levelsLabel(at);
  return `${country}, ${typeName(projectType(log, at.project))}${levels ? ` ${levels}` : ''} in ${placeLabel(at.project, hour, stateAt)}, ${statusLabel(at).toLowerCase()}, ${row.by ? 'by ' : ''}${dayLabel(row.hour)}${at.earlierUnknown ? ', earlier progress unknown' : ''}`;
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

/** The factories column of a row: only while the project is in the queue, its
 * factories at the read in view (null between a gap and the next read). An ended
 * project shows none; its tooltip keeps those of its last read. */
export function rowFactories(at: ProjectAt): { factories: number | null } | null {
  return inQueue(at) ? { factories: factoriesShown(at) } : null;
}

/** The factory output read a tooltip shows at `hour`: the project's latest by then,
 * from the runs held. A run was read at each of the output table's reads it spans,
 * which the hour's window lists; an absent row's reason is the table's at its read. */
export function outputAt(log: ConstructionLog, project: ConstructionProject, hour: number): (ConstructionOutputRun & { knownAt: number }) | null {
  const runs = log.output.get(project.id) ?? [];
  let run: ConstructionOutputRun | undefined;
  for (let index = runs.length - 1; index >= 0 && !run; index--) if (runs[index].from <= hour) run = runs[index];
  if (!run) return null;
  const window = log.windows.get(windowAt(log.manifest.windows, hour))!;
  const knownAt = hour >= run.to ? run.to : readAt(window.outputReads, hour)!;
  const reason: SpendingGap | null = run.absent ? window.outputUnavailable.find(([read]) => read === knownAt)?.[1] ?? 'finalOutputMissing' : run.reason;
  return { ...run, knownAt, reason };
}

/** The tooltip, fullest first: when it started and ended, its levels, factories and
 * cost, and (fuller) where exactly; its note dates the read behind the values. */
export function projectTooltip(log: ConstructionLog, at: ProjectAt, hour: number, country: string, stateAt: StateNameAt): TooltipContent[] {
  const { project } = at;
  const type = projectType(log, project);
  const end = project.end && project.end.knownAt <= hour ? project.end : null;
  const pending = at.status === 'unread' ? 'Not recorded' : 'Not yet';
  const rows: TooltipRow[] = [{ label: 'Started', value: dayLabel(project.createdHour) }];
  // Completions follow what it did per stretch: a repair, then new levels, each
  // with its own date.
  const dated = (risen: number | null, knownAt: number) => risen != null ? dayLabel(risen) : `by ${dayLabel(knownAt)}`;
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
  // In the queue: the factories at the read in view; once ended, those of its last read.
  const factories = factoriesShown(at);
  rows.push(inQueue(at) ? { label: 'Factories', value: factories == null ? 'Not recorded' : String(factories) }
    : { label: 'Factories at its last read', value: String(at.last.factories) });
  // Factory IC is written in whole numbers; the recorded amounts stay exact.
  const scale = Math.round(Math.log10(log.manifest.costScale));
  const cost = formatAmount(project.costRaw, scale, 0);
  rows.push(project.kind === 'railway' ? { label: 'Route cost', value: cost } : { label: `Cost per ${unit}`, value: cost });
  const output = outputAt(log, project, hour);
  if (output) {
    rows.push({ label: 'Construction output', value: output.constructionRaw == null ? 'Not recorded' : formatAmount(output.constructionRaw, scale, 0) });
    rows.push({ label: 'Repair output', value: output.repairRaw == null ? 'Not recorded' : formatAmount(output.repairRaw, scale, 0) });
  }
  const detail: TooltipRow[] = [];
  if (project.kind === 'railway') {
    detail.push({ label: 'Railway level', value: `${project.baseLevel ?? '?'} → ${project.targetLevel ?? '?'}`, divider: true });
    if (project.routeProvinces != null) detail.push({ label: 'Provinces on the route', value: String(project.routeProvinces) });
  }
  const place = placeName({ state: project.state, toState: project.toState, province: project.province, railway: project.kind === 'railway' }, hour, stateAt);
  const notes = [...new Set([place.missing ?? null, output?.reason ? 'Output unavailable.' : null, output?.beforeObservation ? 'Earlier progress unknown.' : null, end?.unknown ? 'Completion unknown.' : null, end?.kind === 'removed' ? 'Removed from the queue.' : null,
    at.earlierUnknown ? 'Earlier progress unknown.' : null,
    at.possible ? 'Completion count uncertain.' : null].filter(Boolean))].join(' ');
  const base = { title: typeName(type), icon: type?.icon ?? undefined, subtitle: `${country} · ${place.label}${at.kind === 'new' ? '' : ` · ${kindLabel(at.kind).toLowerCase()}`}`, note: notes || undefined };
  return [{ ...base, rows: [...rows, ...detail] }, { ...base, rows }];
}
