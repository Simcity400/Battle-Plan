import type { LogPage, LogRow } from '../../charts/log/logData';
import type { LogReplyRow } from '../../charts/log/logSource';
import { countryNameDetails } from '../../charts/tooltip.ts';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { drawableIcon } from '../../dashboard/artwork.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { compareCountries } from '../../dashboard/intelligence.ts';
import { setToken, storableToken, UNSTORABLE } from '../../dashboard/options.ts';
import type {
  Country, EndedOperation, EspionageEndedReply, EspionageOperationReply, EspionageOperationsReply, EspionageTypesReply, EspionageTypeTokensReply, LogCell, OperationState, OperationType,
  OperationUnderWay, OperativeName,
} from '../../generated/dashboard';

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const integer = (value: unknown): value is number => Number.isSafeInteger(value);
const text = (value: unknown): value is string | null => value === null || typeof value === 'string';
const maybe = (value: unknown) => value === null || integer(value);
const handle = (value: unknown): value is string => typeof value === 'string' && /^\d{1,20}$/.test(value);
const country = (value: unknown): value is Country => object(value) && integer(value.id) && (value.tag === null || (typeof value.tag === 'string' && value.tag.length > 0));
const warnings = (value: Record<string, unknown>) => Array.isArray(value.warnings) && value.warnings.every(item => typeof item === 'string');
const state = (value: unknown): value is OperationState => object(value) && maybe(value.startHour) && integer(value.durationDays) && integer(value.phaseCount)
  && maybe(value.currentPhase) && maybe(value.elapsedDays) && (value.preparationRatio === null || (integer(value.preparationRatio) && value.preparationRatio >= 0
    && value.preparationRatio <= 100_000)) && integer(value.assignedSlots) && typeof value.autoRepeat === 'boolean' && typeof value.autoCommence === 'boolean'
  && typeof value.terminal === 'boolean';
const malformed = (what: string) => new Error(`The ${what} reply is malformed.`);

export function validateOperations(value: unknown): EspionageOperationsReply {
  if (!object(value) || !Array.isArray(value.operations) || !value.operations.every(item => object(item) && handle(item.id) && country(item.owner)
    && country(item.target) && text(item.operation) && state(item.state)) || !text(value.missing) || !warnings(value)) throw malformed('espionage operations');
  return value as unknown as EspionageOperationsReply;
}

export function validateEnded(value: unknown): EspionageEndedReply {
  if (!object(value) || !Array.isArray(value.operations) || !value.operations.every(item => object(item) && handle(item.id) && country(item.owner)
    && country(item.target) && text(item.operation) && integer(item.firstSeen) && integer(item.lastSeen) && integer(item.byHour)
    && item.firstSeen <= item.lastSeen && item.lastSeen < item.byHour && typeof item.completionsRecorded === 'boolean' && state(item.state))
    || !warnings(value)) throw malformed('ended operations');
  return value as unknown as EspionageEndedReply;
}

export function validateTypes(value: unknown): EspionageTypesReply {
  if (!object(value) || !Array.isArray(value.types) || !value.types.every(item => object(item) && typeof item.key === 'string' && item.key.length > 0
    && text(item.name) && text(item.icon) && text(item.missing))
    || new Set(value.types.map(item => (item as { key: string }).key)).size !== value.types.length || !text(value.missing) || !warnings(value)) {
    throw malformed('operation types');
  }
  const reply = value as unknown as EspionageTypesReply;
  for (const type of reply.types) {
    const original = type.icon;
    type.icon = drawableIcon(original);
    if (original !== null && type.icon === null) type.missing = [type.missing, `Artwork refused for operation type ${type.key}: not a drawable bounded PNG`].filter(Boolean).join('; ');
  }
  return reply;
}

export function validateTypeTokens(value: unknown): EspionageTypeTokensReply {
  if (!object(value) || !Array.isArray(value.types) || !value.types.every(item => object(item) && typeof item.key === 'string' && item.key.length > 0
    && Array.isArray(item.tokens) && item.tokens.every(token => integer(token) && token >= 0))
    || new Set(value.types.map(item => (item as { key: string }).key)).size !== value.types.length || !text(value.missing) || !warnings(value)) {
    throw malformed('operation type tokens');
  }
  return value as unknown as EspionageTypeTokensReply;
}

const operative = (value: unknown): value is OperativeName => object(value) && handle(value.id) && text(value.name) && text(value.unavailable)
  && (value.name === null) !== (value.unavailable === null);
export function validateOperation(value: unknown, id: string): EspionageOperationReply {
  if (!object(value) || value.id !== id || !integer(value.firstSeen) || !integer(value.lastSeen) || !integer(value.readHour) || !state(value.state)
    || !(value.operatives === null || (Array.isArray(value.operatives) && value.operatives.every(operative))) || !warnings(value)) throw malformed('espionage operation');
  return value as unknown as EspionageOperationReply;
}

export type OperationKind = 'preparing' | 'running' | 'completed' | 'ended';

export interface Completion {
  completionHour: number | null; terminal: number | null; bonus: number | null; detected: number | null;
  description: string | null; descriptionUnrecorded: string | null;
}

export interface OperationItem {
  id: string | null;
  kind: OperationKind;
  owner: Country;
  target: Country | null;
  operation: string | null;
  countryHour: number;
  startHour: number | null;
  firstSeen: number | null;
  lastSeen: number | null;
  byHour: number | null;
  state: OperationState | null;
  completion: Completion | null;
  completionsRecorded: boolean;
}

export const kindOf = (value: OperationState): OperationKind => value.durationDays === 0 ? 'preparing' : 'running';

const cellNumber = (cell: LogCell | undefined) => cell?.kind === 'number' ? Number(cell.value) : null;
const cellText = (cell: LogCell | undefined) => cell?.kind === 'text' ? cell.value : null;

export function completionRow(row: LogReplyRow): LogRow<OperationItem> {
  const cells = row.cells;
  const target = cellNumber(cells.target);
  return { key: row.key, id: `completion:${row.key}`, hour: row.hour, item: {
    id: cellText(cells.id), kind: 'completed', owner: { id: cellNumber(cells.owner) ?? 0, tag: cellText(cells.ownerTag) },
    target: target === null ? null : { id: target, tag: cellText(cells.targetTag) }, operation: cellText(cells.operation), countryHour: row.hour,
    startHour: cellNumber(cells.startHour), firstSeen: cellNumber(cells.firstSeen), lastSeen: cellNumber(cells.lastSeen), byHour: null, state: null,
    completion: { completionHour: cellNumber(cells.completionHour), terminal: cellNumber(cells.terminal), bonus: cellNumber(cells.bonus), detected: cellNumber(cells.detected),
      description: cellText(cells.description), descriptionUnrecorded: cellText(cells.descriptionUnrecorded) },
    completionsRecorded: true,
  } };
}

export function underWayItem(operation: OperationUnderWay, hour: number): OperationItem {
  return { id: operation.id, kind: kindOf(operation.state), owner: operation.owner, target: operation.target, operation: operation.operation, countryHour: hour,
    startHour: operation.state.startHour, firstSeen: null, lastSeen: null, byHour: null, state: operation.state, completion: null, completionsRecorded: true };
}

export function endedItem(operation: EndedOperation): OperationItem {
  return { id: operation.id, kind: 'ended', owner: operation.owner, target: operation.target, operation: operation.operation, countryHour: operation.lastSeen,
    startHour: operation.state.startHour, firstSeen: operation.firstSeen, lastSeen: operation.lastSeen, byHour: operation.byHour, state: operation.state,
    completion: null, completionsRecorded: operation.completionsRecorded };
}

export interface OperationFilter {
  types: ReadonlySet<string> | null;
  role: 'owner' | 'target' | 'either';
  country: (tag: string, hour: number) => boolean;
  includeUnknown: boolean;
  from: number | null;
}

export function keeps(item: OperationItem, date: number, filter: OperationFilter): boolean {
  if (filter.types && (item.operation === null || !filter.types.has(item.operation))) return false;
  if (filter.from !== null && date < filter.from) return false;
  const side = (value: Country | null) => value?.tag ? filter.country(value.tag, item.countryHour) : filter.includeUnknown;
  return filter.role === 'owner' ? side(item.owner) : filter.role === 'target' ? side(item.target) : side(item.owner) || side(item.target);
}

export function underWayRows(reply: EspionageOperationsReply | undefined, hour: number, filter: OperationFilter, order: 'start' | 'preparation'): LogRow<OperationItem>[] {
  if (!reply) return [];
  const rows = reply.operations.map(operation => underWayItem(operation, hour)).filter(item => keeps(item, hour, filter));
  const start = (item: OperationItem) => item.startHour ?? -Infinity;
  const prepared = (item: OperationItem) => item.state?.preparationRatio ?? -1;
  rows.sort((a, b) => (order === 'preparation' ? prepared(b) - prepared(a) : 0) || start(b) - start(a) || compareCountries(a.owner, b.owner) || a.id!.localeCompare(b.id!));
  return rows.map(item => ({ key: `op:${item.id}`, id: `op:${item.id}`, hour: item.startHour ?? hour, item }));
}

export function underWayStructure(reply: EspionageOperationsReply, order: 'start' | 'preparation'): string {
  const rows = underWayRows(reply, 0, { types: null, role: 'owner', country: () => true, includeUnknown: true, from: null }, order);
  return JSON.stringify([reply.missing, rows.map(row => [row.item.id, row.item.kind, row.item.owner, row.item.target, row.item.operation, row.item.startHour,
    row.item.state?.phaseCount, row.item.state?.assignedSlots, row.item.state?.autoRepeat, row.item.state?.autoCommence])]);
}

export function endedRows(reply: EspionageEndedReply | undefined, shown: number | null, filter: OperationFilter): LogRow<OperationItem>[] {
  if (!reply) return [];
  const through = shown === null ? reply.operations : reply.operations.slice(Math.max(0, reply.operations.length - shown));
  return through.map(endedItem).filter(item => keeps(item, item.byHour!, filter))
    .map(item => ({ key: `ended:${item.id}:${item.byHour}`, id: `ended:${item.id}`, hour: item.byHour!, by: true, item }));
}

export function endedThrough(reply: EspionageEndedReply | undefined, hour: number): number {
  if (!reply) return 0;
  let low = 0, high = reply.operations.length;
  while (low < high) {
    const middle = (low + high) >> 1;
    if (reply.operations[middle].byHour > hour) low = middle + 1; else high = middle;
  }
  return reply.operations.length - low;
}

export function filteredPages(pages: readonly LogPage<OperationItem>[], filter: OperationFilter | null): readonly LogPage<OperationItem>[] {
  if (!filter) return pages;
  return pages.map(page => page.rows ? { ...page, rows: page.rows.filter(row => keeps(row.item, row.hour, filter)) } : page);
}

export function operationChoices(types: EspionageTypesReply | undefined, tokens: EspionageTypeTokensReply | undefined) {
  const recorded = new Set((tokens?.types ?? []).filter(type => type.tokens.length > 0).map(type => type.key));
  return (types?.types ?? []).filter(type => recorded.has(type.key)).map(type => ({ value: setToken(type.key), label: operationName(type), icon: type.icon ?? undefined,
    unavailable: storableToken(setToken(type.key)) ? undefined : UNSTORABLE })).sort((a, b) => a.label.localeCompare(b.label));
}

export function typeTokens(tokens: EspionageTypeTokensReply | undefined, chosen: ReadonlySet<string> | null): number[] | null | undefined {
  if (!chosen) return null;
  if (!tokens) return undefined;
  return [...new Set(tokens.types.filter(type => chosen.has(type.key)).flatMap(type => type.tokens))].sort((a, b) => a - b);
}

export const operationName = (type: OperationType | undefined, pending = false) => contentTitle(type?.name, type?.key, 'Operation name unavailable', pending);

export function operationState(item: OperationItem): string {
  switch (item.kind) {
    case 'preparing': return 'Preparing';
    case 'running': return 'Running';
    case 'completed': return 'Completed';
    case 'ended': return item.completionsRecorded ? 'Ended without completing' : 'Ended; completion not recorded';
  }
}

export function operationWording(item: OperationItem): { action: string; afterTarget: string } {
  switch (item.kind) {
    case 'preparing': return { action: ' is preparing ', afterTarget: '' };
    case 'running': return { action: ' is running ', afterTarget: '' };
    case 'completed': return { action: ' completed ', afterTarget: '' };
    case 'ended': return { action: '’s ', afterTarget: ' ended' };
  }
}

const flag = (name: string, value: number | null): { value: boolean | null; refused: string | null } => value === null ? { value: null, refused: null }
  : value === 0 || value === 1 ? { value: value === 1, refused: null } : { value: null, refused: `Stored operation ${name} refused: flag byte ${value} is not 0 or 1` };

export function outcomeText(item: OperationItem): string | null {
  if (item.kind === 'ended') return item.completionsRecorded ? 'Outcome not recorded.' : 'Completion not recorded.';
  const completion = item.completion;
  if (!completion) return null;
  if (completion.bonus === null || completion.detected === null || flag('bonus', completion.bonus).refused || flag('detection', completion.detected).refused) return 'Outcome not recorded.';
  return completion.descriptionUnrecorded !== null ? 'Description unavailable.' : 'Outcome';
}

export function operationProgress(item: Pick<OperationItem, 'kind'>, value: OperationState | null): number | null {
  if (!value) return null;
  if (item.kind === 'preparing') return value.preparationRatio === null ? null : value.preparationRatio / 100_000;
  if (item.kind === 'running') return value.elapsedDays === null || value.durationDays === 0 ? null : value.elapsedDays / value.durationDays;
  return null;
}

export function progressText(item: Pick<OperationItem, 'kind'>, value: OperationState | null): string {
  if (!value) return 'Not recorded';
  if (item.kind === 'preparing') return value.preparationRatio === null ? 'Preparation not recorded' : `Preparation ${Math.trunc(value.preparationRatio / 1000)}%`;
  return value.elapsedDays === null ? 'Elapsed days not recorded' : `${value.elapsedDays} / ${value.durationDays} days`;
}

export function jumpHours(item: OperationItem, rowHour: number): number[] {
  const end = item.kind === 'completed' ? item.completion?.completionHour ?? rowHour : item.byHour;
  const hours = [item.startHour, end].filter((hour): hour is number => hour !== null);
  return [...new Set(hours.length ? hours : [item.firstSeen ?? rowHour])].sort((a, b) => a - b);
}

export type OperationDetail = { reply: EspionageOperationReply } | { failure: string } | null;

export function operationTooltip(item: OperationItem, rowHour: number, type: OperationType | undefined, label: (value: Country | null) => ReturnType<CountryLabel>,
  current: OperationState | null, detail: OperationDetail, pending = false): TooltipContent {
  const countryRow = (name: string, value: Country | null): TooltipRow => {
    const shown = label(value);
    return { ...countryNameDetails(shown), label: name, value: shown.name, countryFlag: value?.tag ? {
      flag: shown.flag, flagReason: shown.flagReason, ...(shown.flagDetail !== undefined ? { flagDetail: shown.flagDetail } : {}), flagPending: shown.flagPending } : undefined };
  };
  const read = detail && 'reply' in detail ? detail.reply : null;
  const firstSeen = item.firstSeen ?? read?.firstSeen ?? null;
  const rows: TooltipRow[] = [
    countryRow('Owner', item.owner), countryRow('Target', item.target),
    { label: 'First seen', value: firstSeen === null ? detail === null ? 'Loading…' : 'Not recorded' : `by ${formatDateTime(firstSeen)}` },
    { label: 'Execution started', value: item.startHour === null ? 'Not recorded' : formatDateTime(item.startHour) },
    { label: 'Status', value: operationState(item) },
  ];
  const completion = item.completion;
  if (item.kind === 'completed' && completion) {
    const bonus = flag('bonus', completion.bonus), detected = flag('detection', completion.detected);
    rows.push({ label: 'Outcome', value: completion.description ?? completion.descriptionUnrecorded ?? 'Outcome not recorded.' },
      { label: 'Completed', value: completion.completionHour === null ? `by ${formatDateTime(rowHour)}` : formatDateTime(completion.completionHour) },
      { label: 'Bonus result', value: bonus.refused ?? (bonus.value === null ? 'Not recorded' : bonus.value ? 'Yes' : 'No') },
      { label: 'Detected', value: detected.refused ?? (detected.value === null ? 'Not recorded' : detected.value ? 'Yes' : 'No') });
  } else if (item.kind === 'ended') {
    rows.push({ label: 'Outcome', value: outcomeText(item)! }, { label: 'Ended', value: `by ${formatDateTime(item.byHour!)}` });
  }
  const values = current ?? read?.state ?? null;
  if (values) {
    if (item.kind === 'running') rows.push({ label: 'Execution progress', value: values.elapsedDays === null ? 'Not recorded' : `${values.elapsedDays} / ${values.durationDays} days` });
    rows.push({ label: 'Phases', value: values.currentPhase === null ? `${values.phaseCount} phases; current phase unknown` : `Phase ${values.currentPhase + 1} of ${values.phaseCount}` },
      { label: 'Preparation', value: values.preparationRatio === null ? 'Not recorded' : `${Math.trunc(values.preparationRatio / 1000)}%` },
      { label: 'Auto-start', value: values.autoCommence ? 'Enabled' : 'Disabled' },
      { label: 'Auto-repeat', value: values.autoRepeat ? 'Enabled' : 'Disabled' });
  } else rows.push({ label: 'Preparation, phases and assignments', value: detail === null ? 'Loading…' : 'Not recorded', tone: 'muted' });
  if (read) {
    rows.push({ label: 'Operatives', value: `${read.operatives?.length ?? 'Unknown'} / ${read.state.assignedSlots}` });
    for (const name of read.operatives ?? []) rows.push({ label: 'Operative', value: name.name === '' ? 'Unnamed operative' : name.name ?? name.unavailable ?? 'Name unavailable' });
  } else if (detail && 'failure' in detail) rows.push({ label: 'Operatives', value: detail.failure, tone: 'muted' });
  return { title: operationName(type, pending), icon: type?.icon ?? undefined, rows, ...(type?.missing ? { note: type.missing } : {}) };
}

export function paintOperations(root: Element | null, reply: EspionageOperationsReply) {
  if (!root) return;
  const byId = new Map(reply.operations.map(operation => [operation.id, operation]));
  for (const element of root.querySelectorAll<HTMLElement>('[data-esp-op]')) {
    const operation = byId.get(element.dataset.espOp!);
    if (!operation) continue;
    const item = { kind: kindOf(operation.state) };
    if (element.dataset.espField === 'meter') {
      const progress = operationProgress(item, operation.state);
      const bar = element.firstElementChild as HTMLElement | null;
      if (bar) bar.style.width = progress === null ? '0' : `${Math.max(0, Math.min(1, progress)) * 100}%`;
      element.classList.toggle('is-unknown', progress === null);
    } else element.textContent = progressText(item, operation.state);
  }
}
