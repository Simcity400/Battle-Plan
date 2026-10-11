import { contentTitle } from '../../dashboard/contentTitles.ts';
import type { EspionageLogReply, EspionageOperation, EspionageOperationRun, EspionageOperationType } from '../../generated/dashboard';
import { isOrderedSpans, isSpan, runAt, spansCovered } from '../../charts/series.ts';
import type { LogRow } from '../../charts/log/logData';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { object, int, unsigned32, unsigned64 } from '../../dashboard/validation.ts';
import { drawableIcon } from '../../dashboard/artwork.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { entryAt, readAt } from '../../dashboard/reads.ts';
import { setToken, storableToken, UNSTORABLE } from '../../dashboard/options.ts';

const nullableText = (v: unknown) => v === null || typeof v === 'string';
const descriptionRef = (v: unknown) => object(v) && int(v.hour) && unsigned64(v.id) && unsigned32(v.owner) && (v.sequence === null || unsigned64(v.sequence));
const nullableInt = (v: unknown) => v === null || int(v);
const type = (v: unknown) => object(v) && typeof v.key === 'string' && v.key.length > 0 && nullableText(v.name) && nullableText(v.iconMissing) && 'icon' in v;
const run = (v: unknown) => isSpan(v) && int(v.readHour) && v.readHour <= v.firstHour
  && (v.preparationRatio === null || (int(v.preparationRatio) && v.preparationRatio >= 0 && v.preparationRatio <= 100_000))
  && nullableInt(v.elapsedDays) && nullableInt(v.startHour) && nullableInt(v.currentPhase) && typeof v.terminal === 'boolean'
  && typeof v.autoCommence === 'boolean'
  && nullableText(v.ownerTag) && nullableText(v.targetTag) && nullableText(v.typeKey)
  && unsigned32(v.phaseCount) && typeof v.autoRepeat === 'boolean' && int(v.durationDays) && unsigned32(v.assignedSlots)
  && (v.operatives === null || (Array.isArray(v.operatives) && v.operatives.every(id => typeof id === 'string')))
  && (v.operativeNames === null || (Array.isArray(v.operativeNames) && Array.isArray(v.operatives)
    && v.operativeNames.length === v.operatives.length && v.operativeNames.every((name, index) => object(name)
      && name.id === (v.operatives as unknown[])[index] && nullableText(name.name) && nullableText(name.unavailable))));

/** Types are content identities, not a fixed catalogue; bad art keeps its named placeholder. */
export function validateEspionageLogReply(data: unknown): EspionageLogReply {
  const reads = object(data) && Array.isArray(data.readHours) ? data.readHours : null;
  if (!object(data) || typeof data.branchId !== 'string' || !int(data.firstHour) || !int(data.lastHour) || data.firstHour > data.lastHour
    || !(data.window === null || (int(data.window) && data.window >= 0)) || !isOrderedSpans(data.windows)
    || !isOrderedSpans(data.available) || !reads || !reads.every((hour, i) => int(hour) && (!i || reads[i - 1] < hour))
    || !isOrderedSpans(data.completionsAvailable) || !nullableText(data.completionsUnrecorded)
    || !Array.isArray(data.types) || !data.types.every(type) || new Set(data.types.map(item => item.key)).size !== data.types.length
    || !Array.isArray(data.operations) || !data.operations.every(op => object(op) && typeof op.id === 'string' && int(op.firstRead)
      && unsigned32(op.definitionToken)
      && nullableInt(op.startHour) && nullableInt(op.startKnownAt) && ((op.startHour === null) === (op.startKnownAt === null))
      && isOrderedSpans(op.runs) && op.runs.every(run) && (!op.runs.length || op.runs[0].firstHour >= op.firstRead)
      && (op.end === null || (object(op.end) && int(op.end.byHour) && int(op.end.lastSeen) && op.end.byHour >= op.end.lastSeen
        && nullableInt(op.end.completionHour) && (op.end.completionSequence === null || unsigned64(op.end.completionSequence))
        && ((op.end.completionHour === null) === (op.end.completionSequence === null))
        && (op.end.completionHour === null || op.end.completionHour <= op.end.byHour)
        && nullableText(op.end.typeKey)
        && (op.end.outcome === null || (object(op.end.outcome) && typeof op.end.outcome.bonus === 'boolean'
          && typeof op.end.outcome.detected === 'boolean' && typeof op.end.outcome.recordedLanguage === 'string'
          && (op.end.outcome.descriptionRef === null || descriptionRef(op.end.outcome.descriptionRef))
          && nullableText(op.end.outcome.descriptionUnrecorded) && unsigned64(op.end.outcome.descriptionRequiredBytes)
          && ((op.end.outcome.descriptionRef === null) !== (op.end.outcome.descriptionUnrecorded === null))))
        && nullableText(op.end.outcomeUnrecorded) && ((op.end.outcome === null) !== (op.end.outcomeUnrecorded === null))
        && nullableText(op.end.ownerTag) && nullableText(op.end.targetTag)
        && (op.end.reason === 'completed' || op.end.reason === 'disappeared')))
      && (op.runs.length > 0 || (op.end !== null && op.end.completionHour !== null)))
    || new Set(data.operations.map(op => op.id)).size !== data.operations.length
    || !Array.isArray(data.warnings) || !data.warnings.every(warning => typeof warning === 'string') || !nullableText(data.unavailable)) {
    throw new Error('The espionage operations reply is malformed.');
  }
  const reply = data as unknown as EspionageLogReply;
  if (reply.window === null && (reply.operations.length || reply.readHours.length)) throw new Error('The espionage manifest contains detail history.');
  if (reply.window !== null && reply.windows.length) throw new Error('The espionage detail contains a manifest.');
  const types = new Set(reply.types.map(item => item.key));
  for (const operation of reply.operations) {
    if (!spansCovered(operation.runs, reply.available)) throw new Error('The espionage operations reply crosses unobserved hours.');
    if (operation.end && operation.end.completionHour !== null && !runAt(reply.completionsAvailable, operation.end.byHour)) {
      throw new Error('The espionage operations reply has an unrecorded completion read.');
    }
    const reference = operation.end?.outcome?.descriptionRef;
    if (reference && (reference.hour !== operation.end?.byHour || reference.sequence !== operation.end?.completionSequence
      || reference.id !== operation.id.split(':')[0])) throw new Error('The espionage description does not name its ending read.');
    for (const run of [...operation.runs, ...(operation.end ? [operation.end] : [])]) {
      if (run.typeKey !== null && !types.has(run.typeKey)) throw new Error('The espionage operations reply names an unlisted operation type.');
    }
  }
  for (const item of reply.types) {
    const original = item.icon;
    item.icon = drawableIcon(original);
    if (original !== null && item.icon === null) {
      item.iconMissing = `Artwork refused for operation type ${item.key}: not a drawable bounded PNG`;
      reply.warnings.push(`${item.iconMissing} (1)`);
    }
  }
  return reply;
}
export const operationName = (type: EspionageOperationType | undefined) => contentTitle(type?.name, type?.key, 'Operation name unavailable');
export function operationChoices(reply: EspionageLogReply | undefined) {
  return (reply?.types ?? []).map(type => ({ value: setToken(type.key), label: operationName(type), icon: type.icon,
    unavailable: storableToken(setToken(type.key)) ? undefined : UNSTORABLE }))
    .sort((a, b) => a.label.localeCompare(b.label));
}
export interface OperationAt {
  operation: EspionageOperation;
  windowIndex?: number;
  /** Null in a gap. `last` is identity/history only there, never a current measurement. */
  run: EspionageOperationRun | null;
  last: EspionageOperationRun | null;
  typeKey: string | null;
  readHour: number | null;
  state: 'preparing' | 'running' | 'completed' | 'disappeared' | 'unread';
  startHour: number | null;
  ownerTag: string | null;
  targetTag: string | null;
  countryHour: number;
}
export function operationAt(reply: EspionageLogReply, operation: EspionageOperation, hour: number): OperationAt | null {
  if (hour < operation.firstRead) return null;
  const last = entryAt(operation.runs, hour);
  const end = operation.end && operation.end.byHour <= hour ? operation.end : null;
  if (!last && !end) return null;
  const current = end ? runAt(operation.runs, end.lastSeen) : runAt(operation.runs, hour);
  const state = end ? end.reason === 'completed' ? 'completed' : 'disappeared'
    : !current ? 'unread' : current.durationDays === 0 ? 'preparing' : 'running';
  const read = current ? readAt(reply.readHours, end ? Math.min(end.lastSeen, current.lastHour) : Math.min(hour, current.lastHour)) ?? current.readHour : null;
  return { operation, run: current, last, readHour: read, state,
    countryHour: end?.byHour ?? read ?? readAt(reply.readHours, last!.lastHour) ?? last!.readHour,
    ownerTag: end ? end.ownerTag : (current ?? last)?.ownerTag ?? null, targetTag: end ? end.targetTag : (current ?? last)?.targetTag ?? null,
    typeKey: end ? end.typeKey : (current ?? last)?.typeKey ?? null,
    startHour: operation.startKnownAt !== null && operation.startKnownAt <= hour ? operation.startHour : null };
}
export interface OperationFilter {
  types: readonly string[];
  country: (tag: string, hour: number) => boolean;
  role: 'owner' | 'target' | 'either';
  includeUnknownCountries?: boolean;
  from: number | null;
  live: boolean;
  runningOrder: 'start' | 'preparation';
}
export function operationRows(reply: EspionageLogReply, hour: number, filter: OperationFilter): LogRow<OperationAt>[] {
  const types = new Set(filter.types);
  const rows: LogRow<OperationAt>[] = [];
  for (const operation of reply.operations) {
    const at = operationAt(reply, operation, hour);
    if (!at || (types.size && (at.typeKey === null || !types.has(at.typeKey)))) continue;
    const end = operation.end && operation.end.byHour <= hour ? operation.end : null;
    const date = end?.completionHour ?? end?.byHour ?? at.readHour ?? (at.last ? readAt(reply.readHours, at.last.lastHour) ?? at.last.readHour : operation.firstRead);
    const owner = at.ownerTag === null ? filter.includeUnknownCountries === true : filter.country(at.ownerTag, at.countryHour);
    const target = at.targetTag === null ? filter.includeUnknownCountries === true : filter.country(at.targetTag, at.countryHour);
    if ((filter.role === 'owner' && !owner) || (filter.role === 'target' && !target) || (filter.role === 'either' && !owner && !target)) continue;
    if (filter.from !== null && date < filter.from) continue;
    rows.push({ key: `${operation.id}:${end?.byHour ?? 'active'}`, id: operation.id, hour: date, by: end !== null && end.completionHour === null, item: at });
  }
  return sortOperationRows(rows, filter);
}
function sortOperationRows(rows: LogRow<OperationAt>[], filter: OperationFilter): LogRow<OperationAt>[] {
  const ongoing = (row: LogRow<OperationAt>) => row.item.state === 'preparing' || row.item.state === 'running';
  return rows.sort((a, b) => {
    if (filter.live) {
      const first = ongoing(a); const second = ongoing(b);
      if (first !== second) return first ? -1 : 1;
      if (first && filter.runningOrder === 'preparation') {
        const av = a.item.run?.preparationRatio; const bv = b.item.run?.preparationRatio;
        if (av !== bv) return av == null ? 1 : bv == null ? -1 : bv - av;
      }
      if (first) return (b.item.startHour ?? b.item.operation.firstRead) - (a.item.startHour ?? a.item.operation.firstRead) || a.id!.localeCompare(b.id!);
    }
    return b.hour - a.hour || a.id!.localeCompare(b.id!);
  });
}
export function operationWindowIdentity(sourceId: string, revision: string, branchId: string, content: number): string {
  return JSON.stringify([sourceId, revision, branchId, content]);
}
export function operationWindowIndex(windows: readonly { firstHour: number; lastHour: number }[], at: number): number {
  for (let index = windows.length - 1; index >= 0; index--) if (windows[index].firstHour <= at) return index;
  return -1;
}
/** Hold the playhead, visible windows and their earlier neighbour. */
export function operationWindows(manifest: EspionageLogReply | undefined, at: number, live: boolean, visible: readonly number[] = [], from = -Infinity): number[] {
  if (!manifest) return [];
  const current = operationWindowIndex(manifest.windows, at);
  if (current < 0) return [];
  const wanted = new Set([current, ...visible.filter(index => index >= 0 && index <= current && manifest.windows[index]?.lastHour >= from)]);
  const oldest = Math.min(...wanted);
  if (oldest > 0 && manifest.windows[oldest - 1].lastHour >= from) wanted.add(oldest - 1);
  if (live && current + 1 < manifest.windows.length) wanted.add(current + 1);
  return [...wanted].sort((a, b) => a - b);
}
/** A row belongs to its actual ending/read window; a spanning episode is shown
 * once, using the latest loaded window at or before the requested moment. */
export function windowedOperationRows(windows: ReadonlyMap<number, EspionageLogReply>, hour: number, filter: OperationFilter): LogRow<OperationAt>[] {
  const rows = new Map<string, LogRow<OperationAt>>();
  const sorted = [...windows].sort(([a], [b]) => a - b);
  const current = sorted.find(([, reply]) => reply.firstHour <= hour && hour <= reply.lastHour)?.[0];
  for (const [index, reply] of sorted) {
    if (reply.firstHour > hour) continue;
    for (const row of operationRows(reply, Math.min(hour, reply.lastHour), filter)) {
      const ended = row.item.state === 'completed' || row.item.state === 'disappeared';
      if (!ended && index !== current) continue;
      row.item.windowIndex = index;
      rows.set(row.item.operation.id, row);
    }
  }
  return sortOperationRows([...rows.values()], filter);
}

export function operationState(at: OperationAt): string {
  switch (at.state) {
    case 'preparing': return 'Preparing';
    case 'running': return 'Running';
    case 'completed': return 'Completed';
    case 'disappeared': return 'Ended';
    case 'unread': return 'Not recorded at this hour';
  }
}
/** Sentence parts let the log keep each country's flag beside its own name. */
export function operationWording(at: OperationAt): { action: string; afterTarget: string } {
  switch (at.state) {
    case 'preparing': return { action: ' is preparing ', afterTarget: '' };
    case 'running': return { action: ' is running ', afterTarget: '' };
    case 'completed': return { action: ' completed ', afterTarget: '' };
    case 'disappeared': return { action: '’s ', afterTarget: ' ended' };
    case 'unread': return { action: '’s ', afterTarget: ' has no recorded read at this hour' };
  }
}
export const preparationRatio = (at: OperationAt) => at.run?.preparationRatio == null ? null : at.run.preparationRatio / 100_000;
/** Native elapsed-day result is supplied by the read, never the playhead. */
export const executionRatio = (at: OperationAt) => at.run?.elapsedDays == null || at.run.durationDays === 0 ? null : at.run.elapsedDays / at.run.durationDays;
export const operationProgress = (at: OperationAt) => at.state === 'preparing' ? preparationRatio(at) : at.state === 'running' ? executionRatio(at) : null;
/** Active rows have no outcome. Stored descriptions retain their recorded language. */
export function operationOutcome(at: OperationAt): string | null {
  if (at.state !== 'completed' && at.state !== 'disappeared') return null;
  const end = at.operation.end;
  return end?.outcome?.descriptionUnrecorded ?? (end?.outcome?.descriptionRef ? 'Recorded outcome' : end?.outcomeUnrecorded ?? 'Unknown outcome');
}
export function operationJumpHours(at: OperationAt, shownHour: number, rowHour?: number): number[] {
  const end = at.operation.end && at.operation.end.byHour <= shownHour ? (at.operation.end.completionHour ?? at.operation.end.byHour) : null;
  const hours = [at.startHour, end].filter((hour): hour is number => hour !== null);
  return [...new Set(hours.length ? hours : [rowHour ?? at.operation.firstRead])].sort((a, b) => a - b);
}
export function operationTooltip(reply: EspionageLogReply, row: LogRow<OperationAt>, country: (tag: string | null) => ReturnType<CountryLabel>, description?: string): TooltipContent[] {
  const at = row.item;
  const type = reply.types.find(type => type.key === at.typeKey);
  const current = at.run;
  const countryRow = (label: string, tag: string | null): TooltipRow => {
    const value = country(tag);
    return { label, value: value.name, countryFlag: tag === null ? undefined : {
      flag: value.flag, flagReason: value.flagReason, flagPending: value.flagPending,
    } };
  };
  const rows: TooltipRow[] = [
    countryRow('Owner', at.ownerTag), countryRow('Target', at.targetTag),
    { label: 'Operation first observed', value: `by ${formatDateTime(at.operation.firstRead)}` },
    { label: 'Execution started', value: at.startHour === null ? 'Not recorded' : formatDateTime(at.startHour) },
    { label: 'Status', value: operationState(at) },
  ];
  const ended = at.state === 'completed' || at.state === 'disappeared';
  const outcome = ended && at.operation.end?.outcome?.descriptionRef ? description ?? 'Loading recorded description...' : operationOutcome(at);
  if (outcome !== null) rows.push({ label: 'Outcome', value: outcome });
  const end = at.operation.end;
  if (end && (at.state === 'completed' || at.state === 'disappeared')) {
    rows.push({ label: at.state === 'completed' ? 'Completed' : 'Ended', value: end.completionHour === null ? `by ${formatDateTime(end.byHour)}` : formatDateTime(end.completionHour) });
    if (end.outcome) rows.push({ label: 'Recorded description language', value: end.outcome.recordedLanguage },
      { label: 'Bonus result', value: end.outcome.bonus ? 'Yes' : 'No' },
      { label: 'Detected', value: end.outcome.detected ? 'Yes' : 'No' });
  }
  if (current) {
    // Preparation percentages truncate to whole percent.
    if (at.state === 'running') rows.push({ label: 'Execution progress', value: current.elapsedDays === null ? 'Not recorded (native elapsed-day sentinel)' : `${current.elapsedDays} / ${current.durationDays} days` });
    rows.push({ label: 'Phases', value: current.currentPhase === null ? `${current.phaseCount} selected phases; current phase not recorded`
      : `Phase ${current.currentPhase + 1} of ${current.phaseCount}; phase names not recorded` },
      { label: 'Overall civilian preparation', value: current.preparationRatio === null ? 'Not recorded' : `${Math.trunc(current.preparationRatio / 1000)}%` },
      { label: 'Equipment readiness', value: 'Not recorded' },
      { label: 'Automatic commencement', value: current.autoCommence ? 'Enabled' : 'Disabled' },
      { label: 'Automatic repeat', value: current.autoRepeat ? 'Enabled' : 'Disabled' },
      // The slot count includes unassigned slots.
      { label: 'Operative slots (including unassigned)', value: String(current.assignedSlots) });
    if (current.operatives === null) rows.push({ label: 'Assigned operatives', value: 'Not recorded' });
    else {
      rows.push({ label: 'Assigned operatives', value: String(current.operatives.length) });
      for (const [index, id] of current.operatives.entries()) {
        const name = current.operativeNames?.[index];
        rows.push({ label: `Operative ${id}`, value: name?.name === '' ? 'Unnamed operative' : name?.name ?? name?.unavailable ?? 'Name not recorded' });
      }
    }
  } else rows.push({ label: 'Preparation, phases and assignments at this hour', value: 'Not recorded', tone: 'muted' });
  if (type?.iconMissing) rows.push({ label: type.iconMissing, tone: 'muted' });
  return [{ title: operationName(type), icon: type?.icon ?? undefined, rows,
    note: end && (at.state === 'completed' || at.state === 'disappeared')
      ? `Ending read ${formatDateTime(end.byHour)}. ${at.readHour === null ? 'Earlier operation values not recorded.' : `Operation values read ${formatDateTime(at.readHour)}.`}`
      : at.readHour === null ? `The current operation read is missing. Gaps stay gaps. Country identities last read ${formatDateTime(at.countryHour)}.`
        : `Values and country identities read ${formatDateTime(at.readHour)}. Names belong to the supplying read.` }];
}
