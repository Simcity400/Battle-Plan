import type { LogRow } from '../../charts/log/logData';
import type { BoundFacetFilter } from '../../charts/log/logSource';
import { drawableIcon } from '../../dashboard/artwork.ts';
import type { ProductRequest } from '../../dashboard/client';
import { int, object, opaqueTag } from '../../dashboard/validation.ts';
import type { EventFeedItem, EventFeedRow, EventFeedSpanReply, EventFeedUnavailable, FeedKindCode } from '../../generated/dashboard';
import { productRequest } from '../../dashboard/request.ts';

const SCRIPT_KINDS = new Set(['scriptEventFired', 'newsEventFired', 'eventOptionSelected', 'decisionSelected', 'decisionRemoved', 'decisionCancelled',
  'decisionTimedOut', 'decisionActivated', 'decisionCompleted', 'decisionReenabled', 'decisionRemovalEffect', 'decisionCancellationEffect',
  'decisionCompletionEffect', 'decisionTimeoutEffect', 'decisionDurationChanged']);
const NATIVE_KINDS = new Set([...SCRIPT_KINDS, 'warDeclared', 'warJoined', 'civilWarWarTransferred', 'annexed', 'warEnded', 'released',
  'stateTransferred', 'peaceConferenceStarted', 'nuclearStrike']);
const CAUSED_KINDS = new Set([...NATIVE_KINDS].filter(kind => kind !== 'nuclearStrike'));
const KINDS = new Set([...NATIVE_KINDS, 'warStarted', 'peace', 'capitulated', 'ceasedToExist', 'factionJoined', 'factionLeft', 'becameSubject',
  'guaranteeGranted', 'rulingPartyChanged', 'lawChanged', 'advisorHired', 'focusCompleted', 'projectFinished', 'nukesIncreased', 'nuclearStrike']);
const CATEGORY_NAMES = new Set(['warsPeace', 'diplomacy', 'politics', 'milestones']);
const text = (value: unknown): value is string | null => value === null || typeof value === 'string';
const amount = (value: unknown): value is number | null => value === null || (typeof value === 'number' && Number.isFinite(value));
const strike = (row: Record<string, unknown>): boolean => {
  if (row.kind !== 'nuclearStrike') return row.strike === undefined;
  const value = row.strike;
  return object(value) && int(value.province) && value.province >= 0 && (value.state === null || int(value.state))
    && amount(value.buildingLevels) && amount(value.warSupport) && (value.units === null || (int(value.units) && value.units >= 0))
    && amount(value.manpowerLost) && amount(value.equipmentLost)
    && Array.isArray(value.notRecorded) && value.notRecorded.every(reason => typeof reason === 'string');
};

const optionalText = (value: unknown) => value === undefined || typeof value === 'string';
const item = (value: unknown): value is EventFeedItem => object(value) && text(value.token) && text(value.name) && text(value.icon) && text(value.description)
  && optionalText(value.nameMissing) && optionalText(value.gaps)
  && (value.options === undefined || (Array.isArray(value.options) && value.options.every(option => object(option) && int(option.ordinal) && text(option.name))));
const roles = (row: Record<string, unknown>): boolean => {
  if (!NATIVE_KINDS.has(row.kind as string)) return row.participants === undefined;
  return row.exact === true && Array.isArray(row.participants) && row.participants.length === 2 && row.participants.every(value => value === null || opaqueTag(value))
    && JSON.stringify(row.participants.filter(value => value !== null)) === JSON.stringify(row.tags);
};
const cause = (row: Record<string, unknown>): boolean => {
  const value = row.cause;
  if (value === undefined) return true;
  return CAUSED_KINDS.has(row.kind as string) && object(value) && text(value.how) && text(value.within) && text(value.conference) && text(value.reason)
    && (value.state === null || (int(value.state) && value.state > 0)) && (value.ally === null || opaqueTag(value.ally))
    && Array.isArray(value.notes) && value.notes.every(note => typeof note === 'string');
};

const decisionState = (row: Record<string, unknown>): boolean => {
  const value = row.decisionState;
  if (value === undefined) return true;
  if (!SCRIPT_KINDS.has(row.kind as string) || !object(value) || typeof value.kind !== 'string') return false;
  switch (value.kind) {
    case 'active': case 'available': case 'completed': case 'aborted': case 'reEnableCooldown': return true;
    case 'failed': return int(value.observedHour);
    case 'unsupported': return int(value.raw);
    case 'notApplicable':
    case 'notRecorded': return typeof value.reason === 'string' && value.reason.length > 0;
    default: return false;
  }
};

export function validateFeedRow(value: unknown): EventFeedRow {
  if (!object(value) || typeof value.id !== 'string' || value.id.length === 0 || !int(value.hour) || !int(value.readHour) || !int(value.nameHour)
    || value.hour > value.readHour || typeof value.exact !== 'boolean' || typeof value.category !== 'string' || !CATEGORY_NAMES.has(value.category)
    || typeof value.kind !== 'string' || !KINDS.has(value.kind) || !Array.isArray(value.tags) || !value.tags.every(opaqueTag)
    || (!NATIVE_KINDS.has(value.kind) && value.tags.length === 0) || !roles(value) || !cause(value) || !strike(value) || !decisionState(value)
    || !(value.item === null || item(value.item)) || !(value.amount === null || (int(value.amount) && value.amount > 0))) {
    throw new Error('The event feed row is malformed.');
  }
  const row = value as unknown as EventFeedRow;
  const icon = row.item ? drawableIcon(row.item.icon) : null;
  return row.item && icon !== row.item.icon ? { ...row, item: { ...row.item, icon } } : row;
}

export interface FeedEvent { event: EventFeedRow; item: EventFeedItem | null }

const feedLogRow = (event: EventFeedRow, key: string): LogRow<FeedEvent> =>
  ({ key, id: event.id, hour: event.hour, revealedHour: event.readHour, by: false, item: { event, item: event.item } });

export function kindsRequest(track: { source: { id: string; revision: string }; branch: { id: string } }): ProductRequest {
  return productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'event-feed-span',
    params: { branchId: track.branch.id, categories: [], kindsOnly: true } });
}

export function validateSpan(value: unknown): EventFeedSpanReply {
  if (!object(value) || typeof value.branchId !== 'string' || !int(value.firstHour) || !int(value.lastHour) || !Array.isArray(value.kinds)
    || !value.kinds.every(kind => object(kind) && typeof kind.kind === 'string' && KINDS.has(kind.kind) && int(kind.code) && typeof kind.category === 'string'
      && CATEGORY_NAMES.has(kind.category)) || !Array.isArray(value.rows) || !Array.isArray(value.unavailable)
    || !value.unavailable.every(note => object(note) && typeof note.kind === 'string' && typeof note.label === 'string' && typeof note.reason === 'string'
      && (note.count === null || (int(note.count) && note.count >= 0)))
    || !Array.isArray(value.warnings) || !value.warnings.every(warning => typeof warning === 'string')) {
    throw new Error('The event feed span reply is malformed.');
  }
  const rows = value.rows.map(validateFeedRow);
  if (rows.some((row, at) => at > 0 && row.hour < rows[at - 1].hour)) throw new Error('The event feed span reply is malformed.');
  return { ...(value as unknown as EventFeedSpanReply), rows };
}

export type Facet = BoundFacetFilter;

export function feedFacets(kinds: readonly FeedKindCode[], categories: ReadonlySet<string>): Facet[] | null {
  const values = kinds.filter(kind => categories.has(kind.category)).map(kind => kind.code);
  return values.length ? [{ name: 'kind', values, countries: [] }] : null;
}

export const eventCountryHour = (event: EventFeedRow): number =>
  event.kind === 'ceasedToExist' || event.kind === 'factionLeft' ? event.nameHour : event.hour;

const eventTags = (event: EventFeedRow): string[] => event.cause?.ally ? [...event.tags, event.cause.ally] : event.tags;

export const rowMatches = (event: EventFeedRow, country: (tag: string, hour: number) => boolean): boolean =>
  eventTags(event).some(tag => country(tag, eventCountryHour(event)));

export function familyRows(span: EventFeedSpanReply, categories: ReadonlySet<string>, firstHour: number | null,
  country: ((tag: string, hour: number) => boolean) | null): LogRow<FeedEvent>[] {
  return span.rows.filter(row => categories.has(row.category) && (firstHour === null || row.hour >= firstHour) && (!country || rowMatches(row, country)))
    .map(row => feedLogRow(row, `family:${row.id}`));
}

export function spanNotes(span: EventFeedSpanReply): string[] {
  return [...span.warnings, ...notRecorded(span.unavailable)];
}

function notRecorded(unavailable: readonly EventFeedUnavailable[]): string[] {
  return unavailable.map(item => `${item.label}: Not recorded — ${item.reason}${item.count === null ? '' : ` (${item.count.toLocaleString()} counted)`}`);
}
