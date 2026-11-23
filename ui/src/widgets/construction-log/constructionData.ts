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
const isOutputRun = (run: ConstructionOutputRun) => typeof run === 'object' && run != null && int(run.from) && int(run.to) && run.from <= run.to
  && [run.constructionRaw, run.repairRaw].every(raw => raw == null || (typeof raw === 'string' && RAW.test(raw)))
  && (run.reason == null || isSpendingGap(run.reason)) && typeof run.beforeObservation === 'boolean'
  && (run.reason == null) === (run.constructionRaw != null && run.repairRaw != null);
const isLevels = (levels: LevelsBuilt) => typeof levels === 'object' && levels != null && optionalInt(levels.hour) && int(levels.knownAt)
  && int(levels.count) && levels.count > 0 && typeof levels.repair === 'boolean' && typeof levels.carried === 'boolean';
const inOrder = <T>(list: readonly T[], first: (item: T) => number, last: (item: T) => number) => list.every((item, index) => index === 0 || last(list[index - 1]) < first(item));
const isHourSpan = (span: unknown): span is [number, number] => Array.isArray(span) && span.length === 2 && int(span[0]) && int(span[1]) && span[0] <= span[1];
const isUnreadSpans = (spans: unknown) => spans === undefined || (Array.isArray(spans) && spans.every(isHourSpan) && inOrder(spans, span => span[0], span => span[1]));
const inSpans = (spans: readonly [number, number][] | undefined, hour: number) => spans?.some(([first, last]) => first <= hour && hour <= last) ?? false;
const isProject = (project: ConstructionProject) => typeof project === 'object' && project != null && typeof project.id === 'string'
  && (project.kind === 'building' || project.kind === 'railway' || project.kind === 'unknown') && typeof project.tag === 'string'
  && (project.source === 'census' || project.source === 'outputAlone' || project.source === 'unknown') && optionalInt(project.censusRead) && optionalInt(project.queueOrdinal)
  && optionalInt(project.building) && (project.repair == null || typeof project.repair === 'boolean')
  && [project.state, project.toState, project.province, project.routeProvinces, project.baseLevel, project.targetLevel, project.createdHour].every(optionalInt)
  && (project.costRaw == null || RAW.test(project.costRaw)) && int(project.firstRead)
  && (project.outputOrigin == null || (typeof project.outputOrigin.tag === 'string'
    && [project.outputOrigin.building, project.outputOrigin.state, project.outputOrigin.toState, project.outputOrigin.province].every(optionalInt)))
  && typeof project.earlierUnknown === 'boolean' && optionalInt(project.firstRepairTick)
  && isUnreadSpans(project.builtUnread)
  && isUnreadSpans(project.repairedUnread)
  && Array.isArray(project.runs) && project.runs.every(isRun) && inOrder(project.runs, run => run.firstHour, run => run.lastHour)
  && [project.built, project.unaccounted, project.possible, project.added].every(list => Array.isArray(list) && list.every(isLevels))
  && Array.isArray(project.outputs) && project.outputs.every(isOutputRun) && inOrder(project.outputs, run => run.from, run => run.to)
  && (project.end == null || (typeof project.end === 'object' && END_KINDS.includes(project.end.kind) && optionalInt(project.end.hour)
    && int(project.end.knownAt) && (project.end.unknown == null || UNKNOWN_ENDS.includes(project.end.unknown))));

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

export function validateConstructionLogWindow(data: unknown): ConstructionLogReply {
  const reply = data as ConstructionLogReply;
  const ok = typeof reply === 'object' && reply != null && int(reply.window) && reply.window >= 0 && isSpan(reply.interval)
    && Array.isArray(reply.projects) && reply.projects.every(isProject) && Array.isArray(reply.states) && reply.states.every(isStateName)
    && strings(reply.warnings);
  if (!ok) throw new Error('The construction log window is malformed.');
  return reply;
}

export function windowAt(windows: readonly BattleWindow[], hour: number): number {
  let low = 0, high = windows.length;
  while (low < high) { const middle = (low + high) >> 1; if (windows[middle].firstHour <= hour) low = middle + 1; else high = middle; }
  return Math.max(0, low - 1);
}

export interface ConstructionLog {
  manifest: ConstructionLogManifestReply;
  windows: ReadonlyMap<number, ConstructionLogReply>;
  current: number;
  held: number;
  projects: readonly ConstructionProject[];
  types: ReadonlyMap<number, ConstructionType>;
  states: StateNameAt;
}

const kindRank = (kind: ConstructionProject['kind']) => kind === 'building' ? 0 : kind === 'railway' ? 1 : 2;
const scanOrder = (a: ConstructionProject, b: ConstructionProject) => a.firstRead - b.firstRead
  || kindRank(a.kind) - kindRank(b.kind) || a.id.length - b.id.length || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);

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

export function joinProject(a: ConstructionProject, b: ConstructionProject): ConstructionProject {
  const counts = (x: readonly LevelsBuilt[], y: readonly LevelsBuilt[]) => {
    const prior = [0, 0];
    for (const levels of x) prior[levels.repair ? 1 : 0] += levels.count;
    const out = [...x];
    for (const levels of y) {
      const side = levels.repair ? 1 : 0;
      if (!levels.carried) {
        prior[side] += levels.count;
        out.push(levels);
        continue;
      }
      const delta = levels.count - prior[side];
      if (delta > 0) {
        prior[side] += delta;
        out.push(prior[side] === delta ? levels : { ...levels, count: delta, carried: false });
      }
    }
    return out;
  };
  const origin = a.outputOrigin ?? (a.source === 'outputAlone' && b.source === 'census'
    ? { tag: a.tag, building: a.building, state: a.state, toState: a.toState, province: a.province } : b.outputOrigin);
  const mergeSpans = (x?: readonly [number, number][], y?: readonly [number, number][]) =>
    joined(x ?? [], y ?? [], span => span[0], span => span[1], () => true, (first, second): [number, number] => [first[0], second[1]]);
  const builtUnread = mergeSpans(a.builtUnread, b.builtUnread);
  const repairedUnread = mergeSpans(a.repairedUnread, b.repairedUnread);
  return { ...b, kind: b.kind === 'unknown' ? a.kind : b.kind, source: b.source === 'unknown' ? a.source : b.source,
    outputOrigin: origin, repair: a.repair ?? b.repair,
    ...(builtUnread.length ? { builtUnread } : { builtUnread: undefined }),
    ...(repairedUnread.length ? { repairedUnread } : { repairedUnread: undefined }),
    runs: joined(a.runs, b.runs, run => run.firstHour, run => run.lastHour, sameRun, (x, y) => ({ ...x, lastHour: y.lastHour })),
    outputs: joined(a.outputs, b.outputs, run => run.from, run => run.to, sameOutput, (x, y) => ({ ...x, to: y.to })),
    built: counts(a.built, b.built), unaccounted: counts(a.unaccounted, b.unaccounted), possible: counts(a.possible, b.possible), added: counts(a.added, b.added),
    end: b.end ?? a.end };
}

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

export const logComplete = (log: ConstructionLog, from: number | null) =>
  log.held === 0 || (from != null && log.manifest.windows[log.held].firstHour <= from);

export function logReach(log: ConstructionLog, rows: readonly LogRow<ProjectAt>[], drawn: readonly LogRow<ProjectAt>[], from: number | null): number {
  const last = drawn.at(-1);
  const needed = (last ? rows.findIndex(row => row.key === last.key) + 1 : 0) + Math.max(1, drawn.length);
  if (needed <= rows.length) return Math.min(log.current, windowAt(log.manifest.windows, rows[needed - 1].hour));
  return logComplete(log, from) ? log.held : Math.max(0, log.held - (log.current - log.held + 1));
}

export function constructionWindows(count: number, current: number, live: boolean, reach: { at: number; lower: number } | null): number[] {
  if (current < 0 || count === 0) return [];
  const lower = Math.max(0, reach && reach.lower <= current && reach.at >= current - 1 ? reach.lower : current - 1);
  return Array.from({ length: Math.min(count - 1, live ? current + 1 : current) - lower + 1 }, (_, index) => lower + index);
}

export type Status = 'building' | 'unread' | 'observed' | 'finished' | 'removed' | 'ended';

export type ProjectKind = 'new' | 'repair' | 'repairThenNew' | 'unknown';

export interface ProjectAt {
  project: ConstructionProject;
  status: Status;
  earlierUnknown: boolean;
  countersUnread: boolean;
  built: number;
  unknown: number;
  possible: number;
  levels: number;
  added: number;
  kind: ProjectKind;
  repairing: boolean;
  run: ProjectRun | null;
  last: ProjectRun | null;
  changed: number;
  changedBy: boolean;
  stage: number;
}

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
  const readHour = Math.min(hour, last?.lastHour ?? project.firstRead);
  const before = sum(project.built, readHour) + sum(project.unaccounted, readHour);
  const addedLater = sum(project.added, hour) - sum(project.added, readHour);
  const levels = before + Math.max(0, last?.remaining ?? 0) + Math.max(0, last?.repairLevels ?? 0) + addedLater;
  const known = [...project.built, ...project.unaccounted, ...project.possible].filter(levels => levels.knownAt <= hour);
  const events = known.filter(levels => !levels.carried);
  const repaired = runs.some(item => item.repairLevels > 0) || known.some(levels => levels.repair)
    || (project.firstRepairTick != null && project.firstRepairTick <= hour);
  const builtNew = runs.some(item => item.remaining > 0) || known.some(levels => !levels.repair);
  const kind: ProjectKind = project.source === 'outputAlone' || project.source === 'unknown' ? 'unknown'
    : repaired && builtNew ? 'repairThenNew' : repaired ? 'repair' : builtNew || project.repair === false ? 'new' : 'unknown';
  const builtUnread = inSpans(project.builtUnread, hour);
  const repairedUnread = inSpans(project.repairedUnread, hour);
  const countersUnread = kind === 'repair' ? repairedUnread : kind === 'new' ? builtUnread : builtUnread || repairedUnread;
  const repairing = run != null && run.repairLevels > 0;
  const dated = (levels: LevelsBuilt) => levels.hour ?? levels.knownAt;
  const latest = events.reduce<LevelsBuilt | null>((found, levels) => !found || levels.knownAt > found.knownAt
    || (levels.knownAt === found.knownAt && dated(levels) > dated(found)) ? levels : found, null);
  const [changed, changedBy] = end ? [end.hour ?? end.knownAt, end.hour == null]
    : latest ? [latest.hour ?? latest.knownAt, latest.hour == null] : [project.createdHour ?? project.firstRead, project.createdHour == null];
  return { project, status, earlierUnknown: project.earlierUnknown, countersUnread, built: count, unknown, possible, levels, added: sum(project.added, hour), kind, repairing, run, last, changed, changedBy,
    stage: count + unknown + possible + (end ? 1 : 0) + (project.outputOrigin && project.source === 'census' ? 1 : 0) };
}

export const projectType = (log: ConstructionLog, project: ConstructionProject) => project.building == null ? undefined : log.types.get(project.building);

export function buildingChoices(data: ConstructionLogManifestReply | undefined): { value: string; label: string; icon?: string | null }[] {
  return (data?.types ?? []).flatMap(type => type.token ? [{ value: type.token, label: typeName(type), icon: type.icon }] : [])
    .sort((a, b) => a.label.localeCompare(b.label));
}

export interface LogFilter {
  types: readonly string[];
  projects: 'all' | 'new' | 'repair';
  queue?: boolean;
  country: (tag: string, hour: number) => boolean;
  from: number | null;
}

export const inQueue = (at: ProjectAt) => at.status === 'building' || at.status === 'unread';

export function logRows(log: ConstructionLog, hour: number, filter: LogFilter, order: 'change' | 'start'): ReadonlyArray<LogRow<ProjectAt>> {
  const row = projectRows(log, filter, order);
  const rows: LogRow<ProjectAt>[] = [];
  for (const project of log.projects) {
    const at = row(project, hour);
    if (at) rows.push(at);
  }
  return newestFirst(rows);
}

function projectRows(log: ConstructionLog, filter: LogFilter, order: 'change' | 'start') {
  const types = new Set(filter.types);
  const from = Math.max(filter.from ?? -Infinity, log.held > 0 ? log.manifest.windows[log.held].firstHour : -Infinity);
  return (project: ConstructionProject, hour: number): LogRow<ProjectAt> | null => {
    const at = projectAt(project, hour);
    if (!at) return null;
    if (types.size && !types.has(projectType(log, at.project)?.token ?? '')) return null;
    if (!filter.country(at.project.tag, project.firstRead)) return null;
    if (filter.projects !== 'all' && at.kind === 'unknown') return null;
    if (filter.projects === 'new' && at.kind === 'repair') return null;
    if (filter.projects === 'repair' && at.kind === 'new') return null;
    if (filter.queue && !inQueue(at)) return null;
    const date = order === 'start' ? at.project.createdHour ?? project.firstRead : at.changed;
    if (date < from) return null;
    return { key: order === 'start' ? project.id : `${project.id}:${at.stage}`, id: project.id, hour: date, by: order === 'change' && at.changedBy, item: at };
  };
}

const rowState = (row: LogRow<ProjectAt>) => {
  const at = row.item, project = at.project;
  return JSON.stringify([row.hour, row.by ?? false, at.status, at.kind, at.repairing, at.earlierUnknown, at.countersUnread, project.source, project.tag, project.building,
    project.state, project.toState, project.province]);
};

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

function* projectChanges(project: ConstructionProject): Generator<number> {
  yield project.firstRead;
  if (project.censusRead != null) yield project.censusRead;
  if (project.firstRepairTick != null) yield project.firstRepairTick;
  if (project.end) yield project.end.knownAt;
  yield* shapeChanges(project.runs);
  for (const levels of [project.built, project.unaccounted, project.possible, project.added]) for (const level of levels) yield level.knownAt;
  for (const [first, last] of project.builtUnread ?? []) { yield first; yield last + 1; }
  for (const [first, last] of project.repairedUnread ?? []) { yield first; yield last + 1; }
}

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
  const place = new Map(selected.projects.map((project, index) => [project.id, index]));
  const byId = new Map(selected.projects.map(project => [project.id, project]));
  const listed = (a: LogRow<ProjectAt>, b: LogRow<ProjectAt>) => b.hour - a.hour || place.get(a.item.project.id)! - place.get(b.item.project.id)!;
  return {
    at: (hour: number) => playback.at(hour),
    span: (hour: number) => playback.span(hour),
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

export function rowCountryRequests(drawn: readonly LogRow<ProjectAt>[]): { wanted: NameAt[]; maxHours: number } {
  const wanted = logCountryRequests(drawn, row => ({ hour: row.item.project.firstRead, tags: [row.item.project.tag] }));
  return { wanted, maxHours: Math.max(MAX_NAME_HOURS, new Set(wanted.map(item => item.hour)).size) };
}

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

export function statusTone(at: ProjectAt): 'accent' | 'positive' | 'neutral' {
  return at.status === 'building' ? 'accent' : at.status === 'finished' ? 'positive' : 'neutral';
}

export function kindLabel(kind: ProjectKind): string {
  if (kind === 'unknown') return 'Not recorded';
  return kind === 'repair' ? 'Repair' : kind === 'repairThenNew' ? 'Repair, then new' : 'New';
}

export function levelsLabel(at: ProjectAt): string | null {
  if (at.project.source === 'outputAlone' || at.project.source === 'unknown') return null;
  if (at.countersUnread || at.earlierUnknown || at.unknown > 0 || at.possible > 0) return builtLabel(at);
  if (at.levels <= 1) return null;
  return (at.status === 'building' || at.status === 'unread') && at.built > 0 ? builtLabel(at) : `×${at.levels}`;
}

export const placeLabel = (project: ConstructionProject, stateAt: StateNameAt) =>
  placeName({ state: project.state, toState: project.toState, province: project.province, railway: project.kind === 'railway' }, stateAt).label;

export function placeTitle(project: ConstructionProject, stateAt: StateNameAt): string {
  const label = placeLabel(project, stateAt);
  return project.state != null || (project.kind === 'railway' && project.toState != null) ? label : label.charAt(0).toUpperCase() + label.slice(1);
}

export function describeRow(log: ConstructionLog, row: LogRow<ProjectAt>, country: string): string {
  const at = row.item;
  const levels = levelsLabel(at);
  return `${country}, ${typeName(projectType(log, at.project))}${levels ? ` ${levels}` : ''} in ${placeLabel(at.project, log.states)}, ${statusLabel(at).toLowerCase()}, ${row.by ? 'by ' : ''}${dayLabel(row.hour)}${at.project.source === 'outputAlone' ? ', known from output' : ''}${at.earlierUnknown ? ', earlier progress unknown' : ''}`;
}

export function builtLabel(at: ProjectAt): string {
  if (at.project.source === 'outputAlone' || at.project.source === 'unknown') return 'Not recorded';
  if (at.countersUnread && at.built === 0 && at.unknown === 0 && at.possible === 0) return 'Not recorded';
  const more = at.earlierUnknown || at.countersUnread ? '+' : '';
  const extra = at.unknown + at.possible;
  const built = extra ? `${at.built}–${at.built + extra}` : at.built;
  const total = at.possible ? `${at.levels}–${at.levels + at.possible}` : at.levels;
  return `${built}${more} of ${total}${more}`;
}

export function jumpHours(at: ProjectAt, hour: number): number[] {
  const { end } = at.project;
  const ended = end && end.knownAt <= hour ? [end.hour ?? end.knownAt] : [];
  return [...new Set([at.project.createdHour ?? at.project.firstRead, at.changed, ...ended])].sort((a, b) => a - b);
}

export function factoriesShown(at: ProjectAt): number | null {
  return at.run ? at.run.factories : at.status === 'unread' ? null : at.last?.factories ?? null;
}

export function rowFactories(at: ProjectAt): { factories: number | null } | null {
  return inQueue(at) ? { factories: factoriesShown(at) } : null;
}

export type LivePart = 'levels' | 'factories';
export const liveKey = (id: string, part: LivePart) => `${id}:${part}`;

export function projectText(project: ConstructionProject, part: LivePart, hour: number): string {
  const at = projectAt(project, hour);
  if (!at) return '';
  if (part === 'levels') return levelsLabel(at) ?? '';
  const factories = factoriesShown(at);
  return factories == null ? '—' : String(factories);
}

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

export function heldThrough(log: ConstructionLog): number {
  let last = log.current;
  while (log.windows.has(last + 1)) last++;
  return log.manifest.windows[last]?.lastHour ?? -Infinity;
}

export type ConstructionPlayback = ReturnType<typeof constructionPlayback>;

export function drawLimit(windows: readonly BattleWindow[], log: ConstructionLog, hour: number): number {
  return Math.min(windows[windowAt(windows, hour) + 1]?.firstHour ?? Infinity, heldThrough(log) + 1);
}

export function outputAt(project: ConstructionProject, hour: number): (ConstructionOutputRun & { knownAt: number }) | null {
  let run: ConstructionOutputRun | undefined;
  for (let index = project.outputs.length - 1; index >= 0 && !run; index--) if (project.outputs[index].from <= hour) run = project.outputs[index];
  return run ? { ...run, knownAt: run.from } : null;
}

export function projectTooltip(log: ConstructionLog, at: ProjectAt, hour: number, country: string): TooltipContent[] {
  const { project } = at;
  const type = projectType(log, project);
  const end = project.end && project.end.knownAt <= hour ? project.end : null;
  const pending = at.status === 'unread' ? 'Not recorded' : 'Not yet';
  const rows: TooltipRow[] = project.createdHour != null
    ? [{ label: 'Started', value: dayLabel(project.createdHour) }]
    : project.source === 'census'
      ? [{ label: 'Started', value: 'Not recorded', tone: 'muted' }, { label: 'First seen', value: dayLabel(project.firstRead) }]
      : [{ label: 'Source', value: project.source === 'unknown' ? 'Not recorded' : 'Known from output', ...(project.source === 'unknown' ? { tone: 'muted' as const } : {}) }, { label: 'First evidence', value: dayLabel(project.firstRead) }];
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
    : project.kind === 'railway' ? 'Segments built' : project.kind === 'unknown' ? 'Progress' : 'Levels built';
  const builtValue = builtLabel(at);
  rows.push({ label: done, value: builtValue, ...(builtValue === 'Not recorded' ? { tone: 'muted' as const } : {}) });
  if (at.added || at.possible) {
    const added = at.possible ? `${at.added}–${at.added + at.possible}` : at.added;
    rows.push({ label: 'Added while under way', value: `${added} ${unit}${at.added + at.possible === 1 ? '' : 's'}`, tone: 'muted' });
  }
  if (at.unknown) rows.push({ label: 'Built or dropped, not recorded', value: `${at.unknown} ${unit}${at.unknown === 1 ? '' : 's'}`, tone: 'muted' });
  const factories = factoriesShown(at);
  if (project.source === 'census') rows.push(inQueue(at) ? { label: 'Factories', value: factories == null ? 'Not recorded' : String(factories) }
    : { label: 'Factories at its last read', value: at.last == null ? 'Not recorded' : String(at.last.factories) });
  const scale = Math.round(Math.log10(log.manifest.costScale));
  if (project.costRaw != null) {
    const cost = formatAmount(project.costRaw, scale, 0);
    rows.push(project.kind === 'railway' ? { label: 'Route cost', value: cost } : project.kind === 'unknown' ? { label: 'Cost', value: cost } : { label: `Cost per ${unit}`, value: cost });
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
