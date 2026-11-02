import type { LogRow } from '../../charts/log/logData';
import { newestFirst } from '../../charts/log/logData.ts';
import type { PlaybackBounds } from '../../charts/playbackIntervals';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { formatDateTime } from '../../dashboard/calendar.ts';
import type { EventFeedItem, EventFeedKind, ScriptEdge, ScriptOccurrence, ScriptOccurrenceReply } from '../../generated/dashboard';
import { beforeBreak, breakAfter, dayOf, daysBetween, daysText, edgesAt, lastEdge, occurrenceItem, occurrenceMatches } from '../event-feed/scriptOccurrences.ts';
import type { RecordingBreak } from '../event-feed/scriptOccurrences';

export const TITLE_UNAVAILABLE = 'Title unavailable';
export const OPTION_UNAVAILABLE = 'Option text unavailable';

/** One recorded delivery of an event to one country, with the option it chose. */
export interface ChoiceCard {
  occurrence: ScriptOccurrence;
  item: EventFeedItem | null;
  /** The edge the card is dated and named by: its firing, else its first choice. */
  first: ScriptEdge;
  /** The delivery, when its firing was recorded. */
  fired: ScriptEdge | null;
  /** The receiving country (or, without a recorded firing, the chooser). */
  country: string | null;
  /** The sending country, when recorded and not the receiver. */
  sender: string | null;
}

/** A card as the reads by the playback hour show it. `pause` is a break in the
 * recording after the firing, while no choice is recorded: a choice then is unknown. */
export interface ChoiceAt {
  card: ChoiceCard;
  hour: number;
  choice: ScriptEdge | null;
  pause: RecordingBreak | null;
}

export interface ChoiceFilter {
  /** The opening kinds shown: country events, news or both. */
  kinds: ReadonlySet<EventFeedKind>;
  country: (tag: string, hour: number) => boolean;
  allCountries: boolean;
  firstHour: number | null;
}

const isChoice = (edge: ScriptEdge) => edge.row.kind === 'eventOptionSelected' && (edge.option === null || edge.option >= 0);

/** The edge a card is dated by: its firing where recorded, else its first choice.
 * Immediate effects with neither are no card: they are part of a dispatch, never a choice. */
const cardEdge = (occurrence: ScriptOccurrence): ScriptEdge | null =>
  occurrence.opened ? occurrence.edges[0] : occurrence.edges.find(isChoice) ?? null;

/** Each page's cards once per filter, newest first by firing date. A choice
 * whose firing was not recorded is its own card. */
export function prepareChoices(reply: ScriptOccurrenceReply, filter: ChoiceFilter): ReadonlyArray<LogRow<ChoiceAt>> {
  return newestFirst(reply.occurrences.flatMap(occurrence => {
    const first = cardEdge(occurrence);
    if (!first) return [];
    const fired = occurrence.opened ? first : null;
    if (fired && !filter.kinds.has(fired.row.kind)) return [];
    if (filter.firstHour !== null && first.row.hour < filter.firstHour) return [];
    if (!filter.allCountries && !occurrenceMatches(occurrence, filter.country)) return [];
    const roles = first.row.participants ?? [null, null];
    const country = fired ? roles[1] : roles[0];
    const sender = fired && roles[0] !== null && roles[0] !== country ? roles[0] : null;
    const card: ChoiceCard = { occurrence, item: occurrenceItem(reply, occurrence), first, fired, country, sender };
    return [{ key: occurrence.id, id: occurrence.id, hour: first.row.hour, revealedHour: first.row.readHour, by: false,
      item: { card, hour: first.row.readHour, choice: null, pause: null } }];
  }));
}

/** A card at the playback hour: shown from its first read, chosen from the read
 * that shows the choice. `awaiting` keeps only cards still awaiting a choice. */
export function choiceAt(row: LogRow<ChoiceAt>, hour: number, breaks: readonly RecordingBreak[], awaiting: boolean): LogRow<ChoiceAt> | null {
  const { card } = row.item;
  if ((row.revealedHour ?? row.hour) > hour) return null;
  const edges = edgesAt(card.occurrence.edges, hour);
  const choice = edges.filter(isChoice).at(-1) ?? null;
  const pause = choice ? null : breakAfter(breaks, row.revealedHour ?? row.hour, hour);
  if (awaiting && (choice || pause)) return null;
  return { ...row, item: { card, hour, choice, pause } };
}

export const isOpen = (at: ChoiceAt) => !at.choice && !at.pause;
export function choicePlaybackBounds(rows: ReadonlyArray<LogRow<ChoiceAt>>, hour: number): PlaybackBounds {
  let first = -Infinity, end = Infinity;
  for (const row of rows) if (isOpen(row.item)) {
    const start = (Math.floor(row.item.card.first.row.hour / 24) + 1) * 24;
    if (hour < start) end = Math.min(end, start);
    else { first = Math.floor(hour / 24) * 24; end = Math.min(end, first + 24); }
  }
  return { first, end };
}
export const choiceTitle = (card: ChoiceCard) => card.item?.name ?? TITLE_UNAVAILABLE;

/** "A", "B", … "Z", "AA": an option's place in the game's order. */
export function optionLetter(index: number): string {
  let letters = '';
  for (let at = index + 1; at > 0; at = Math.floor((at - 1) / 26)) letters = String.fromCharCode(65 + (at - 1) % 26) + letters;
  return letters;
}

export interface OptionLine { key: string; letter: string | null; text: string; missing: boolean; chosen: boolean }

/** Every authored option in the game's order, the chosen one marked. A chosen
 * option the content does not list is its own line. */
export function optionLines(at: ChoiceAt): OptionLine[] {
  const authored = at.card.item?.options ?? [];
  const chosen = at.choice?.option ?? null;
  const lines = authored.map((option, index): OptionLine => ({ key: String(option.ordinal), letter: optionLetter(index),
    text: option.name ?? OPTION_UNAVAILABLE, missing: option.name === null, chosen: chosen !== null && option.ordinal === chosen }));
  if (at.choice && (chosen === null || !lines.some(line => line.chosen)))
    lines.push({ key: 'chosen', letter: null, text: chosen === null ? 'Chosen option not recorded' : OPTION_UNAVAILABLE, missing: chosen !== null, chosen: true });
  return lines;
}

/** The card's date line: when it fired and was chosen, or how long it has been open. */
export function choiceDates(at: ChoiceAt): string {
  const fired = at.card.fired;
  if (at.choice) {
    if (!fired) return `Chosen ${dayOf(at.choice.row.hour)}`;
    const days = daysBetween(fired.row.hour, at.choice.row.hour);
    return `${dayOf(fired.row.hour)} → chosen ${dayOf(at.choice.row.hour)} · ${days ? daysText(days) : 'same day'}`;
  }
  const start = at.card.first.row.hour;
  if (at.pause) return `${dayOf(start)} · not chosen by ${dayOf(beforeBreak(at.pause, at.card.first.row.readHour))}`;
  return dayOf(start);
}

/** The open card's pill: "Awaiting choice · N days open" up to the playback date. */
export function awaitingText(at: ChoiceAt): string {
  const days = daysBetween(at.card.first.row.hour, at.hour);
  return `Awaiting choice · ${daysText(Math.max(0, days))} open`;
}

/** What the options say when none is chosen. */
export const noChoiceText = (at: ChoiceAt) => at.pause ? 'No choice recorded' : 'No option chosen yet';

export function choiceDescription(at: ChoiceAt, name: (tag: string, hour: number) => string): string {
  const country = at.card.country === null ? 'A country not identified' : name(at.card.country, at.card.first.row.nameHour);
  const chosen = optionLines(at).find(line => line.chosen);
  return `${country}: ${choiceTitle(at.card)}, ${choiceDates(at)}${at.choice ? `, ${chosen?.letter ? `${chosen.letter}. ` : ''}${chosen?.text}`
    : isOpen(at) ? `, ${awaitingText(at)}` : `, ${noChoiceText(at)}`}`;
}

/** The event feed's tooltip form: title, dates, the option taken, countries and Content
 * identity. Each country is named at the fact naming it: the chooser at its choice. */
export function choiceTooltip(at: ChoiceAt, name: (tag: string, hour: number) => string): TooltipContent[] {
  const { card, choice } = at;
  const fired = card.first.row.nameHour;
  const who = (tag: string | null, hour: number) => tag === null ? 'Not identified' : name(tag, hour);
  const rows: TooltipRow[] = [{ label: 'Country', value: who(card.country, fired) }];
  if (card.sender) rows.push({ label: 'From', value: name(card.sender, fired) });
  rows.push(card.fired ? { label: 'Fired', value: formatDateTime(card.fired.row.hour) } : { label: 'Fired', value: 'Not recorded', tone: 'muted' });
  if (choice) {
    rows.push({ label: 'Chosen', value: formatDateTime(choice.row.hour) });
    const chooser = choice.row.participants?.[0] ?? null;
    if (chooser !== card.country) rows.push({ label: 'Chosen by', value: who(chooser, choice.row.nameHour) });
    const line = optionLines(at).find(line => line.chosen);
    if (line) rows.push({ label: 'Option taken', value: `${line.letter ? `${line.letter}. ` : ''}${line.text}`, tone: line.missing || !line.letter ? 'muted' : undefined });
  } else if (at.pause) {
    rows.push({ label: 'Chosen', value: 'Not recorded', tone: 'muted' });
    rows.push({ label: 'Recording paused', value: `${formatDateTime(at.pause.firstHour)} to ${formatDateTime(at.pause.lastHour)}`, tone: 'muted' });
  } else rows.push({ label: 'Chosen', value: 'Not yet', tone: 'muted' });
  if (card.item?.token) rows.push({ label: 'Content identity', value: card.item.token });
  if (card.item?.description) rows.push({ label: card.item.description });
  return [{ title: choiceTitle(card), icon: card.item?.icon ?? undefined, rows }];
}

/** The menu's dates: when it fired and when it was chosen. */
export const choiceJumps = (at: ChoiceAt): number[] => [...new Set([at.card.fired?.row.hour, at.choice?.row.hour]
  .filter((hour): hour is number => hour !== undefined))];

/** Titles and option texts the content could not resolve, named by the event's
 * Content identity and counted once per card. */
export function choiceDiagnostics(replies: readonly ScriptOccurrenceReply[]): string[] {
  const titles = new Map<string, number>(), options = new Map<string, number>();
  const count = (map: Map<string, number>, key: string) => map.set(key, (map.get(key) ?? 0) + 1);
  for (const reply of replies) for (const occurrence of reply.occurrences) {
    if (!cardEdge(occurrence)) continue;
    const item = occurrenceItem(reply, occurrence);
    const name = item?.token ?? 'an event not identified';
    if (!item?.name) count(titles, name);
    const chosen = lastEdge(occurrence.edges.filter(isChoice), ['eventOptionSelected'])?.option ?? null;
    if (!item?.options || item.options.some(option => option.name === null)
      || (chosen !== null && !item.options.some(option => option.ordinal === chosen))) count(options, name);
  }
  const line = (label: string, map: Map<string, number>) => map.size ? [`${label}: ${[...map.keys()].sort().join(', ')} (${[...map.values()].reduce((a, b) => a + b, 0).toLocaleString()} counted)`] : [];
  return [...line('Event title unavailable', titles), ...line('Event option text unavailable', options)];
}
