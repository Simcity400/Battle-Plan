import type { LogRow } from '../../charts/log/logData';
import type { LogReplyRow } from '../../charts/log/logSource';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { beforeBreak, breakAfter, dayOf, daysBetween, daysText, edgesAt, occurrenceItem, occurrenceMatches, occurrenceOf } from '../event-feed/scriptOccurrences.ts';
import type { RecordingBreak } from '../event-feed/scriptOccurrences';
import { plainScripted, reasonRows } from '../event-feed/scriptedGaps.ts';
import type { EventFeedItem, ScriptEdge, ScriptOccurrence } from '../../generated/dashboard';

export const TITLE_UNAVAILABLE = 'Title not recorded';
export const OPTION_UNAVAILABLE = 'Option text not recorded';

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

/** A card as the reads by the hour shown show it. `pause` is a break in the recording after the firing, while no choice
 * is recorded: a choice then is unknown. */
export interface ChoiceAt {
  card: ChoiceCard;
  hour: number;
  choice: ScriptEdge | null;
  pause: RecordingBreak | null;
}

const isChoice = (edge: ScriptEdge) => edge.row.kind === 'eventOptionSelected' && (edge.option === null || edge.option >= 0);

/** The edge a card is dated by: its firing where recorded, else its first choice. Immediate effects with neither are no
 * card: they are part of a dispatch, never a choice. */
const cardEdge = (occurrence: ScriptOccurrence): ScriptEdge | null =>
  occurrence.opened ? occurrence.edges[0] : occurrence.edges.find(isChoice) ?? null;

/** A page row's card; null for a row that is no card. */
export function choiceCard(occurrence: ScriptOccurrence | null): ChoiceCard | null {
  if (!occurrence) return null;
  const first = cardEdge(occurrence);
  if (!first) return null;
  const fired = occurrence.opened ? first : null;
  const roles = first.row.participants ?? [null, null];
  const country = fired ? roles[1] : roles[0];
  const sender = fired && roles[0] !== null && roles[0] !== country ? roles[0] : null;
  return { occurrence, item: occurrenceItem(occurrence), first, fired, country, sender };
}

/** A row of an `event-choices` page, dated by its card's firing (or first choice). */
export function choiceRow(reply: LogReplyRow): LogRow<ChoiceCard | null> {
  const card = choiceCard(occurrenceOf(reply));
  return { key: reply.key, id: card?.occurrence.id ?? reply.key, hour: card?.first.row.hour ?? reply.hour, revealedHour: card?.first.row.readHour ?? reply.hour,
    by: false, item: card };
}

/** A card at `hour`: shown from its first read, chosen from the read that shows the choice; matched to the chosen countries.
 * `awaiting` keeps only cards still awaiting a choice. */
export function choiceAt(row: LogRow<ChoiceCard | null>, hour: number, breaks: readonly RecordingBreak[], awaiting: boolean,
  country: ((tag: string, hour: number) => boolean) | null): LogRow<ChoiceAt> | null {
  const card = row.item;
  if (!card || (row.revealedHour ?? row.hour) > hour || (country && !occurrenceMatches(card.occurrence, country))) return null;
  const choice = edgesAt(card.occurrence.edges, hour).filter(isChoice).at(-1) ?? null;
  const pause = choice ? null : breakAfter(breaks, row.revealedHour ?? row.hour, hour);
  if (awaiting && (choice || pause)) return null;
  return { ...row, item: { card, hour, choice, pause } };
}

export const isOpen = (at: ChoiceAt) => !at.choice && !at.pause;
export const choiceTitle = (card: ChoiceCard) => card.item?.name ?? TITLE_UNAVAILABLE;
/** The title as plain words, every gap written out. */
export const choiceText = (card: ChoiceCard) => plainScripted(choiceTitle(card));

/** "A", "B", … "Z", "AA": an option's place in the game's order. */
export function optionLetter(index: number): string {
  let letters = '';
  for (let at = index + 1; at > 0; at = Math.floor((at - 1) / 26)) letters = String.fromCharCode(65 + (at - 1) % 26) + letters;
  return letters;
}

export interface OptionLine { key: string; letter: string | null; text: string; missing: boolean; chosen: boolean }

/** Every authored option in the game's order, the chosen one marked. A chosen option the content does not list is its own line. */
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

/** The open card's pill: "Awaiting choice · N days open" up to the date shown. */
export function awaitingText(at: ChoiceAt): string {
  return `Awaiting choice · ${daysText(Math.max(0, daysBetween(at.card.first.row.hour, at.hour)))} open`;
}

/** What the options say when none is chosen. */
export const noChoiceText = (at: ChoiceAt) => at.pause ? 'No choice recorded' : 'No option chosen yet';

export function choiceDescription(at: ChoiceAt, name: (tag: string, hour: number) => string): string {
  const country = at.card.country === null ? 'A country not identified' : name(at.card.country, at.card.first.row.nameHour);
  const chosen = optionLines(at).find(line => line.chosen);
  return `${country}: ${choiceText(at.card)}, ${choiceDates(at)}${at.choice ? `, ${chosen?.letter ? `${chosen.letter}. ` : ''}${plainScripted(chosen?.text ?? '')}`
    : isOpen(at) ? `, ${awaitingText(at)}` : `, ${noChoiceText(at)}`}`;
}

/** Title, dates, the option taken, countries and Content identity. Each country is named at the row naming it: the chooser at
 * its choice. */
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
    if (line) rows.push({ label: 'Option taken', value: `${line.letter ? `${line.letter}. ` : ''}${plainScripted(line.text)}`, tone: line.missing || !line.letter ? 'muted' : undefined });
  } else if (at.pause) {
    rows.push({ label: 'Chosen', value: 'Not recorded', tone: 'muted' });
    rows.push({ label: 'Recording paused', value: `${formatDateTime(at.pause.firstHour)} to ${formatDateTime(at.pause.lastHour)}`, tone: 'muted' });
  } else rows.push({ label: 'Chosen', value: 'Not yet', tone: 'muted' });
  if (card.item?.token) rows.push({ label: 'Content identity', value: card.item.token });
  rows.push(...reasonRows(card.item));
  if (card.item?.description) rows.push({ label: plainScripted(card.item.description) });
  return [{ title: choiceText(card), icon: card.item?.icon ?? undefined, rows }];
}

/** The menu's dates: when it fired and when it was chosen. */
export const choiceJumps = (at: ChoiceAt): number[] => [...new Set([at.card.fired?.row.hour, at.choice?.row.hour]
  .filter((hour): hour is number => hour !== undefined))];

/** Titles and option texts the content could not resolve, named by the event's Content identity and counted once per card. */
export function choiceDiagnostics(cards: readonly ChoiceCard[]): string[] {
  const titles = new Map<string, number>(), options = new Map<string, number>();
  const count = (map: Map<string, number>, key: string) => map.set(key, (map.get(key) ?? 0) + 1);
  for (const card of cards) {
    const name = card.item?.token ?? 'an event not identified';
    if (!card.item?.name) count(titles, name);
    const chosen = card.occurrence.edges.filter(isChoice).at(-1)?.option ?? null;
    if (!card.item?.options || card.item.options.some(option => option.name === null)
      || (chosen !== null && !card.item.options.some(option => option.ordinal === chosen))) count(options, name);
  }
  const line = (label: string, map: Map<string, number>) => map.size ? [`${label}: ${[...map.keys()].sort().join(', ')} (${[...map.values()].reduce((a, b) => a + b, 0).toLocaleString()} counted)`] : [];
  return [...line('Event title unavailable', titles), ...line('Event option text unavailable', options)];
}
