/** Event choices and Decisions read the same prepared occurrence pages: each
 * card is an opening fact (a dispatch, a selection) with the facts continuing
 * it, every one at its own native date and supplying read. Pure: validation,
 * playback cuts, recording breaks and day wording shared by both widgets. */
import { formatDate, formatDateTime, gameDate } from '../../dashboard/calendar.ts';
import { preparedPrefix } from '../../dashboard/preparedPrefix.ts';
import type { PreparedPrefixFields } from '../../dashboard/preparedPrefix';
import { int, object } from '../../dashboard/validation.ts';
import { notRecorded, validateEventFeedReply } from './eventFeedData.ts';
import type { EventFeedItem, EventFeedKind, ScriptEdge, ScriptOccurrence, ScriptOccurrenceReply } from '../../generated/dashboard';

/** A page of occurrences; `prefix` while it is answered from the prepared
 * prefix, backwards from that prefix's last hour (as the event feed's pages). */
export interface ScriptPage { reply: ScriptOccurrenceReply; nextCursor: string | null; prefix?: PreparedPrefixFields | null }

const CURRENCIES = new Set(['politicalPower', 'commandPower']);
const RAW = /^-?\d+$/;
const optional = (value: unknown): value is number | null => value === null || int(value);
const costs = (value: unknown): boolean => value === null || (Array.isArray(value)
  && value.every(cost => object(cost) && typeof cost.currency === 'string' && CURRENCIES.has(cost.currency)
    && typeof cost.amount === 'string' && RAW.test(cost.amount)));
const options = (item: Record<string, unknown>): boolean => item.options === undefined || (Array.isArray(item.options)
  && item.options.every(option => object(option) && int(option.ordinal) && (option.name === null || typeof option.name === 'string')));

/** Validate a page's occurrences, rows, items and cursor before drawing. Rows,
 * items and missing-capture notes follow the event feed's own validation. */
export function validateScriptPage(value: unknown): ScriptPage {
  const malformed = () => new Error('The prepared occurrence page is malformed.');
  if (!object(value) || !(value.nextCursor === null || typeof value.nextCursor === 'string') || value.nextCursor === '') throw malformed();
  const reply = value.reply;
  if (!object(reply) || !Array.isArray(reply.occurrences) || !reply.occurrences.every(occurrence => object(occurrence)
    && typeof occurrence.id === 'string' && occurrence.id.length > 0 && typeof occurrence.opened === 'boolean'
    && Array.isArray(occurrence.edges) && occurrence.edges.length > 0 && occurrence.edges.every(edge => object(edge) && object(edge.row)
      && optional(edge.option) && optional(edge.lifecycle) && optional(edge.days) && costs(edge.costs)))
    || !Array.isArray(reply.items) || !reply.items.every(item => object(item) && options(item))) throw malformed();
  const occurrences = reply.occurrences as unknown as ScriptOccurrence[];
  let items: EventFeedItem[];
  try {
    // The event feed's checks: identities, dates within the reply, item references, artwork.
    items = validateEventFeedReply({ branchId: reply.branchId, firstHour: reply.firstHour, lastHour: reply.lastHour, windows: [],
      rows: occurrences.flatMap(occurrence => occurrence.edges.map(edge => edge.row)), items: reply.items,
      modifiers: [], texts: [], unavailable: reply.unavailable, warnings: reply.warnings }).items;
  } catch { throw malformed(); }
  const validated = reply as unknown as ScriptOccurrenceReply;
  return { reply: items === validated.items ? validated : { ...validated, items }, nextCursor: value.nextCursor, prefix: preparedPrefix(value) };
}

/** Missing captures, as the event feed reports them. */
export const scriptNotRecorded = (reply: ScriptOccurrenceReply): string[] => notRecorded(reply);

/** An edge's content item, or null when it has none. */
export const edgeItem = (reply: ScriptOccurrenceReply, edge: ScriptEdge): EventFeedItem | null =>
  edge.row.item === null ? null : reply.items[edge.row.item] ?? null;

/** The first item any edge of a card names. */
export function occurrenceItem(reply: ScriptOccurrenceReply, occurrence: ScriptOccurrence): EventFeedItem | null {
  for (const edge of occurrence.edges) { const item = edgeItem(reply, edge); if (item) return item; }
  return null;
}

/** The edges a playback hour has read: each appears at its own supplying read. */
export const edgesAt = (edges: readonly ScriptEdge[], hour: number): ScriptEdge[] => edges.filter(edge => edge.row.readHour <= hour);

/** The last edge of these kinds read by `hour`. */
export function lastEdge(edges: readonly ScriptEdge[], kinds: readonly EventFeedKind[]): ScriptEdge | null {
  for (let index = edges.length - 1; index >= 0; index--) if (kinds.includes(edges[index].row.kind)) return edges[index];
  return null;
}

/** A card matches chosen countries through any of its countries at its own fact's hour. */
export const occurrenceMatches = (occurrence: ScriptOccurrence, country: (tag: string, hour: number) => boolean): boolean =>
  occurrence.edges.some(edge => edge.row.tags.some(tag => country(tag, edge.row.hour)));

/** Names and flags only for drawn cards, each country at its own fact's hour. */
export function occurrenceCountryRequests(occurrences: readonly ScriptOccurrence[]): { hour: number; tag: string }[] {
  const wanted = new Map<string, { hour: number; tag: string }>();
  for (const occurrence of occurrences) for (const edge of occurrence.edges) for (const tag of edge.row.tags) {
    const key = JSON.stringify([edge.row.nameHour, tag]);
    if (!wanted.has(key)) wanted.set(key, { hour: edge.row.nameHour, tag });
  }
  return [...wanted.values()];
}

/** A stretch the recording does not cover: a capture gap, or reads whose
 * events are incomplete because the recorder was detached between them. */
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

/** A fact's day, as the logs date it. */
export const dayOf = (hour: number): string => { const date = gameDate(hour); return date ? formatDate(date) : formatDateTime(hour); };

/** Whole calendar days from one hour to a later one. */
export const daysBetween = (from: number, to: number): number => Math.floor(to / 24) - Math.floor(from / 24);
export const daysText = (days: number): string => `${days.toLocaleString()} ${days === 1 ? 'day' : 'days'}`;

/** A card's last read hour before a break, the date its "by" bound names. */
export const beforeBreak = (span: RecordingBreak, from: number): number => Math.max(from, span.firstHour - 1);
