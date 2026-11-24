import { drawableIcon } from '../../dashboard/artwork.ts';
import { int, object, opaqueTag } from '../../dashboard/validation.ts';
import type { EventFeedItem, EventFeedRow } from '../../generated/dashboard';

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
    || (!NATIVE_KINDS.has(value.kind) && value.kind !== 'eventWindowPresented' && value.tags.length === 0) || !roles(value) || !cause(value) || !strike(value) || !decisionState(value)
    || !(value.item === null || item(value.item)) || !(value.amount === null || (int(value.amount) && value.amount > 0))) {
    throw new Error('The event feed row is malformed.');
  }
  const row = value as unknown as EventFeedRow;
  const icon = row.item ? drawableIcon(row.item.icon) : null;
  return row.item && icon !== row.item.icon ? { ...row, item: { ...row.item, icon } } : row;
}

