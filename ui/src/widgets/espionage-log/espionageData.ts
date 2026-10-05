import type { EspionageLogReply, EspionageOperation, EspionageOperationRun, EspionageOperationType } from '../../generated/dashboard';
import { isOrderedSpans, isSpan, runAt, spansCovered } from '../../charts/series.ts';
import type { LogRow } from '../../charts/log/logData';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { object, int } from '../../dashboard/validation.ts';
import { drawableIcon } from '../../dashboard/artwork.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { entryAt, readAt } from '../../dashboard/reads.ts';
import { setToken, storableToken, UNSTORABLE } from '../../dashboard/options.ts';

const nullableText = (v: unknown) => v === null || typeof v === 'string';
const nullableInt = (v: unknown) => v === null || int(v);
const type = (v: unknown) => object(v) && typeof v.key === 'string' && v.key.length > 0 && nullableText(v.name) && nullableText(v.iconMissing) && 'icon' in v;
const run = (v: unknown) => isSpan(v) && int(v.readHour) && v.readHour <= v.firstHour
  && (v.preparationRatio === null || (int(v.preparationRatio) && v.preparationRatio >= 0 && v.preparationRatio <= 100_000))
  && nullableInt(v.elapsedDays) && nullableInt(v.startHour) && nullableInt(v.currentPhase) && typeof v.terminal === 'boolean'
  && typeof v.autoCommence === 'boolean'
  && (v.operatives === null || (Array.isArray(v.operatives) && v.operatives.every(id => typeof id === 'string')));

/** Types are content identities, not a fixed catalogue; bad art keeps its named placeholder. */
export function validateEspionageLogReply(data: unknown): EspionageLogReply {
  const reads = object(data) && Array.isArray(data.readHours) ? data.readHours : null;
  if (!object(data) || typeof data.branchId !== 'string' || !int(data.firstHour) || !int(data.lastHour) || data.firstHour > data.lastHour
    || !isOrderedSpans(data.available) || !reads || !reads.every((hour, i) => int(hour) && (!i || reads[i - 1] < hour))
    || !Array.isArray(data.types) || !data.types.every(type) || new Set(data.types.map(item => item.key)).size !== data.types.length
    || !Array.isArray(data.operations) || !data.operations.every(op => object(op) && typeof op.id === 'string' && int(op.firstRead)
      && nullableText(op.ownerTag) && nullableText(op.targetTag) && typeof op.typeKey === 'string'
      && int(op.phaseCount) && op.phaseCount >= 0 && typeof op.autoRepeat === 'boolean' && int(op.durationDays)
      && int(op.assignedSlots) && op.assignedSlots >= 0
      && nullableInt(op.startHour) && nullableInt(op.startKnownAt) && ((op.startHour === null) === (op.startKnownAt === null))
      && isOrderedSpans(op.runs) && op.runs.length > 0 && op.runs.every(run) && op.runs[0].firstHour === op.firstRead
      && (op.end === null || (object(op.end) && int(op.end.byHour) && int(op.end.lastSeen) && op.end.byHour >= op.end.lastSeen
        && (op.end.outcome === null || (object(op.end.outcome) && typeof op.end.outcome.description === 'string'))
        && nullableText(op.end.outcomeUnrecorded) && ((op.end.outcome === null) !== (op.end.outcomeUnrecorded === null))
        && nullableText(op.end.ownerTag) && nullableText(op.end.targetTag)
        && (op.end.reason === 'completed' || op.end.reason === 'disappeared'))))
    || new Set(data.operations.map(op => op.id)).size !== data.operations.length
    || !Array.isArray(data.warnings) || !data.warnings.every(warning => typeof warning === 'string') || !nullableText(data.unavailable)) {
    throw new Error('The espionage operations reply is malformed.');
  }
  const reply = data as unknown as EspionageLogReply;
  const types = new Set(reply.types.map(item => item.key));
  for (const operation of reply.operations) {
    if (!spansCovered(operation.runs, reply.available)) throw new Error('The espionage operations reply crosses unobserved hours.');
    if (!types.has(operation.typeKey)) throw new Error('The espionage operations reply names an unlisted operation type.');
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
export const operationName = (type: EspionageOperationType | undefined) => type?.name || (type?.key ? `Unknown operation type (${type.key})` : 'Operation type not recorded');
export function operationChoices(reply: EspionageLogReply | undefined) {
  return (reply?.types ?? []).map(type => ({ value: setToken(type.key), label: operationName(type), icon: type.icon,
    unavailable: storableToken(setToken(type.key)) ? undefined : UNSTORABLE }))
    .sort((a, b) => a.label.localeCompare(b.label));
}
export interface OperationAt {
  operation: EspionageOperation;
  /** Null in a gap. `last` is identity/history only there, never a current measurement. */
  run: EspionageOperationRun | null;
  last: EspionageOperationRun;
  readHour: number | null;
  state: 'preparing' | 'running' | 'completed' | 'disappeared' | 'unread';
  startHour: number | null;
  ownerTag: string | null;
  targetTag: string | null;
}
export function operationAt(reply: EspionageLogReply, operation: EspionageOperation, hour: number): OperationAt | null {
  if (hour < operation.firstRead) return null;
  const last = entryAt(operation.runs, hour);
  if (!last) return null;
  const end = operation.end && operation.end.byHour <= hour ? operation.end : null;
  const current = end ? runAt(operation.runs, end.lastSeen) : runAt(operation.runs, hour);
  const state = end ? end.reason === 'completed' ? 'completed' : 'disappeared'
    : !current ? 'unread' : operation.durationDays === 0 ? 'preparing' : 'running';
  const read = current ? readAt(reply.readHours, end ? Math.min(end.lastSeen, current.lastHour) : Math.min(hour, current.lastHour)) : null;
  return { operation, run: current, last, readHour: read, state,
    ownerTag: end ? end.ownerTag : operation.ownerTag, targetTag: end ? end.targetTag : operation.targetTag,
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
    if (!at || (types.size && !types.has(at.operation.typeKey))) continue;
    const end = operation.end && operation.end.byHour <= hour ? operation.end : null;
    const date = end?.byHour ?? at.readHour ?? readAt(reply.readHours, at.last.lastHour) ?? at.last.readHour;
    const owner = at.ownerTag === null ? filter.includeUnknownCountries === true : filter.country(at.ownerTag, date);
    const target = at.targetTag === null ? filter.includeUnknownCountries === true : filter.country(at.targetTag, date);
    if ((filter.role === 'owner' && !owner) || (filter.role === 'target' && !target) || (filter.role === 'either' && !owner && !target)) continue;
    if (filter.from !== null && date < filter.from) continue;
    rows.push({ key: `${operation.id}:${end?.byHour ?? 'active'}`, id: operation.id, hour: date, by: end !== null, item: at });
  }
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
export const executionRatio = (at: OperationAt) => at.run?.elapsedDays == null || at.operation.durationDays === 0 ? null : at.run.elapsedDays / at.operation.durationDays;
export const operationProgress = (at: OperationAt) => at.state === 'preparing' ? preparationRatio(at) : at.state === 'running' ? executionRatio(at) : null;
/** One outcome adapter, ready for the later capture; active rows have no outcome. */
export function operationOutcome(at: OperationAt): string | null {
  if (at.state !== 'completed' && at.state !== 'disappeared') return null;
  const end = at.operation.end;
  return end?.outcome?.description ?? end?.outcomeUnrecorded ?? 'Unknown outcome';
}
export function operationJumpHours(at: OperationAt, shownHour: number, rowHour?: number): number[] {
  const end = at.operation.end && at.operation.end.byHour <= shownHour ? at.operation.end.byHour : null;
  const hours = [at.startHour, end].filter((hour): hour is number => hour !== null);
  return [...new Set(hours.length ? hours : [rowHour ?? at.operation.firstRead])].sort((a, b) => a - b);
}
export function operationTooltip(reply: EspionageLogReply, row: LogRow<OperationAt>, name: (tag: string | null) => string): TooltipContent[] {
  const at = row.item;
  const type = reply.types.find(type => type.key === at.operation.typeKey);
  const current = at.run;
  const rows: TooltipRow[] = [
    { label: 'Owner → target', value: `${name(at.ownerTag)} → ${name(at.targetTag)}` },
    { label: 'Operation first observed', value: `by ${formatDateTime(at.operation.firstRead)}` },
    { label: 'Execution started', value: at.startHour === null ? 'Not recorded' : formatDateTime(at.startHour) },
    { label: 'Status', value: operationState(at) },
  ];
  const outcome = operationOutcome(at);
  if (outcome !== null) rows.push({ label: 'Outcome', value: outcome });
  const end = at.operation.end;
  if (end && (at.state === 'completed' || at.state === 'disappeared')) rows.push({ label: at.state === 'completed' ? 'Completed' : 'Ended', value: `by ${formatDateTime(end.byHour)}` });
  if (current) {
    // P: 142259590 -> Fixed5ToString@1424B6200(...,0) truncates native percent.
    if (at.state === 'running') rows.push({ label: 'Execution progress', value: current.elapsedDays === null ? 'Not recorded (native elapsed-day sentinel)' : `${current.elapsedDays} / ${at.operation.durationDays} days` });
    rows.push({ label: 'Phases', value: current.currentPhase === null ? `${at.operation.phaseCount} selected phases; current phase not recorded`
      : `Phase ${current.currentPhase + 1} of ${at.operation.phaseCount}; phase names not recorded` },
      { label: 'Overall civilian preparation', value: current.preparationRatio === null ? 'Not recorded' : `${Math.trunc(current.preparationRatio / 1000)}%` },
      { label: 'Equipment readiness', value: 'Not recorded' },
      { label: 'Automatic commencement', value: current.autoCommence ? 'Enabled' : 'Disabled' },
      { label: 'Automatic repeat', value: at.operation.autoRepeat ? 'Enabled' : 'Disabled' },
      // P: 1413E9210 counts all +E0 slots, including the unassigned +EC slots.
      { label: 'Operative slots (including unassigned)', value: String(at.operation.assignedSlots) });
    if (current.operatives === null) rows.push({ label: 'Assigned operatives', value: 'Not recorded' });
    else {
      rows.push({ label: 'Assigned operatives', value: String(current.operatives.length) });
      for (const id of current.operatives) rows.push({ label: `Operative ${id}`, value: 'Name not recorded' });
    }
  } else rows.push({ label: 'Preparation, phases and assignments at this hour', value: 'Not recorded', tone: 'muted' });
  if (type?.iconMissing) rows.push({ label: type.iconMissing, tone: 'muted' });
  return [{ title: operationName(type), icon: type?.icon ?? undefined, rows,
    note: at.readHour === null ? 'The current operation read is missing. Gaps stay gaps.' : `Values read ${formatDateTime(at.readHour)}. Operative names were not recorded.` }];
}
