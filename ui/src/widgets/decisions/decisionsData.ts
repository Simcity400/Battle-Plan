import type { LogRow } from '../../charts/log/logData';
import { formatScalar } from '../../families/valueFormat.ts';
import { beforeBreak, breakAfter, dayOf, daysBetween, daysText, edgesAt, lastEdge, occurrenceItem, occurrenceMatches } from '../event-feed/scriptOccurrences.ts';
import type { RecordingBreak } from '../event-feed/scriptOccurrences';
import type { EventFeedItem, EventFeedKind, ScriptEdge, ScriptOccurrence } from '../../generated/dashboard';

const NAME_UNAVAILABLE = 'Decision name not recorded';

export type DecisionStatus = 'active' | 'taken' | 'completed' | 'failed' | 'cancelled' | 'timedOut' | 'removed' | 'unsupported' | 'unknown';

export const STATUS_CHOICES = [
  { value: 'active', label: 'Active' },
  { value: 'completed', label: 'Completed', hint: 'Includes decisions taken at once.' },
  { value: 'failed', label: 'Failed' },
  { value: 'cancelled', label: 'Cancelled' },
  { value: 'timedOut', label: 'Timed out' },
  { value: 'removed', label: 'Removed' },
] as const;
export type StatusChoice = typeof STATUS_CHOICES[number]['value'];
export const statusChoice = (status: DecisionStatus): StatusChoice =>
  status === 'unknown' ? 'active' : status === 'taken' ? 'completed' : status === 'unsupported' ? 'failed' : status;

const STARTS: readonly EventFeedKind[] = ['decisionSelected', 'decisionActivated'];
const ENDS: readonly EventFeedKind[] = ['decisionCompleted', 'decisionCancelled', 'decisionTimedOut', 'decisionRemoved'];
const TIMERS: readonly EventFeedKind[] = ['decisionSelected', 'decisionActivated', 'decisionDurationChanged'];
const END_STATUS: Partial<Record<EventFeedKind, DecisionStatus>> = { decisionCompleted: 'completed', decisionCancelled: 'cancelled',
  decisionTimedOut: 'timedOut', decisionRemoved: 'removed' };
const STATUS_LABELS: Record<DecisionStatus, string> = { active: 'Active', taken: 'Taken', completed: 'Completed', failed: 'Failed', cancelled: 'Cancelled',
  timedOut: 'Timed out', removed: 'Removed', unsupported: 'Unsupported', unknown: 'Not recorded' };

export interface DecisionCard {
  occurrence: ScriptOccurrence;
  item: EventFeedItem | null;
  country: string | null;
  target: string | null;
}

export interface DecisionAt {
  card: DecisionCard;
  hour: number;
  edges: ScriptEdge[];
  status: DecisionStatus;
  start: ScriptEdge | null;
  end: ScriptEdge | null;
  failedAt: number | null;
  unsupportedState: number | null;
  failureMissing: string | null;
  timer: { days: number; from: number } | null;
  pause: RecordingBreak | null;
}

export const OPENERS: readonly EventFeedKind[] = ['decisionSelected', 'decisionActivated'];
export const CONTINUING: readonly EventFeedKind[] = ['decisionActivated', 'decisionDurationChanged', 'decisionRemovalEffect', 'decisionCancellationEffect',
  'decisionCompletionEffect', 'decisionTimeoutEffect', 'decisionCompleted', 'decisionCancelled', 'decisionTimedOut', 'decisionRemoved'];

export function decisionCard(occurrence: ScriptOccurrence | null): DecisionCard | null {
  if (!occurrence) return null;
  const [country, target] = occurrence.edges[0].row.participants ?? [null, null];
  return { occurrence, item: occurrenceItem(occurrence), country, target: target !== country ? target : null };
}

export function decisionAt(row: LogRow<DecisionCard | null>, hour: number, breaks: readonly RecordingBreak[], statuses: ReadonlySet<string>,
  country: ((tag: string, hour: number) => boolean) | null): LogRow<DecisionAt> | null {
  const card = row.item;
  if (!card || (row.revealedHour ?? row.hour) > hour || (country && !occurrenceMatches(card.occurrence, country))) return null;
  const edges = edgesAt(card.occurrence.edges, hour);
  const start = card.occurrence.opened && STARTS.includes(edges[0]?.row.kind) ? edges[0] : null;
  const latest = lastEdge(edges, [...ENDS, 'decisionActivated']);
  const end = latest && ENDS.includes(latest.row.kind) ? latest : null;
  const timing = edges.filter(edge => TIMERS.includes(edge.row.kind) && edge.days !== null).at(-1);
  const timer = timing && timing.days! > 0 ? { days: timing.days!, from: timing.row.hour } : null;
  const hasLifecycle = (state: ScriptEdge['row']['decisionState']) => state !== undefined && state.kind !== 'notApplicable';
  const latestState = edges.filter(edge => hasLifecycle(edge.row.decisionState)).at(-1)?.row.decisionState ?? null;
  const state = (end && hasLifecycle(end.row.decisionState) ? end.row.decisionState : null) ?? latestState;
  const selected = edges.some(edge => edge.row.kind === 'decisionSelected');
  if (!end && !timer && !selected && state?.kind === 'available') return null;
  let failedAt: number | null = null;
  let unsupportedState: number | null = null;
  let failureMissing: string | null = null;
  let status: DecisionStatus = end ? END_STATUS[end.row.kind]!
    : timer || edges.some(edge => edge.row.kind === 'decisionActivated' || edge.row.kind === 'decisionDurationChanged') ? 'active' : 'taken';
  if (state?.kind === 'failed') {
    status = 'failed';
    failedAt = state.observedHour;
  } else if (state?.kind === 'completed' && !end) {
    status = 'completed';
  } else if (state?.kind === 'aborted' && !end) {
    status = 'cancelled';
  } else if (state?.kind === 'unsupported') {
    status = 'unsupported';
    unsupportedState = state.raw;
  } else if (state?.kind === 'notRecorded') {
    failureMissing = state.reason;
  }
  let pause: RecordingBreak | null = null;
  if (status === 'active') {
    pause = breakAfter(breaks, edges.at(-1)!.row.readHour, hour);
    if (pause) status = 'unknown';
  }
  if (!statuses.has(statusChoice(status))) return null;
  return { ...row, item: { card, hour, edges, status, start, end, failedAt, unsupportedState, failureMissing, timer, pause } };
}

export const decisionName = (card: DecisionCard) => card.item?.name ?? NAME_UNAVAILABLE;
export const statusLabel = (at: DecisionAt) =>
  at.status === 'unsupported' && at.unsupportedState !== null ? `Unsupported (state ${at.unsupportedState})` : STATUS_LABELS[at.status];

export function daysLeft(at: DecisionAt): number | null {
  if (at.status !== 'active' || !at.timer) return null;
  const left = Math.ceil((at.timer.from + at.timer.days * 24 - at.hour) / 24);
  return left > 0 ? left : null;
}

export function lifecycleLine(at: DecisionAt): string {
  const started = at.start ? `${at.start.row.kind === 'decisionActivated' ? 'Activated' : 'Selected'} ${dayOf(at.start.row.hour)}` : null;
  const after = (verb: string) => {
    if (!at.start) return `${verb} ${dayOf(at.end!.row.hour)}`;
    const days = daysBetween(at.start.row.hour, at.end!.row.hour);
    return `${verb} ${days ? `after ${daysText(days)}` : 'the same day'}`;
  };
  const missing = at.failureMissing ? ` · ${at.failureMissing}` : '';
  switch (at.status) {
    case 'active': {
      if (!at.timer) return started ?? 'Running';
      const left = daysLeft(at);
      return `${formatScalar(at.timer.days, 'integer')}-day timer${left === null ? '' : ` · ${daysText(left)} left`}`;
    }
    case 'taken': return started ?? 'Taken';
    case 'completed': return `${at.end ? (started ? `${started} → completed ${dayOf(at.end.row.hour)}` : `Completed ${dayOf(at.end.row.hour)}`) : (started ? `${started} → completed` : 'Completed')}${missing}`;
    case 'failed': {
      if (at.end?.row.kind === 'decisionTimedOut') return `Failed · ${after('Timed out')}`;
      if (at.end?.row.kind === 'decisionCancelled') return `Failed · ${after('Cancelled')}`;
      return at.start ? `${started} → failed ${dayOf(at.failedAt!)}` : `Failed ${dayOf(at.failedAt!)}`;
    }
    case 'cancelled': return `${at.end ? after('Cancelled') : (started ? `${started} → aborted` : 'Aborted')}${missing}`;
    case 'timedOut': return `${after('Timed out')}${missing}`;
    case 'removed': return `Removed by the game${missing}`;
    case 'unsupported': return `Unsupported native state ${at.unsupportedState}`;
    case 'unknown': return `${started ?? 'Running'} · no end recorded by ${dayOf(beforeBreak(at.pause!, at.edges.at(-1)!.row.readHour))}`;
  }
}
