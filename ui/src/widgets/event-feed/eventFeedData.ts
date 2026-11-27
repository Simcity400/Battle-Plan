import { drawableIcon } from '../../dashboard/artwork.ts';
import type { ProductRequest } from '../../dashboard/client';
import { int, object, opaqueTag } from '../../dashboard/validation.ts';
import { EVENT_FEED_ROW_LABELS, type EventFeedItem, type EventFeedKind, type EventFeedKindInfo, type EventFeedRow, type ScriptEdge } from '../../generated/dashboard.ts';
import { productRequest } from '../../dashboard/request.ts';
import { isEventFeedGap } from './scriptedGaps.ts';
import type { LogKindFilterItem } from '../../families/table/pickerList.ts';

const SCRIPT_KINDS = new Set(['scriptEventFired', 'newsEventFired', 'eventOptionSelected', 'decisionSelected', 'decisionRemoved', 'decisionCancelled',
  'decisionTimedOut', 'decisionActivated', 'decisionCompleted', 'decisionReenabled', 'decisionRemovalEffect', 'decisionCancellationEffect',
  'decisionCompletionEffect', 'decisionTimeoutEffect', 'decisionDurationChanged']);
const NATIVE_KINDS = new Set([...SCRIPT_KINDS, 'warDeclared', 'warJoined', 'civilWarWarTransferred', 'annexed', 'warEnded', 'released',
  'stateTransferred', 'peaceConferenceStarted', 'nuclearStrike']);
const CAUSED_KINDS = new Set([...NATIVE_KINDS].filter(kind => kind !== 'nuclearStrike'));
const KINDS = new Set([...NATIVE_KINDS, 'warStarted', 'peace', 'capitulated', 'ceasedToExist', 'factionJoined', 'factionLeft', 'becameSubject',
  'guaranteeGranted', 'rulingPartyChanged', 'lawChanged', 'advisorHired', 'focusCompleted', 'projectFinished', 'nukesIncreased', 'nuclearStrike',
  'eventWindowPresented']);
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
const item = (value: unknown): value is EventFeedItem => object(value) && text(value.token) && text(value.name) && text(value.icon) && (value.description === undefined || text(value.description))
  && optionalText(value.nameMissing) && (value.gaps === undefined || (Array.isArray(value.gaps) && value.gaps.every(isEventFeedGap)))
  && (value.options === undefined || (Array.isArray(value.options) && value.options.every(option => object(option) && int(option.ordinal) && text(option.name))));
const LISTED_KINDS = new Set(['ceasedToExist', 'guaranteeGranted', 'projectFinished']);
const TRANSITION_KINDS = new Set(['peace', 'factionJoined', 'factionLeft']);
const listed = (row: Record<string, unknown>): boolean => Array.isArray(row.participants) && row.participants.length === 2
  && row.participants.every(value => value === null || opaqueTag(value))
  && JSON.stringify(row.participants.filter(value => value !== null)) === JSON.stringify(row.tags);
const roles = (row: Record<string, unknown>): boolean => {
  if (row.kind === 'warStarted' || NATIVE_KINDS.has(row.kind as string)) return row.exact === true && listed(row);
  if (LISTED_KINDS.has(row.kind as string)) return listed(row) && (row.kind === 'guaranteeGranted' || (row.participants as unknown[])[1] === null);
  if (TRANSITION_KINDS.has(row.kind as string)) return row.participants === undefined || (listed(row) && (row.kind === 'peace' || (row.participants as unknown[])[1] === null));
  return row.participants === undefined;
};
const participantsMissing = (row: Record<string, unknown>): boolean => {
  const kind = row.kind as string;
  if (kind !== 'warStarted' && !(TRANSITION_KINDS.has(kind) && row.participants !== undefined)) return row.participantsMissing === undefined;
  return Array.isArray(row.participantsMissing) && row.participantsMissing.length === 2 && Array.isArray(row.participants)
    && row.participantsMissing.every((reason, index) => (kind === 'warStarted' || kind === 'peace' || index === 0) && (row.participants as unknown[])[index] === null
      ? typeof reason === 'string' && reason.length > 0 : reason === null);
};
const ownerMissing = (row: Record<string, unknown>): boolean => {
  if (row.kind === 'projectFinished' && Array.isArray(row.participants) && row.participants[0] === null) {
    return typeof row.ownerMissing === 'string' && row.ownerMissing.length > 0;
  }
  return row.ownerMissing === undefined;
};
const cause = (row: Record<string, unknown>): boolean => {
  const value = row.cause;
  if (value === undefined) return true;
  return CAUSED_KINDS.has(row.kind as string) && object(value) && optionalText(value.how) && optionalText(value.within) && optionalText(value.conference) && optionalText(value.reason)
    && (value.state === null || value.state === undefined || (int(value.state) && value.state > 0)) && (value.ally === null || value.ally === undefined || opaqueTag(value.ally))
    && Array.isArray(value.notes) && value.notes.every(note => typeof note === 'string')
    && (value.selectedOption === undefined || int(value.selectedOption))
    && (value.target === undefined || (object(value.target) && (
      value.target.kind === 'country'
      || (value.target.kind === 'state' && (value.target.id === null || (int(value.target.id) && value.target.id > 0)))
    )));
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
    || value.hour > value.readHour || value.nameHour > value.readHour || typeof value.exact !== 'boolean' || typeof value.category !== 'string' || !CATEGORY_NAMES.has(value.category)
    || typeof value.kind !== 'string' || !KINDS.has(value.kind) || !Array.isArray(value.tags) || !value.tags.every(opaqueTag)
    || (!NATIVE_KINDS.has(value.kind) && !LISTED_KINDS.has(value.kind) && !(TRANSITION_KINDS.has(value.kind) && value.participants !== undefined) && value.kind !== 'eventWindowPresented' && value.tags.length === 0) || !roles(value) || !participantsMissing(value) || !ownerMissing(value) || !cause(value) || !strike(value) || !decisionState(value)
    || !(value.item === null || item(value.item)) || !(value.amount == null || (int(value.amount) && value.amount > 0))) {
    throw new Error('The event feed row is malformed.');
  }
  const row = value as unknown as EventFeedRow;
  const icon = row.item ? drawableIcon(row.item.icon) : null;
  return row.item && icon !== row.item.icon ? { ...row, item: { ...row.item, icon } } : row;
}

export function kindsRequest(track: { source: { id: string; revision: string }; branch: { id: string } }): ProductRequest {
  return productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'event-feed-kinds', params: { branchId: track.branch.id } });
}

export function validateKinds(value: unknown): readonly EventFeedKindInfo[] {
  if (!object(value) || !Array.isArray(value.kinds)
    || !value.kinds.every(kind => object(kind) && typeof kind.kind === 'string' && KINDS.has(kind.kind) && typeof kind.category === 'string' && CATEGORY_NAMES.has(kind.category)
      && (int(kind.firstObserved) && kind.unavailable === null
        || kind.firstObserved === null && typeof kind.unavailable === 'string' && kind.unavailable.length > 0))) {
    throw new Error('The event feed kinds reply is malformed.');
  }
  return value.kinds as unknown as EventFeedKindInfo[];
}

const KIND_LABEL_OVERRIDES: Readonly<Partial<Record<EventFeedKind, string>>> = {
  warStarted: 'War started',
  warEnded: 'War ended',
  scriptEventFired: 'Country events',
  newsEventFired: 'News events',
  eventOptionSelected: 'Event choices',
  decisionSelected: 'Decision taken',
  decisionRemovalEffect: 'Decision removal effect',
  decisionCancellationEffect: 'Decision cancellation effect',
  decisionCompletionEffect: 'Decision completion effect',
  decisionTimeoutEffect: 'Decision timeout effect',
  decisionDurationChanged: 'Decision duration changed',
};

export function feedKindLabel(kind: EventFeedKind): string {
  const known = KIND_LABEL_OVERRIDES[kind] ?? EVENT_FEED_ROW_LABELS[kind];
  if (known) return known;
  const spaced = kind.replace(/([A-Z])/g, ' $1').trim();
  return spaced ? spaced[0]!.toUpperCase() + spaced.slice(1).toLowerCase() : kind;
}

export function feedKindItems(kinds: readonly EventFeedKindInfo[], categories: readonly { value: string; label: string }[], enabled: readonly string[], excluded: readonly string[], hour: number | null): LogKindFilterItem[] {
  return kinds.flatMap(item => {
    const checked = item.firstObserved !== null && enabled.includes(item.category) && !excluded.includes(item.kind);
    const future = item.firstObserved !== null && (hour === null || item.firstObserved > hour);
    if (future && !checked) return [];
    const unavailable = item.unavailable ?? (future ? 'No events of this kind recorded by the shown hour.' : undefined);
    return [{ value: `kind:${item.kind}`, label: feedKindLabel(item.kind), group: categories.find(category => category.value === item.category)?.label ?? item.category,
      checked, ...(unavailable ? { unavailable } : {}) }];
  });
}

export function effectiveEventItem(event: EventFeedRow, choice: ScriptEdge | null): EventFeedItem | null {
  const base = event.item;
  const fallback = choice?.row.item ?? null;
  if (!base) return fallback;
  if ((!base.options || base.options.length === 0) && fallback?.options?.length) {
    const optionGaps = fallback.gaps?.filter(gap => gap.field === 'option') ?? [];
    const gaps = [...(base.gaps ?? []), ...optionGaps];
    return { ...base, options: fallback.options, ...(gaps.length ? { gaps } : {}) };
  }
  return base;
}
