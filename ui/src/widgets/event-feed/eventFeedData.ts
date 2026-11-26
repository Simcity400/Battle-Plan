import { drawableIcon } from '../../dashboard/artwork.ts';
import type { ProductRequest } from '../../dashboard/client';
import { int, object, opaqueTag } from '../../dashboard/validation.ts';
import type { EventFeedItem, EventFeedKindInfo, EventFeedRow, ScriptEdge } from '../../generated/dashboard';
import { productRequest } from '../../dashboard/request.ts';

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
  && optionalText(value.nameMissing) && optionalText(value.gaps)
  && (value.options === undefined || (Array.isArray(value.options) && value.options.every(option => object(option) && int(option.ordinal) && text(option.name))));
const LISTED_KINDS = new Set(['ceasedToExist', 'guaranteeGranted']);
const listed = (row: Record<string, unknown>): boolean => Array.isArray(row.participants) && row.participants.length === 2
  && row.participants.every(value => value === null || opaqueTag(value))
  && JSON.stringify(row.participants.filter(value => value !== null)) === JSON.stringify(row.tags);
const roles = (row: Record<string, unknown>): boolean => {
  if (NATIVE_KINDS.has(row.kind as string)) return row.exact === true && listed(row);
  if (LISTED_KINDS.has(row.kind as string)) return listed(row) && (row.kind === 'guaranteeGranted' || (row.participants as unknown[])[1] === null);
  return row.participants === undefined;
};
const cause = (row: Record<string, unknown>): boolean => {
  const value = row.cause;
  if (value === undefined) return true;
  return CAUSED_KINDS.has(row.kind as string) && object(value) && optionalText(value.how) && optionalText(value.within) && optionalText(value.conference) && optionalText(value.reason)
    && (value.state === null || value.state === undefined || (int(value.state) && value.state > 0)) && (value.ally === null || value.ally === undefined || opaqueTag(value.ally))
    && Array.isArray(value.notes) && value.notes.every(note => typeof note === 'string')
    && (value.selectedOption === undefined || int(value.selectedOption)) && typeof value.targetRecorded === 'boolean';
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
    || (!NATIVE_KINDS.has(value.kind) && !LISTED_KINDS.has(value.kind) && value.kind !== 'eventWindowPresented' && value.tags.length === 0) || !roles(value) || !cause(value) || !strike(value) || !decisionState(value)
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
    || !value.kinds.every(kind => object(kind) && typeof kind.kind === 'string' && KINDS.has(kind.kind) && typeof kind.category === 'string' && CATEGORY_NAMES.has(kind.category))) {
    throw new Error('The event feed kinds reply is malformed.');
  }
  return value.kinds as unknown as EventFeedKindInfo[];
}

const KIND_LABELS: Readonly<Record<string, string>> = {
  warStarted: 'War started',
  warDeclared: 'War declared',
  warJoined: 'Joined a war',
  civilWarWarTransferred: 'Civil war transferred',
  annexed: 'Annexed',
  warEnded: 'War ended',
  released: 'Released',
  stateTransferred: 'State transferred',
  peaceConferenceStarted: 'Peace conference',
  peace: 'Peace',
  capitulated: 'Capitulated',
  ceasedToExist: 'Ceased to exist',
  factionJoined: 'Joined faction',
  factionLeft: 'Left faction',
  becameSubject: 'Became subject',
  guaranteeGranted: 'Guaranteed',
  rulingPartyChanged: 'New government',
  lawChanged: 'Law changed',
  advisorHired: 'Advisor hired',
  focusCompleted: 'Focus completed',
  projectFinished: 'Project finished',
  nukesIncreased: 'Nuclear stockpile',
  nuclearStrike: 'Nuclear strike',
  scriptEventFired: 'Country events',
  newsEventFired: 'News events',
  eventOptionSelected: 'Event choices',
  decisionSelected: 'Decision taken',
  decisionActivated: 'Mission activated',
  decisionCompleted: 'Decision completed',
  decisionCancelled: 'Decision cancelled',
  decisionTimedOut: 'Decision timed out',
  decisionRemoved: 'Decision removed',
};

export function feedKindLabel(kind: string): string {
  const known = KIND_LABELS[kind];
  if (known) return known;
  const spaced = kind.replace(/([A-Z])/g, ' $1').trim();
  return spaced ? spaced[0]!.toUpperCase() + spaced.slice(1).toLowerCase() : kind;
}

export function effectiveEventItem(event: EventFeedRow, choice: ScriptEdge | null): EventFeedItem | null {
  const base = event.item;
  const fallback = choice?.row.item ?? null;
  if (!base) return fallback;
  if ((!base.options || base.options.length === 0) && fallback?.options?.length) {
    return { ...base, options: fallback.options };
  }
  return base;
}
