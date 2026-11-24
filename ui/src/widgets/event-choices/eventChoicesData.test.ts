import assert from 'node:assert/strict';
import test from 'node:test';
import { engineHour } from '../../dashboard/calendar.ts';
import { awaitingText, choiceAt, choiceCard, choiceDates, choiceText, choiceTitle, isOpen, noChoiceText, optionLetter, optionLines } from './eventChoicesData.ts';
import type { EventFeedItem, EventFeedKind, EventFeedRow, ScriptEdge, ScriptOccurrence } from '../../generated/dashboard';

const day = (month: number, date: number, year = 1938) => engineHour({ year, month, day: date, hour: 1 })!;
let serial = 0;
const authored = (): EventFeedItem => ({ token: 'mod_events.1', name: 'A modded crisis', icon: null, description: 'What the modder wrote.',
  options: [{ ordinal: 0, name: 'Stand firm' }, { ordinal: 1, name: 'Back down' }] });
const edge = (kind: EventFeedKind, hour: number, participants: [string | null, string | null], more: Partial<EventFeedRow> = {}, extra: Partial<ScriptEdge> = {}): ScriptEdge => ({
  row: { id: `native:1:${serial++}`, hour, readHour: hour, nameHour: hour, exact: true, category: 'politics', kind,
    tags: participants.filter((tag): tag is string => tag !== null), participants, item: authored(), amount: null, ...more },
  option: null, lifecycle: null, days: null, costs: null, ...extra });
const fire = (hour: number, receiver = 'POL', sender: string | null = 'GER') => edge('scriptEventFired', hour, [sender, receiver]);
const choose = (hour: number, option: number | null, chooser = 'POL') => edge('eventOptionSelected', hour, [chooser, null], {}, { option });
const occurrence = (edges: ScriptEdge[], opened = true): ScriptOccurrence => ({ id: edges[0].row.id, opened, edges });
const row = (held: ScriptOccurrence) => {
  const card = choiceCard(held);
  return { key: '1', id: card?.occurrence.id ?? '1', hour: card?.first.row.hour ?? held.edges[0].row.readHour, revealedHour: card?.first.row.readHour ?? held.edges[0].row.readHour, by: false, item: card };
};

test('a card is dated by its firing, opens at its read, and shows its choice only from the read that shows it', () => {
  const fired = day(3, 3), chosen = day(3, 5);
  const card = row(occurrence([fire(fired), choose(chosen, 1)]));
  assert.deepEqual([card.hour, card.item?.country, card.item?.sender], [fired, 'POL', 'GER']);
  assert.equal(choiceAt(card, fired - 1, [], false, null), null, 'not read yet');
  const open = choiceAt(card, fired + 30, [], false, null)!.item;
  assert.deepEqual([isOpen(open), awaitingText(open), noChoiceText(open)], [true, 'Awaiting choice · 1 day open', 'No option chosen yet']);
  const done = choiceAt(card, chosen, [], false, null)!.item;
  assert.deepEqual(optionLines(done).map(line => [line.letter, line.text, line.chosen]), [['A', 'Stand firm', false], ['B', 'Back down', true]]);
  assert.match(choiceDates(done), /→ chosen .* · 2 days/);
  assert.equal(choiceAt(card, chosen, [], true, null), null, 'awaiting only hides a chosen card');
  assert.equal(choiceAt(card, chosen, [], false, tag => tag === 'ITA'), null, 'a card naming no chosen country');
});

test('a break after the firing with no choice recorded is a pause, never a choice', () => {
  const fired = day(3, 3);
  const card = row(occurrence([fire(fired)]));
  const paused = choiceAt(card, day(4, 1), [{ firstHour: fired + 10, lastHour: fired + 20 }], false, null)!.item;
  assert.deepEqual([isOpen(paused), noChoiceText(paused)], [false, 'No choice recorded']);
  assert.match(choiceDates(paused), /not chosen by/);
  assert.equal(choiceAt(card, day(4, 1), [{ firstHour: fired + 10, lastHour: fired + 20 }], true, null), null);
});

test('a choice whose firing was not recorded is its own card, and immediate effects alone are none', () => {
  const alone = row(occurrence([choose(day(5, 1), 0, 'ITA')], false));
  assert.deepEqual([alone.item?.fired, alone.item?.country], [null, 'ITA']);
  assert.equal(row(occurrence([choose(day(5, 1), -1)], false)).item, null);
  assert.equal(choiceAt(row(occurrence([choose(day(5, 1), -1)], false)), day(6, 1), [], false, null), null);
});

test('titles and options the content could not resolve fall back cleanly; a chosen option not listed is its own line', () => {
  const unnamed = row(occurrence([fire(day(3, 3)), choose(day(3, 4), 5)]));
  const card = unnamed.item!;
  const missing = { ...card, item: { ...authored(), name: null } };
  assert.equal(choiceTitle(missing), 'Title not recorded');
  const lines = optionLines(choiceAt(unnamed, day(3, 4), [], false, null)!.item);
  assert.deepEqual(lines.at(-1), { key: 'chosen', letter: null, text: 'Option text not recorded', missing: true, chosen: true });
  assert.deepEqual([optionLetter(0), optionLetter(25), optionLetter(26)], ['A', 'Z', 'AA']);
});

test('an event the recording cannot title reads "Title not recorded"; a gap in its text becomes [not recorded]', () => {
  const why = 'event fra.1 has no title in its content';
  const untitled = edge('scriptEventFired', day(1, 6), ['GER', 'FRA'], { item: { ...authored(), name: null, nameMissing: why } as EventFeedItem });
  const at = choiceAt(row(occurrence([untitled])), day(1, 7), [], false, null)!.item;
  assert.equal(choiceTitle(at.card), 'Title not recorded');
  assert.deepEqual(optionLines(at).map(line => line.text), ['Stand firm', 'Back down'], 'its authored options still show');
  const gapped = edge('scriptEventFired', day(1, 6), ['GER', 'FRA'], { item: { ...authored(), name: 'Recruit in \uFFFC', gaps: 'it names a highlighted state the recording does not hold' } as EventFeedItem });
  const shown = choiceAt(row(occurrence([gapped])), day(1, 7), [], false, null)!.item;
  assert.equal(choiceText(shown.card), 'Recruit in [not recorded]');
});
