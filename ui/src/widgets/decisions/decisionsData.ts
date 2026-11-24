import type { LogRow } from '../../charts/log/logData';
import type { LogReplyRow } from '../../charts/log/logSource';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { formatPowerChange, formatSpent } from '../../dashboard/powerDecimals.ts';
import { beforeBreak, breakAfter, dayOf, daysBetween, daysText, edgesAt, lastEdge, occurrenceItem, occurrenceMatches, occurrenceOf } from '../event-feed/scriptOccurrences.ts';
import type { RecordingBreak } from '../event-feed/scriptOccurrences';
import { plainScripted, reasonRows } from '../event-feed/scriptedGaps.ts';
import type { EventFeedItem, EventFeedKind, ScriptCurrency, ScriptEdge, ScriptOccurrence } from '../../generated/dashboard';

export const NAME_UNAVAILABLE = 'Decision name not recorded';

export const decisionText = (card: DecisionCard) => plainScripted(decisionName(card));

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
const EDGE_LABELS: Partial<Record<EventFeedKind, string>> = { decisionSelected: 'Selected', decisionActivated: 'Activated',
  decisionDurationChanged: 'Duration changed', decisionCompleted: 'Completed', decisionCancelled: 'Cancelled', decisionTimedOut: 'Timed out',
  decisionRemoved: 'Removed', decisionCompletionEffect: 'Completion effects ran', decisionCancellationEffect: 'Cancellation effects ran',
  decisionTimeoutEffect: 'Timeout effects ran', decisionRemovalEffect: 'Removal effects ran' };
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

export function decisionRow(reply: LogReplyRow): LogRow<DecisionCard | null> {
  const card = decisionCard(occurrenceOf(reply));
  const first = card?.occurrence.edges[0].row;
  return { key: reply.key, id: card?.occurrence.id ?? reply.key, hour: first?.hour ?? reply.hour, revealedHour: first?.readHour ?? reply.hour, by: false, item: card };
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
  let failedAt: number | null = null;
  let unsupportedState: number | null = null;
  let failureMissing: string | null = null;
  let status: DecisionStatus = end ? END_STATUS[end.row.kind]!
    : timer || edges.some(edge => edge.row.kind === 'decisionActivated' || edge.row.kind === 'decisionDurationChanged') ? 'active' : 'taken';
  if (state?.kind === 'failed') {
    status = 'failed';
    failedAt = state.observedHour;
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
export const statusTone = (at: DecisionAt): 'accent' | 'positive' | 'warning' | 'neutral' =>
  at.status === 'active' ? 'accent' : at.status === 'completed' ? 'positive' : at.status === 'failed' ? 'warning' : 'neutral';

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
      return `${at.timer.days.toLocaleString()}-day timer${left === null ? '' : ` · ${daysText(left)} left`}`;
    }
    case 'taken': return started ?? 'Taken';
    case 'completed': return `${started ? `${started} → completed ${dayOf(at.end!.row.hour)}` : `Completed ${dayOf(at.end!.row.hour)}`}${missing}`;
    case 'failed': {
      if (at.end?.row.kind === 'decisionTimedOut') return `Failed · ${after('Timed out')}`;
      if (at.end?.row.kind === 'decisionCancelled') return `Failed · ${after('Cancelled')}`;
      return at.start ? `${started} → failed ${dayOf(at.failedAt!)}` : `Failed ${dayOf(at.failedAt!)}`;
    }
    case 'cancelled': return `${after('Cancelled')}${missing}`;
    case 'timedOut': return `${after('Timed out')}${missing}`;
    case 'removed': return `Removed by the game${missing}`;
    case 'unsupported': return `Unsupported native state ${at.unsupportedState}`;
    case 'unknown': return `${started ?? 'Running'} · no end recorded by ${dayOf(beforeBreak(at.pause!, at.edges.at(-1)!.row.readHour))}`;
  }
}

const UNITS: Record<ScriptCurrency, string> = { politicalPower: 'PP', commandPower: 'CP' };

export function costText(at: DecisionAt): string | null {
  const proven = at.edges.filter(edge => edge.costs !== null);
  if (!proven.length) return null;
  const totals = new Map<ScriptCurrency, bigint>();
  for (const edge of proven) for (const cost of edge.costs!) totals.set(cost.currency, (totals.get(cost.currency) ?? 0n) + BigInt(cost.amount));
  if (!totals.size) return 'None';
  return (['politicalPower', 'commandPower'] as const).filter(currency => totals.has(currency)).map(currency => {
    const raw = totals.get(currency)!;
    return `${raw < 0n ? formatSpent(-raw, 0) : formatPowerChange(raw, 0)} ${UNITS[currency]}`;
  }).join(', ');
}

export function decisionDescription(at: DecisionAt, name: (tag: string) => string): string {
  const country = at.card.country === null ? 'A country not identified' : name(at.card.country);
  return `${country}, ${decisionText(at.card)}, ${statusLabel(at).toLowerCase()}, ${lifecycleLine(at)}, cost ${costText(at) ?? 'not recorded'}`;
}

export function decisionTooltip(at: DecisionAt, name: (tag: string) => string): TooltipContent[] {
  const { card } = at;
  const rows: TooltipRow[] = [{ label: 'Country', value: card.country === null ? 'Not identified' : name(card.country) }];
  if (card.target) rows.push({ label: 'Target', value: name(card.target) });
  rows.push({ label: 'Status', value: statusLabel(at), tone: at.status === 'unknown' || at.status === 'unsupported' ? 'muted' : undefined });
  if (at.failedAt !== null) rows.push({ label: 'Failure observed', value: formatDateTime(at.failedAt) });
  if (at.failureMissing) rows.push({ label: 'Mission failure state', value: `Not recorded — ${at.failureMissing}`, tone: 'muted' });
  for (const edge of at.edges) {
    const label = EDGE_LABELS[edge.row.kind];
    if (label) rows.push({ label, value: `${formatDateTime(edge.row.hour)}${edge.row.kind === 'decisionDurationChanged' && edge.days !== null ? ` · ${daysText(edge.days)}` : ''}` });
  }
  if (at.timer) rows.push({ label: 'Timer', value: daysText(at.timer.days) });
  const left = daysLeft(at);
  if (left !== null) rows.push({ label: 'Days left', value: left.toLocaleString() });
  if (at.pause) rows.push({ label: 'Recording paused', value: `${formatDateTime(at.pause.firstHour)} to ${formatDateTime(at.pause.lastHour)}`, tone: 'muted' });
  const cost = costText(at);
  rows.push(cost === null ? { label: 'Cost', value: 'Not recorded', tone: 'muted' } : { label: 'Cost', value: cost });
  if (card.item?.token) rows.push({ label: 'Content identity', value: card.item.token });
  rows.push(...reasonRows(card.item));
  if (card.item?.description) rows.push({ label: plainScripted(card.item.description) });
  return [{ title: decisionText(card), icon: card.item?.icon ?? undefined, rows }];
}

export const decisionJumps = (at: DecisionAt): number[] => [...new Set([at.card.occurrence.edges[0].row.hour, at.end?.row.hour, at.failedAt]
  .filter((hour): hour is number => hour !== null && hour !== undefined))];

export const activeCount = (rows: readonly LogRow<DecisionAt>[]) => rows.filter(row => row.item.status === 'active').length;

export function decisionDiagnostics(cards: readonly DecisionCard[]): string[] {
  const names = new Map<string, number>();
  for (const card of cards) {
    if (card.item?.name) continue;
    const key = card.item?.token ?? 'a decision not identified';
    names.set(key, (names.get(key) ?? 0) + 1);
  }
  return names.size ? [`${NAME_UNAVAILABLE}: ${[...names.keys()].sort().join(', ')} (${[...names.values()].reduce((a, b) => a + b, 0).toLocaleString()} counted)`] : [];
}
