import { dayLabel } from '../../charts/draw.ts';
import { isConstructionType, isMissingSpan, isSpendingGap, isStateName, placeName, stateNamesOf, typeName } from '../../dashboard/construction.ts';
import type { StateNameAt } from '../../dashboard/construction';
import type { LogRow } from '../../charts/log/logData';
import { logCountryRequests, newestFirst } from '../../charts/log/logData.ts';
import { logStatesBy } from '../../charts/log/logStates.ts';
import { playbackRows } from '../../charts/playbackIntervals.ts';
import type { LiveTexts, TextClock } from '../../charts/LiveText';
import { MAX_NAME_HOURS } from '../../dashboard/countryFacts.ts';
import type { NameAt } from '../../dashboard/countryFacts';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { formatAmount } from '../../charts/valueAxis.ts';
import type { BattleWindow, ConstructionLogManifestReply, ConstructionLogReply, ConstructionOutputRun, ConstructionProject, ConstructionType, EndKind, LevelsBuilt, ProjectRun, UnknownEnd } from '../../generated/dashboard';

export { typeName };
const RAW = /^-?\d{1,20}$/;
const END_KINDS: readonly EndKind[] = ['finished', 'removed', 'unknown'];
const UNKNOWN_ENDS: readonly UnknownEnd[] = ['completionNotRecorded', 'notRecorded', 'sharedTarget', 'nothingLeft'];

const int = (value: unknown) => Number.isSafeInteger(value);
const optionalInt = (value: unknown) => value == null || int(value);
const strings = (list: unknown) => Array.isArray(list) && list.every(item => typeof item === 'string');
const isSpan = (span: BattleWindow) => typeof span === 'object' && span != null && int(span.firstHour) && int(span.lastHour) && span.firstHour <= span.lastHour;
const isRun = (run: ProjectRun) => isSpan(run) && int(run.factories) && int(run.remaining) && int(run.repairLevels);
// A run with no reason holds exact output.
const isOutputRun = (run: ConstructionOutputRun) => typeof run === 'object' && run != null && int(run.from) && int(run.to) && run.from <= run.to
  && [run.constructionRaw, run.repairRaw].every(raw => raw == null || (typeof raw === 'string' && RAW.test(raw)))
  && (run.reason == null || isSpendingGap(run.reason)) && typeof run.beforeObservation === 'boolean'
  && (run.reason == null) === (run.constructionRaw != null && run.repairRaw != null);
const isLevels = (levels: LevelsBuilt) => typeof levels === 'object' && levels != null && optionalInt(levels.hour) && int(levels.knownAt)
  && int(levels.count) && levels.count > 0 && typeof levels.repair === 'boolean' && typeof levels.carried === 'boolean';
const inOrder = <T>(list: readonly T[], first: (item: T) => number, last: (item: T) => number) => list.every((item, index) => index === 0 || last(list[index - 1]) < first(item));
const isProject = (project: ConstructionProject) => typeof project === 'object' && project != null && typeof project.id === 'string'
  && (project.kind === 'building' || project.kind === 'railway') && typeof project.tag === 'string'
  && (project.source === 'census' || project.source === 'outputAlone') && optionalInt(project.censusRead) && optionalInt(project.queueOrdinal)
  && optionalInt(project.building) && (project.repair == null || typeof project.repair === 'boolean')
  && [project.state, project.toState, project.province, project.routeProvinces, project.baseLevel, project.targetLevel, project.createdHour].every(optionalInt)
  && (project.costRaw == null || RAW.test(project.costRaw)) && int(project.firstRead)
  && (project.outputOrigin == null || (typeof project.outputOrigin.tag === 'string'
    && [project.outputOrigin.building, project.outputOrigin.state, project.outputOrigin.toState, project.outputOrigin.province].every(optionalInt)))
  && typeof project.earlierUnknown === 'boolean' && optionalInt(project.firstRepairTick)
  && Array.isArray(project.runs) && project.runs.every(isRun) && inOrder(project.runs, run => run.firstHour, run => run.lastHour)
  && [project.built, project.unaccounted, project.possible, project.added].every(list => Array.isArray(list) && list.every(isLevels))
  && Array.isArray(project.outputs) && project.outputs.every(isOutputRun) && inOrder(project.outputs, run => run.from, run => run.to)
  && (project.end == null || (typeof project.end === 'object' && END_KINDS.includes(project.end.kind) && optionalInt(project.end.hour)
    && int(project.end.knownAt) && (project.end.unknown == null || UNKNOWN_ENDS.includes(project.end.unknown))));

/** Checks the manifest's shape: its windows in hour order and the types its lines name. */
export function validateConstructionLogManifest(data: unknown): ConstructionLogManifestReply {
  const reply = data as ConstructionLogManifestReply;
  const ok = typeof reply === 'object' && reply != null && int(reply.costScale) && reply.costScale > 0
    && Array.isArray(reply.windows) && reply.windows.every(isSpan) && inOrder(reply.windows, window => window.firstHour, window => window.lastHour)
    && typeof reply.recorded === 'boolean' && Array.isArray(reply.types) && reply.types.every(isConstructionType)
    && (reply.repairIcon == null || typeof reply.repairIcon === 'string') && Array.isArray(reply.missing) && reply.missing.every(isMissingSpan)
    && strings(reply.warnings);
  if (!ok) throw new Error('The construction log manifest is malformed.');
  return reply;
}

/** Checks a window's shape: its lines, their runs and output in hour order, and the states it names. */
export function validateConstructionLogWindow(data: unknown): ConstructionLogReply {
  const reply = data as ConstructionLogReply;
  const ok = typeof reply === 'object' && reply != null && int(reply.window) && reply.window >= 0 && isSpan(reply.interval)
    && Array.isArray(reply.projects) && reply.projects.every(isProject) && Array.isArray(reply.states) && reply.states.every(isStateName)
    && strings(reply.warnings);
  if (!ok) throw new Error('The construction log window is malformed.');
  return reply;
}

/** The window an hour falls in: the last starting at or before it (an hour before the first falls in the first). */
export function windowAt(windows: readonly BattleWindow[], hour: number): number {
  let low = 0, high = windows.length;
  while (low < high) { const middle = (low + high) >> 1; if (windows[middle].firstHour <= hour) low = middle + 1; else high = middle; }
  return Math.max(0, low - 1);
}

/** The log as the windows held show it. */
export interface ConstructionLog {
  manifest: ConstructionLogManifestReply;
  windows: ReadonlyMap<number, ConstructionLogReply>;
  /** The window of the hour shown, and the oldest window held with every window from
   * it to that one: rows dated before it are not shown yet, as others may be dated there. */
  current: number;
  held: number;
  /** Every line the windows list, once, joined across them, in first-read order. */
  projects: readonly ConstructionProject[];
  types: ReadonlyMap<number, ConstructionType>;
  states: StateNameAt;
}

/** By first read, building lines before railway lines read at the same hour, then by native handle. */
const scanOrder = (a: ConstructionProject, b: ConstructionProject) => a.firstRead - b.firstRead
  || Number(a.kind === 'railway') - Number(b.kind === 'railway') || a.id.length - b.id.length || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);

/** Spans joined where one ends the hour before the next begins with the same values. */
function joined<T>(a: readonly T[], b: readonly T[], first: (item: T) => number, last: (item: T) => number, same: (x: T, y: T) => boolean, extend: (x: T, y: T) => T): T[] {
  const out = [...a];
  for (const item of b) {
    const held = out.at(-1);
    if (held && last(held) + 1 === first(item) && same(held, item)) out[out.length - 1] = extend(held, item);
    else if (!held || last(held) < first(item)) out.push(item);
  }
  return out;
}

const sameRun = (x: ProjectRun, y: ProjectRun) => x.factories === y.factories && x.remaining === y.remaining && x.repairLevels === y.repairLevels;
const sameOutput = (x: ConstructionOutputRun, y: ConstructionOutputRun) => x.constructionRaw === y.constructionRaw && x.repairRaw === y.repairRaw
  && x.reason === y.reason && x.beforeObservation === y.beforeObservation;

/** One line as two consecutive windows show it: the earlier's counts, then the later's rises (its carried counts are
 * the earlier's), runs and output joined, and what the later shows of it now. */
export function joinProject(a: ConstructionProject, b: ConstructionProject): ConstructionProject {
  const counts = (x: readonly LevelsBuilt[], y: readonly LevelsBuilt[]) => [...x, ...y.filter(levels => !levels.carried)];
  const origin = a.outputOrigin ?? (a.source === 'outputAlone' && b.source === 'census'
    ? { tag: a.tag, building: a.building, state: a.state, toState: a.toState, province: a.province } : b.outputOrigin);
  return { ...b, outputOrigin: origin, repair: a.repair ?? b.repair,
    runs: joined(a.runs, b.runs, run => run.firstHour, run => run.lastHour, sameRun, (x, y) => ({ ...x, lastHour: y.lastHour })),
    outputs: joined(a.outputs, b.outputs, run => run.from, run => run.to, sameOutput, (x, y) => ({ ...x, to: y.to })),
    built: counts(a.built, b.built), unaccounted: counts(a.unaccounted, b.unaccounted), possible: counts(a.possible, b.possible), added: counts(a.added, b.added),
    end: b.end ?? a.end };
}

/** The windows held at the window `current`, joined: a line several windows list is one line. */
export function mergeWindows(manifest: ConstructionLogManifestReply, windows: ReadonlyMap<number, ConstructionLogReply>, current: number): ConstructionLog {
  let held = current;
  while (held > 0 && windows.has(held - 1)) held--;
  const projects = new Map<string, ConstructionProject>();
  const states = new Map<number, string | null>();
  for (const index of [...windows.keys()].filter(index => index >= held).sort((a, b) => a - b)) {
    const reply = windows.get(index)!;
    for (const project of reply.projects) {
      const before = projects.get(project.id);
      projects.set(project.id, before ? joinProject(before, project) : project);
    }
    for (const state of reply.states) if (state.name != null || !states.has(state.id)) states.set(state.id, state.name);
  }
  return { manifest, windows, current, held, projects: [...projects.values()].sort(scanOrder),
    types: new Map(manifest.types.map(type => [type.id, type])),
    states: stateNamesOf([...states].map(([id, name]) => ({ id, name }))) };
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
  if (current < 0 || count === 0) return [];
  const lower = Math.max(0, reach && reach.lower <= current && reach.at >= current - 1 ? reach.lower : current - 1);
  return Array.from({ length: Math.min(count - 1, live ? current + 1 : current) - lower + 1 }, (_, index) => lower + index);
}

/** Where a project stands at an hour, as far as the reads by then show. */
export type Status = 'building' | 'unread' | 'observed' | 'finished' | 'removed' | 'ended';

/** What a project has done by an hour, per stretch: built new levels, repaired, or
 * repaired and then built the new levels queued behind the repair. */
export type ProjectKind = 'new' | 'repair' | 'repairThenNew' | 'unknown';

export interface ProjectAt {
  project: ConstructionProject;
  status: Status;
  /** Work before its first read was not recorded; counts are lower bounds. */
  earlierUnknown: boolean;
  /** Levels (railway: segments) its reads by then show built. */
  built: number;
  /** Levels its reads by then show it lost where the recording cannot tell built from dropped. */
  unknown: number;
  /** Extra levels it may have both built and had added, known only by the read. */
  possible: number;
  /** All its levels as then known: built, unaccounted, added and still to build or repair. */
  levels: number;
  /** Levels its reads by then show added to it (queued more, or new damage). */
  added: number;
  /** What it did by then, per stretch, and whether it is repairing now. */
  kind: ProjectKind;
  repairing: boolean;
  /** The read whose values hold at the hour; null between a gap and the next read, or once it ended. */
  run: ProjectRun | null;
  /** Its last read at or before the hour. */
  last: ProjectRun | null;
  /** Its latest change by then: when it ended, a level was built, or it started. */
  changed: number;
  /** That hour is an upper bound: only the read shows it. */
  changedBy: boolean;
  /** What is known of it by then, so a row that changes gets a new identity, whichever windows are held. */
  stage: number;
}

/** A project at `hour`: null before its first read. Nothing shows before the read that establishes it (`knownAt`). */
export function projectAt(project: ConstructionProject, hour: number): ProjectAt | null {
  if (hour < project.firstRead) return null;
  if (project.outputOrigin && (project.censusRead == null || hour < project.censusRead)) {
    project = { ...project, ...project.outputOrigin, source: 'outputAlone', censusRead: null,
      repair: null, costRaw: null, createdHour: null, queueOrdinal: null, routeProvinces: null, baseLevel: null, targetLevel: null, earlierUnknown: true };
  }
  const sum = (list: readonly LevelsBuilt[], until: number) => list.reduce((total, levels) => total + (levels.knownAt <= until ? levels.count : 0), 0);
  const runs = project.runs.filter(run => run.firstHour <= hour);
  const last = runs.at(-1) ?? null;
  const end = project.end && project.end.knownAt <= hour ? project.end : null;
  const run = !end && last && last.lastHour >= hour ? last : null;
  const status: Status = !end ? run ? 'building' : project.source === 'outputAlone' ? 'observed' : 'unread' : end.kind === 'finished' ? 'finished' : end.kind === 'removed' ? 'removed' : 'ended';
  const count = sum(project.built, hour);
  const unknown = sum(project.unaccounted, hour);
  const possible = sum(project.possible, hour);
  // Its last read's levels still to build and to repair, with what it had built or lost by that read. Levels known
  // after it (at its ending) came out of those, so they are not counted twice, except what the ending shows added.
  const readHour = Math.min(hour, last?.lastHour ?? project.firstRead);
  const before = sum(project.built, readHour) + sum(project.unaccounted, readHour);
  const addedLater = sum(project.added, hour) - sum(project.added, readHour);
  const levels = before + Math.max(0, last?.remaining ?? 0) + Math.max(0, last?.repairLevels ?? 0) + addedLater;
  const known = [...project.built, ...project.unaccounted, ...project.possible].filter(levels => levels.knownAt <= hour);
  const events = known.filter(levels => !levels.carried);
  // A stretch repairs while it has levels to repair (the engine's repair branch), else builds its remaining new levels.
  const repaired = runs.some(item => item.repairLevels > 0) || known.some(levels => levels.repair)
    || (project.firstRepairTick != null && project.firstRepairTick <= hour);
  // New levels queued behind a repair make it a repair, then new, from the start.
  const builtNew = runs.some(item => item.remaining > 0) || known.some(levels => !levels.repair);
  const kind: ProjectKind = project.source === 'outputAlone' ? 'unknown' : repaired && builtNew ? 'repairThenNew' : repaired ? 'repair' : 'new';
  const repairing = run != null && run.repairLevels > 0;
  // The latest change: the latest read to confirm one, and of the changes one read confirms, the latest dated.
  const dated = (levels: LevelsBuilt) => levels.hour ?? levels.knownAt;
  const latest = events.reduce<LevelsBuilt | null>((found, levels) => !found || levels.knownAt > found.knownAt
    || (levels.knownAt === found.knownAt && dated(levels) > dated(found)) ? levels : found, null);
  const [changed, changedBy] = end ? [end.hour ?? end.knownAt, end.hour == null]
    : latest ? [latest.hour ?? latest.knownAt, latest.hour == null] : [project.createdHour ?? project.firstRead, project.createdHour == null];
  return { project, status, earlierUnknown: project.earlierUnknown, built: count, unknown, possible, levels, added: sum(project.added, hour), kind, repairing, run, last, changed, changedBy,
    stage: count + unknown + possible + (end ? 1 : 0) + (project.outputOrigin && project.source === 'census' ? 1 : 0) };
}

/** A project's building type in the manifest. */
export const projectType = (log: ConstructionLog, project: ConstructionProject) => project.building == null ? undefined : log.types.get(project.building);

/** The building types this campaign's lines name, as the game names and pictures them, by name: the choices of the
 * Building types setting and chips. */
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

/** Whether a project is still in the queue at its hour, as far as the reads show: building or repairing, or between
 * a gap and its next read. */
export const inQueue = (at: ProjectAt) => at.status === 'building' || at.status === 'unread';

/** The log's rows at `hour`: every project read by then that passes the filter, newest first by its latest change (or
 * its start), back to the windows held. A row is keyed by its project and, ordered by change, by what is known of it,
 * so a project that reaches a new stage enters again at the top. */
export function logRows(log: ConstructionLog, hour: number, filter: LogFilter, order: 'change' | 'start'): ReadonlyArray<LogRow<ProjectAt>> {
  const row = projectRows(log, filter, order);
  const rows: LogRow<ProjectAt>[] = [];
  for (const project of log.projects) {
    const at = row(project, hour);
    if (at) rows.push(at);
  }
  return newestFirst(rows);
}

/** A project's row at an hour as `logRows` lists it, null where the filter leaves it out. */
function projectRows(log: ConstructionLog, filter: LogFilter, order: 'change' | 'start') {
  const types = new Set(filter.types);
  const from = Math.max(filter.from ?? -Infinity, log.held > 0 ? log.manifest.windows[log.held].firstHour : -Infinity);
  return (project: ConstructionProject, hour: number): LogRow<ProjectAt> | null => {
    const at = projectAt(project, hour);
    if (!at) return null;
    if (types.size && !types.has(projectType(log, at.project)?.token ?? '')) return null;
    if (!filter.country(at.project.tag, project.firstRead)) return null;
    if (filter.projects !== 'all' && at.kind === 'unknown') return null;
    // By what it did by then: a repair that went on to build new levels is both.
    if (filter.projects === 'new' && at.kind === 'repair') return null;
    if (filter.projects === 'repair' && at.kind === 'new') return null;
    if (filter.queue && !inQueue(at)) return null;
    const date = order === 'start' ? at.project.createdHour ?? project.firstRead : at.changed;
    if (date < from) return null;
    return { key: order === 'start' ? project.id : `${project.id}:${at.stage}`, id: project.id, hour: date, by: order === 'change' && at.changedBy, item: at };
  };
}

/** What a row renders beside its painted factories and levels: a project whose state this changes gets a state of its
 * own from that hour (`logStatesBy`). */
const rowState = (row: LogRow<ProjectAt>) => {
  const at = row.item, project = at.project;
  return JSON.stringify([row.hour, row.by ?? false, at.status, at.kind, at.repairing, at.earlierUnknown, project.source, project.tag, project.building,
    project.state, project.toState, project.province]);
};

/** The hours a line's reads change its row's shape: a gap opening or closing, repair work or new levels starting or
 * stopping. Its factories and levels between them are painted (`liveProjectTexts`), never rendered for. */
export function* shapeChanges(runs: readonly ProjectRun[]): Generator<number> {
  let previous: ProjectRun | null = null;
  for (const run of runs) {
    const joins = previous != null && previous.lastHour + 1 === run.firstHour;
    if (previous && !joins) yield previous.lastHour + 1;
    if (!previous || !joins || (previous.repairLevels > 0) !== (run.repairLevels > 0) || (previous.remaining > 0) !== (run.remaining > 0)) yield run.firstHour;
    previous = run;
  }
  if (previous) yield previous.lastHour + 1;
}

/** The hours a line's row may change: its first and census reads, repair starting, its ending read, a shape change and
 * each read of levels. */
function* projectChanges(project: ConstructionProject): Generator<number> {
  yield project.firstRead;
  if (project.censusRead != null) yield project.censusRead;
  if (project.firstRepairTick != null) yield project.firstRepairTick;
  if (project.end) yield project.end.knownAt;
  yield* shapeChanges(project.runs);
  for (const levels of [project.built, project.unaccounted, project.possible, project.added]) for (const level of levels) yield level.knownAt;
}

/** The log as it plays: the rows at each hour (`at`), made again only at the hours a line the filter keeps changes
 * shape (`span`); and over a block of hours, each row's states (`states`), a line read again only at its own changes. */
export function constructionPlayback(log: ConstructionLog, filter: LogFilter, order: 'change' | 'start') {
  const types = new Set(filter.types);
  const typeToken = (building: number | null) => building == null ? '' : log.types.get(building)?.token ?? '';
  const selected = { ...log, projects: log.projects.filter(project => {
    const matches = (tag: string, building: number | null) => (!types.size || types.has(typeToken(building))) && filter.country(tag, project.firstRead);
    return matches(project.tag, project.building) || (project.outputOrigin != null && matches(project.outputOrigin.tag, project.outputOrigin.building));
  }) };
  const changing = new Map<number, ConstructionProject[]>();
  for (const project of selected.projects) for (const hour of new Set(projectChanges(project))) {
    const projects = changing.get(hour);
    if (projects) projects.push(project); else changing.set(hour, [project]);
  }
  const hours = Float64Array.from(changing.keys()).sort();
  const playback = playbackRows(hours, hour => logRows(selected, hour, filter, order));
  const row = projectRows(selected, filter, order);
  // A row's place in `newestFirst`: its date, then its line's place in the log.
  const place = new Map(selected.projects.map((project, index) => [project.id, index]));
  const byId = new Map(selected.projects.map(project => [project.id, project]));
  const listed = (a: LogRow<ProjectAt>, b: LogRow<ProjectAt>) => b.hour - a.hour || place.get(a.item.project.id)! - place.get(b.item.project.id)!;
  return {
    at: (hour: number) => playback.at(hour),
    span: (hour: number) => playback.span(hour),
    /** Each row's states from `first` through `last`: the rows at `first`, then each line's row again at its changes. */
    states(first: number, last: number): LogRow<ProjectAt>[] {
      let low = 0, high = hours.length;
      while (low < high) { const middle = (low + high) >> 1; if (hours[middle] <= first) low = middle + 1; else high = middle; }
      const block = [first];
      for (let at = low; at < hours.length && hours[at] <= last; at++) block.push(hours[at]);
      return logStatesBy(block, hour => playback.at(hour), hour => changing.get(hour)!.map(project => project.id),
        (id, hour) => row(byId.get(id)!, hour), rowState, listed);
    },
  };
}

/** The countries the drawn rows name, each at its project's first read, and as many hours as they ask for. */
export function rowCountryRequests(drawn: readonly LogRow<ProjectAt>[]): { wanted: NameAt[]; maxHours: number } {
  const wanted = logCountryRequests(drawn, row => ({ hour: row.item.project.firstRead, tags: [row.item.project.tag] }));
  return { wanted, maxHours: Math.max(MAX_NAME_HOURS, new Set(wanted.map(item => item.hour)).size) };
}

/** The status as a row's tag says it. */
export function statusLabel(at: ProjectAt): string {
  switch (at.status) {
    case 'building': return at.repairing ? 'Repairing' : at.kind === 'repairThenNew' ? 'Building, after repair' : 'Building';
    case 'unread': return 'Not recorded';
    case 'observed': return 'Observed';
    case 'finished': return at.kind === 'unknown' ? 'Finished' : at.kind === 'repair' ? 'Repaired' : at.kind === 'repairThenNew' ? 'Repaired and built' : 'Built';
    case 'removed': return 'Removed';
    case 'ended': return 'Left the queue';
  }
}

/** The status pill's tone: building or repairing, finished, or neither. */
export function statusTone(at: ProjectAt): 'accent' | 'positive' | 'neutral' {
  return at.status === 'building' ? 'accent' : at.status === 'finished' ? 'positive' : 'neutral';
}

/** What a project did, as the log's repair marking says it. */
export function kindLabel(kind: ProjectKind): string {
  if (kind === 'unknown') return 'Not recorded';
  return kind === 'repair' ? 'Repair' : kind === 'repairThenNew' ? 'Repair, then new' : 'New';
}

/** The levels part of a row: "×3" for several, "2 of 3" while building several, none for one. Levels the recording
 * cannot tell built from dropped keep the count a range ("1–2 of 2") at every status. */
export function levelsLabel(at: ProjectAt): string | null {
  if (at.project.source === 'outputAlone') return null;
  if (at.earlierUnknown || at.unknown > 0 || at.possible > 0) return builtLabel(at);
  if (at.levels <= 1) return null;
  return (at.status === 'building' || at.status === 'unread') && at.built > 0 ? builtLabel(at) : `×${at.levels}`;
}

/** Where the project is: its state (a railway: from one state to another). */
export const placeLabel = (project: ConstructionProject, stateAt: StateNameAt) =>
  placeName({ state: project.state, toState: project.toState, province: project.province, railway: project.kind === 'railway' }, stateAt).label;

/** Where the project is, as a line of its own: a place without a state's name capitalised. */
export function placeTitle(project: ConstructionProject, stateAt: StateNameAt): string {
  const label = placeLabel(project, stateAt);
  return project.state != null || (project.kind === 'railway' && project.toState != null) ? label : label.charAt(0).toUpperCase() + label.slice(1);
}

/** The row read out as one sentence. */
export function describeRow(log: ConstructionLog, row: LogRow<ProjectAt>, country: string): string {
  const at = row.item;
  const levels = levelsLabel(at);
  return `${country}, ${typeName(projectType(log, at.project))}${levels ? ` ${levels}` : ''} in ${placeLabel(at.project, log.states)}, ${statusLabel(at).toLowerCase()}, ${row.by ? 'by ' : ''}${dayLabel(row.hour)}${at.project.source === 'outputAlone' ? ', known from output' : ''}${at.earlierUnknown ? ', earlier progress unknown' : ''}`;
}

/** Levels built as then known, with ranges for work built or dropped unseen and extra levels it may have both built
 * and had added ("1–2 of 2–3"). */
export function builtLabel(at: ProjectAt): string {
  if (at.project.source === 'outputAlone') return 'Not recorded';
  const more = at.earlierUnknown ? '+' : '';
  const extra = at.unknown + at.possible;
  const built = extra ? `${at.built}–${at.built + extra}` : at.built;
  const total = at.possible ? `${at.levels}–${at.levels + at.possible}` : at.levels;
  return `${built}${more} of ${total}${more}`;
}

/** Where a row's Jump to menu can go, in time order: when the project started, its latest change and its ending. */
export function jumpHours(at: ProjectAt, hour: number): number[] {
  const { end } = at.project;
  const ended = end && end.knownAt <= hour ? [end.hour ?? end.knownAt] : [];
  return [...new Set([at.project.createdHour ?? at.project.firstRead, at.changed, ...ended])].sort((a, b) => a - b);
}

/** The factories on a project as the row shows them: at the read in view, or at its last read once it ended; null
 * between a gap and the next read. */
export function factoriesShown(at: ProjectAt): number | null {
  return at.run ? at.run.factories : at.status === 'unread' ? null : at.last?.factories ?? null;
}

/** The factories column of a row: only while the project is in the queue. */
export function rowFactories(at: ProjectAt): { factories: number | null } | null {
  return inQueue(at) ? { factories: factoriesShown(at) } : null;
}

/** What a row paints at `hour`: its levels (`levels`) or its factories (`factories`, "—" where not recorded). */
export type LivePart = 'levels' | 'factories';
export const liveKey = (id: string, part: LivePart) => `${id}:${part}`;

export function projectText(project: ConstructionProject, part: LivePart, hour: number): string {
  const at = projectAt(project, hour);
  if (!at) return '';
  if (part === 'levels') return levelsLabel(at) ?? '';
  const factories = factoriesShown(at);
  return factories == null ? '—' : String(factories);
}

/** The rows' factories and levels at `hour()` as each frame paints them: a text is made once an hour, and only for a row
 * that asks (the rows drawn). */
export function liveProjectTexts(clock: TextClock, hour: () => number, projects: ReadonlyMap<string, ConstructionProject>): LiveTexts {
  let made: { hour: number; texts: Map<string, string> } | null = null;
  return {
    text: key => {
      const at = hour();
      if (made?.hour !== at) made = { hour: at, texts: new Map() };
      let text = made.texts.get(key);
      if (text === undefined) {
        const split = key.lastIndexOf(':');
        const project = projects.get(key.slice(0, split));
        text = project ? projectText(project, key.slice(split + 1) as LivePart, at) : '';
        made.texts.set(key, text);
      }
      return text;
    },
    subscribe: changed => {
      const stops = [clock.subscribeFrame(changed), clock.subscribe(changed)];
      return () => { for (const stop of stops) stop(); };
    },
  };
}

/** The last hour the log holds every line's reads through: the end of the latest window held without a gap from the
 * hour's. A painted hour past it would read lines the log has not been told of yet. */
export function heldThrough(log: ConstructionLog): number {
  let last = log.current;
  while (log.windows.has(last + 1)) last++;
  return log.manifest.windows[last]?.lastHour ?? -Infinity;
}

export type ConstructionPlayback = ReturnType<typeof constructionPlayback>;

/** The hour before which the log drawn ahead at `hour` may draw: the next window's start, where the windows it holds
 * move on, and never past what it holds. */
export function drawLimit(windows: readonly BattleWindow[], log: ConstructionLog, hour: number): number {
  return Math.min(windows[windowAt(windows, hour) + 1]?.firstHour ?? Infinity, heldThrough(log) + 1);
}

/** The factory output a tooltip shows at `hour`: the project's latest by then, known from the read that changed it. */
export function outputAt(project: ConstructionProject, hour: number): (ConstructionOutputRun & { knownAt: number }) | null {
  let run: ConstructionOutputRun | undefined;
  for (let index = project.outputs.length - 1; index >= 0 && !run; index--) if (project.outputs[index].from <= hour) run = project.outputs[index];
  return run ? { ...run, knownAt: run.from } : null;
}

/** The tooltip, fullest first: when it started and ended, its levels, factories and cost, and (fuller) where exactly;
 * its note says what is not known. */
export function projectTooltip(log: ConstructionLog, at: ProjectAt, hour: number, country: string): TooltipContent[] {
  const { project } = at;
  const type = projectType(log, project);
  const end = project.end && project.end.knownAt <= hour ? project.end : null;
  const pending = at.status === 'unread' ? 'Not recorded' : 'Not yet';
  const rows: TooltipRow[] = project.createdHour == null
    ? [{ label: 'Source', value: 'Known from output' }, { label: 'First evidence', value: dayLabel(project.firstRead) }]
    : [{ label: 'Started', value: dayLabel(project.createdHour) }];
  const dated = (risen: number | null, knownAt: number) => risen != null ? dayLabel(risen) : `by ${dayLabel(knownAt)}`;
  if (at.kind === 'repairThenNew') {
    const latest = project.built.filter(levels => levels.repair && !levels.carried && levels.knownAt <= hour)
      .reduce<LevelsBuilt | null>((found, levels) => !found || (levels.hour ?? levels.knownAt) > (found.hour ?? found.knownAt) ? levels : found, null);
    const repaired = end?.kind === 'finished' || (at.run ? at.run.repairLevels <= 0 : end != null && at.last != null && at.last.repairLevels <= 0);
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
  if (project.source === 'census') rows.push(inQueue(at) ? { label: 'Factories', value: factories == null ? 'Not recorded' : String(factories) }
    : { label: 'Factories at its last read', value: at.last == null ? 'Not recorded' : String(at.last.factories) });
  // Factory IC is written in whole numbers; the recorded amounts stay exact.
  const scale = Math.round(Math.log10(log.manifest.costScale));
  if (project.costRaw != null) {
    const cost = formatAmount(project.costRaw, scale, 0);
    rows.push(project.kind === 'railway' ? { label: 'Route cost', value: cost } : { label: `Cost per ${unit}`, value: cost });
  }
  const output = outputAt(project, hour);
  if (output) {
    rows.push({ label: 'Construction output', value: output.constructionRaw == null ? 'Not recorded' : formatAmount(output.constructionRaw, scale, 0) });
    rows.push({ label: 'Repair output', value: output.repairRaw == null ? 'Not recorded' : formatAmount(output.repairRaw, scale, 0) });
  }
  const detail: TooltipRow[] = [];
  if (project.kind === 'railway' && project.source === 'census') {
    detail.push({ label: 'Railway level', value: `${project.baseLevel ?? '?'} → ${project.targetLevel ?? '?'}`, divider: true });
    if (project.routeProvinces != null) detail.push({ label: 'Provinces on the route', value: String(project.routeProvinces) });
  }
  const place = placeName({ state: project.state, toState: project.toState, province: project.province, railway: project.kind === 'railway' }, log.states);
  const notes = [...new Set([place.missing ?? null, output?.reason ? 'Output unavailable.' : null, output?.beforeObservation ? 'Earlier progress unknown.' : null,
    end?.unknown ? 'Completion unknown.' : null, end?.kind === 'removed' ? 'Removed from the queue.' : null,
    at.earlierUnknown ? 'Earlier progress unknown.' : null,
    at.possible ? 'Completion count uncertain.' : null].filter(Boolean))].join(' ');
  const base = { title: typeName(type), icon: type?.icon ?? undefined, subtitle: `${country} · ${place.label}${at.kind === 'new' || at.kind === 'unknown' ? '' : ` · ${kindLabel(at.kind).toLowerCase()}`}`, note: notes || undefined };
  return [{ ...base, rows: [...rows, ...detail] }, { ...base, rows }];
}
