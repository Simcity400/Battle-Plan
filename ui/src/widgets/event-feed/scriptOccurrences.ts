import { formatDate, formatDateTime, gameDate } from '../../dashboard/calendar.ts';
import { int, object } from '../../dashboard/validation.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import { validateFeedRow } from './eventFeedData.ts';
import type { EventFeedItem, EventFeedKind, ScriptEdge, ScriptOccurrence } from '../../generated/dashboard';

const CURRENCIES = new Set(['politicalPower', 'commandPower']);
const RAW = /^-?\d+$/;
const optional = (value: unknown): value is number | null | undefined => value == null || int(value);
const costs = (value: unknown): boolean => value == null || (Array.isArray(value)
  && value.every(cost => object(cost) && typeof cost.currency === 'string' && CURRENCIES.has(cost.currency) && typeof cost.amount === 'string' && RAW.test(cost.amount)));

export function validateScriptEdge(value: unknown): ScriptEdge {
  if (!object(value) || !optional(value.option) || !optional(value.lifecycle) || !optional(value.days) || !costs(value.costs)
    || (value.costsUnread !== undefined && (!Array.isArray(value.costsUnread) || !value.costsUnread.every(currency => typeof currency === 'string' && CURRENCIES.has(currency))))) {
    throw new Error('The occurrence card is malformed.');
  }
  return { ...(value as unknown as ScriptEdge), row: validateFeedRow(value.row) };
}

export function validateOccurrence(value: unknown): ScriptOccurrence {
  if (!object(value) || typeof value.id !== 'string' || value.id.length === 0 || typeof value.opened !== 'boolean' || !Array.isArray(value.edges) || value.edges.length === 0) {
    throw new Error('The occurrence card is malformed.');
  }
  return { id: value.id, opened: value.opened, edges: value.edges.map(validateScriptEdge) };
}

export function occurrenceItem(occurrence: ScriptOccurrence): EventFeedItem | null {
  for (const edge of occurrence.edges) if (edge.row.item) return edge.row.item;
  return null;
}

export const edgesAt = (edges: readonly ScriptEdge[], hour: number): ScriptEdge[] => edges.filter(edge => edge.row.readHour <= hour);

export function lastEdge(edges: readonly ScriptEdge[], kinds: readonly EventFeedKind[]): ScriptEdge | null {
  for (let index = edges.length - 1; index >= 0; index--) if (kinds.includes(edges[index].row.kind)) return edges[index];
  return null;
}

export interface RecordingBreak { firstHour: number; lastHour: number }
export function recordingBreaks(branch: { gaps: readonly { firstHour: number; lastHour: number }[];
  eventLosses?: readonly { afterHour: number; hour: number }[] }): RecordingBreak[] {
  return [...branch.gaps.map(gap => ({ firstHour: gap.firstHour, lastHour: gap.lastHour })),
    ...(branch.eventLosses ?? []).map(loss => ({ firstHour: loss.afterHour + 1, lastHour: loss.hour }))]
    .filter(span => span.firstHour <= span.lastHour).sort((a, b) => a.firstHour - b.firstHour || a.lastHour - b.lastHour);
}

export function breakAfter(breaks: readonly RecordingBreak[], from: number, to: number): RecordingBreak | null {
  return breaks.find(span => span.lastHour > from && span.firstHour <= to) ?? null;
}

export const dayOf = (hour: number): string => { const date = gameDate(hour); return date ? formatDate(date) : formatDateTime(hour); };

export const daysBetween = (from: number, to: number): number => Math.floor(to / 24) - Math.floor(from / 24);
export const daysText = (days: number): string => `${formatScalar(days, 'integer')} ${days === 1 ? 'day' : 'days'}`;

export const beforeBreak = (span: RecordingBreak, from: number): number => Math.max(from, span.firstHour - 1);
