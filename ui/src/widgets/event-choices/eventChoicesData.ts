import { breakAfter, dayOf, daysBetween, daysText, edgesAt, occurrenceItem } from '../event-feed/scriptOccurrences.ts';
import type { RecordingBreak } from '../event-feed/scriptOccurrences';
import { gapsForField, plainScripted } from '../event-feed/scriptedGaps.ts';
import type { EventFeedItem, ScriptEdge, ScriptOccurrence } from '../../generated/dashboard';

const TITLE_UNAVAILABLE = 'Title not recorded';
const OPTION_UNAVAILABLE = 'Option text not recorded';
export const FIRING_NOT_RECORDED = 'The event firing was not recorded; only its selected option was recorded';

export interface ChoiceCard {
  occurrence: ScriptOccurrence;
  item: EventFeedItem | null;
  first: ScriptEdge;
  fired: ScriptEdge | null;
  country: string | null;
  sender: string | null;
}

export interface ChoiceAt {
  card: ChoiceCard;
  hour: number;
  choice: ScriptEdge | null;
  pause: RecordingBreak | null;
}

const isChoice = (edge: ScriptEdge) => edge.row.kind === 'eventOptionSelected' && (edge.option == null || edge.option >= 0);

const cardEdge = (occurrence: ScriptOccurrence): ScriptEdge | null =>
  occurrence.opened ? occurrence.edges[0] : occurrence.edges.find(isChoice) ?? null;

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

export function choiceAt(card: ChoiceCard | null, hour: number, breaks: readonly RecordingBreak[]): ChoiceAt | null {
  if (!card || card.first.row.readHour > hour) return null;
  const choice = edgesAt(card.occurrence.edges, hour).filter(isChoice).at(-1) ?? null;
  const pause = choice ? null : breakAfter(breaks, card.first.row.readHour, hour);
  return { card, hour, choice, pause };
}

export const isOpen = (at: ChoiceAt) => !at.choice && !at.pause;
export const choiceTitle = (card: ChoiceCard) => card.item?.name ?? TITLE_UNAVAILABLE;
export const choiceText = (card: ChoiceCard) => plainScripted(choiceTitle(card), gapsForField(card.item?.gaps, 'title'));

export function optionLetter(index: number): string {
  let letters = '';
  for (let at = index + 1; at > 0; at = Math.floor((at - 1) / 26)) letters = String.fromCharCode(65 + (at - 1) % 26) + letters;
  return letters;
}

export interface OptionLine { key: string; ordinal: number | null; letter: string | null; text: string; missing: boolean; chosen: boolean }

export function optionLines(at: ChoiceAt): OptionLine[] {
  const authored = at.card.item?.options ?? [];
  const chosen = at.choice?.option ?? null;
  const lines = authored.map((option, index): OptionLine => ({ key: String(option.ordinal), ordinal: option.ordinal, letter: optionLetter(index),
    text: option.name ?? OPTION_UNAVAILABLE, missing: option.name === null, chosen: chosen !== null && option.ordinal === chosen }));
  if (at.choice && (chosen === null || !lines.some(line => line.chosen)))
    lines.push({ key: 'chosen', ordinal: null, letter: null, text: chosen === null ? 'Chosen option not recorded' : OPTION_UNAVAILABLE, missing: chosen !== null, chosen: true });
  return lines;
}

export function choiceDates(at: ChoiceAt, breakHour?: number | null): string {
  const fired = at.card.fired;
  if (at.choice) {
    if (!fired) return `Firing date not recorded: ${FIRING_NOT_RECORDED}. Chosen by ${dayOf(at.choice.row.readHour)}`;
    const days = daysBetween(fired.row.hour, at.choice.row.hour);
    return `${dayOf(fired.row.hour)} → chosen ${dayOf(at.choice.row.hour)} · ${days ? daysText(days) : 'same day'}`;
  }
  const start = at.card.first.row.hour;
  const lostAt = breakHour !== undefined ? breakHour : at.pause?.firstHour ?? null;
  if (lostAt !== null && lostAt > at.card.first.row.readHour) {
    return `${dayOf(start)} · not chosen by ${dayOf(lostAt - 1)}`;
  }
  return dayOf(start);
}

export function awaitingText(at: ChoiceAt): string {
  return `Awaiting choice · ${daysText(Math.max(0, daysBetween(at.card.first.row.hour, at.hour)))} open`;
}

const PAUSE_REASON = /^(?:recording\s+)?paused\.?$/i;

export function choiceNotRecordedText(reason: string | null | undefined): string {
  const trimmed = reason?.trim();
  if (!trimmed) return 'Choice not recorded (gap reason not recorded)';
  return `Choice not recorded (${PAUSE_REASON.test(trimmed) ? 'recording paused' : trimmed})`;
}

export const noChoiceText = (at: ChoiceAt) => at.pause ? choiceNotRecordedText(at.pause.reason) : 'No option chosen yet';
