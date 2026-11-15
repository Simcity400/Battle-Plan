/** Event choices and Decisions page the same cards (`event-choices`, `decisions`): each an opening occurrence (a dispatch,
 * a selection) with the rows continuing it, every one at its own date and read. Pure: validation, the page facets, the
 * cut of a card at the head, recording breaks and day wording shared by both widgets. */
import type { LogPage, LogRow } from '../../charts/log/logData';
import type { LogReplyRow } from '../../charts/log/logSource';
import { formatDate, formatDateTime, gameDate } from '../../dashboard/calendar.ts';
import { int, object } from '../../dashboard/validation.ts';
import { validateFeedRow } from './eventFeedData.ts';
import type { Facet } from './eventFeedData';
import type { EventFeedItem, EventFeedKind, FeedKindCode, ScriptEdge, ScriptOccurrence } from '../../generated/dashboard';

const CURRENCIES = new Set(['politicalPower', 'commandPower']);
const RAW = /^-?\d+$/;
const optional = (value: unknown): value is number | null => value === null || int(value);
const costs = (value: unknown): boolean => value === null || (Array.isArray(value)
  && value.every(cost => object(cost) && typeof cost.currency === 'string' && CURRENCIES.has(cost.currency) && typeof cost.amount === 'string' && RAW.test(cost.amount)));

/** A card as a page sends it: its edges, each a feed row with the operands its wording needs. */
export function validateOccurrence(value: unknown): ScriptOccurrence {
  if (!object(value) || typeof value.id !== 'string' || value.id.length === 0 || typeof value.opened !== 'boolean' || !Array.isArray(value.edges)
    || value.edges.length === 0 || !value.edges.every(edge => object(edge) && optional(edge.option) && optional(edge.lifecycle) && optional(edge.days) && costs(edge.costs))) {
    throw new Error('The occurrence card is malformed.');
  }
  const edges = (value.edges as Record<string, unknown>[]).map(edge => ({ ...(edge as unknown as ScriptEdge), row: validateFeedRow(edge.row) }));
  return { id: value.id, opened: value.opened, edges };
}

/** A card page row's occurrence; null for a row the list does not show. */
export const occurrenceOf = (reply: LogReplyRow): ScriptOccurrence | null =>
  reply.fields.occurrence === null || reply.fields.occurrence === undefined ? null : validateOccurrence(reply.fields.occurrence);

/** The page facets: the rows of the opening kinds chosen and of the kinds continuing a card, that continue nothing (a card's
 * first row: its opener, or a continuing row whose opening was not recorded). */
export function cardFacets(kinds: readonly FeedKindCode[], openers: readonly EventFeedKind[], continuing: readonly EventFeedKind[]): Facet[] {
  const wanted = new Set<EventFeedKind>([...openers, ...continuing]);
  return [{ name: 'kind', values: kinds.filter(kind => wanted.has(kind.kind)).map(kind => kind.code), tags: [] },
    // A row continuing nothing has no opener read hour: its facet holds the none value.
    { name: 'opener', values: [-1], tags: [] }];
}

/** The loaded pages' cards as `at` shows each at the head (null hides it), each page counted by them. */
export function cutPages<T, U>(pages: readonly LogPage<T>[], at: (row: LogRow<T>) => LogRow<U> | null): readonly LogPage<U>[] {
  return pages.map(page => {
    if (!page.rows) return page as unknown as LogPage<U>;
    const rows = page.rows.flatMap(row => at(row) ?? []);
    return { ...page, rows, count: page.count - (page.rows.length - rows.length) };
  });
}

/** The first item any edge of a card names. */
export function occurrenceItem(occurrence: ScriptOccurrence): EventFeedItem | null {
  for (const edge of occurrence.edges) if (edge.row.item) return edge.row.item;
  return null;
}

/** The edges the head has read: each appears at its own read. */
export const edgesAt = (edges: readonly ScriptEdge[], hour: number): ScriptEdge[] => edges.filter(edge => edge.row.readHour <= hour);

/** The last edge of these kinds. */
export function lastEdge(edges: readonly ScriptEdge[], kinds: readonly EventFeedKind[]): ScriptEdge | null {
  for (let index = edges.length - 1; index >= 0; index--) if (kinds.includes(edges[index].row.kind)) return edges[index];
  return null;
}

/** A card matches chosen countries through any of its countries at its own row's hour. */
export const occurrenceMatches = (occurrence: ScriptOccurrence, country: (tag: string, hour: number) => boolean): boolean =>
  occurrence.edges.some(edge => edge.row.tags.some(tag => country(tag, edge.row.hour)));

/** Names and flags only for drawn cards, each country at its own row's hour. */
export function occurrenceCountryRequests(occurrences: readonly ScriptOccurrence[]): { hour: number; tag: string }[] {
  const wanted = new Map<string, { hour: number; tag: string }>();
  for (const occurrence of occurrences) for (const edge of occurrence.edges) for (const tag of edge.row.tags) {
    const key = JSON.stringify([edge.row.nameHour, tag]);
    if (!wanted.has(key)) wanted.set(key, { hour: edge.row.nameHour, tag });
  }
  return [...wanted.values()];
}

/** The hour a card list reads its cards at while following: the head, held while no card it holds changes there. A card
 * changes where an edge is read, a recording break begins, or (open) a day turns; the key is taken at most four times a
 * second of playback. */
export function cardsKey(edges: readonly number[], breaks: readonly RecordingBreak[], days: boolean, hour: number, step: number): string {
  const at = step > 1 ? Math.floor(hour / step) * step : hour;
  const read = edges.filter(edge => edge <= at).length;
  const broken = breaks.filter(span => span.firstHour <= at).length;
  return `${read}|${broken}|${days ? Math.floor(at / 24) : ''}`;
}

/** A stretch the recording does not cover: a capture gap, or reads whose events are incomplete because the recorder was
 * detached between them. */
export interface RecordingBreak { firstHour: number; lastHour: number }
export function recordingBreaks(branch: { gaps: readonly { firstHour: number; lastHour: number }[];
  eventLosses?: readonly { afterHour: number; hour: number }[] }): RecordingBreak[] {
  return [...branch.gaps.map(gap => ({ firstHour: gap.firstHour, lastHour: gap.lastHour })),
    ...(branch.eventLosses ?? []).map(loss => ({ firstHour: loss.afterHour + 1, lastHour: loss.hour }))]
    .filter(span => span.firstHour <= span.lastHour).sort((a, b) => a.firstHour - b.firstHour || a.lastHour - b.lastHour);
}

/** The first break after the read `from`, up to `to`: what happened from then on may be unrecorded. */
export function breakAfter(breaks: readonly RecordingBreak[], from: number, to: number): RecordingBreak | null {
  return breaks.find(span => span.lastHour > from && span.firstHour <= to) ?? null;
}

/** A row's day, as the logs date it. */
export const dayOf = (hour: number): string => { const date = gameDate(hour); return date ? formatDate(date) : formatDateTime(hour); };

/** Whole calendar days from one hour to a later one. */
export const daysBetween = (from: number, to: number): number => Math.floor(to / 24) - Math.floor(from / 24);
export const daysText = (days: number): string => `${days.toLocaleString()} ${days === 1 ? 'day' : 'days'}`;

/** A card's last read hour before a break, the date its "by" bound names. */
export const beforeBreak = (span: RecordingBreak, from: number): number => Math.max(from, span.firstHour - 1);
