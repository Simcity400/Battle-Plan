import assert from 'node:assert/strict';
import test from 'node:test';
import { engineHour } from '../../dashboard/calendar.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import type { EventFeedItem, EventFeedKind, EventFeedRow, ScriptEdge, ScriptOccurrence } from '../../generated/dashboard';
import { recordingBreaks, validateScriptPage } from '../event-feed/scriptOccurrences.ts';
import { AWAITING, EVENT_KINDS, OPTIONS, definition } from './definition.ts';
import { awaitingText, choiceAt, choiceDates, choiceDescription, choiceDiagnostics, choiceJumps, choiceTitle, choiceTooltip, isOpen,
  noChoiceText, optionLetter, optionLines, prepareChoices } from './eventChoicesData.ts';
import type { ChoiceFilter } from './eventChoicesData';

const day = (month: number, date: number, year = 1938) => engineHour({ year, month, day: date, hour: 1 })!;
const START = day(1, 1), END = day(12, 31);
let serial = 0;
const edge = (kind: EventFeedKind, hour: number, participants: [string | null, string | null], more: Partial<EventFeedRow> = {},
  extra: Partial<ScriptEdge> = {}): ScriptEdge => ({
  row: { id: `native:1:${serial++}`, hour, readHour: hour, nameHour: hour, exact: true, category: 'politics', kind,
    tags: participants.filter((tag): tag is string => tag !== null), participants, item: 0, amount: null, ...more },
  option: null, lifecycle: null, days: null, costs: null, ...extra });
const fire = (hour: number, receiver = 'mod | receiver', sender: string | null = 'mod | sender', kind: EventFeedKind = 'scriptEventFired', more: Partial<EventFeedRow> = {}) =>
  edge(kind, hour, [sender, receiver], more);
const choose = (hour: number, option: number | null, chooser = 'mod | receiver', more: Partial<EventFeedRow> = {}) =>
  edge('eventOptionSelected', hour, [chooser, null], more, { option });
const occurrence = (edges: ScriptEdge[], opened = true): ScriptOccurrence => ({ id: edges[0].row.id, opened, edges });
const authored: EventFeedItem = { token: 'mod_events.1', name: 'A modded crisis', icon: null, description: 'What the modder wrote.', idea: null,
  options: [{ ordinal: 0, name: 'Stand firm' }, { ordinal: 1, name: 'Back down' }] };
const page = (occurrences: ScriptOccurrence[], items: EventFeedItem[] = [authored]) => validateScriptPage({
  reply: { branchId: '1', firstHour: START, lastHour: END, occurrences, items, unavailable: [], warnings: [] }, nextCursor: null });
const all: ChoiceFilter = { kinds: new Set(EVENT_KINDS.map(kind => kind.kind)), country: () => true, allCountries: true, firstHour: null };
const name = (tag: string) => tag.replace('mod | ', '');
const shown = (rows: ReturnType<typeof prepareChoices>, hour: number, breaks = [] as ReturnType<typeof recordingBreaks>, awaiting = false) =>
  rows.flatMap(row => { const at = choiceAt(row, hour, breaks, awaiting); return at ? [at] : []; });

test('a firing and its choice are one card with every authored option in order and the chosen one marked', () => {
  const rows = prepareChoices(page([occurrence([fire(day(3, 3)), choose(day(3, 5), 1)])]).reply, all);
  assert.equal(rows.length, 1);
  const [card] = shown(rows, END);
  assert.equal(choiceTitle(card.item.card), 'A modded crisis');
  assert.equal(card.item.card.country, 'mod | receiver');
  assert.equal(card.item.card.sender, 'mod | sender');
  assert.equal(choiceDates(card.item), '3 Mar 1938 → chosen 5 Mar 1938 · 2 days');
  assert.deepEqual(optionLines(card.item).map(line => [line.letter, line.text, line.chosen]), [['A', 'Stand firm', false], ['B', 'Back down', true]]);
  assert.deepEqual(choiceJumps(card.item), [day(3, 3), day(3, 5)]);
  const tooltip = choiceTooltip(card.item, name)[0];
  assert.equal(tooltip.title, 'A modded crisis');
  assert.deepEqual(tooltip.rows.find(row => row.label === 'Option taken')?.value, 'B. Back down');
  assert.equal(tooltip.rows.find(row => row.label === 'Content identity')?.value, 'mod_events.1');
  assert.equal(tooltip.rows.some(row => row.label === 'Chosen by'), false);
  assert.match(choiceDescription(card.item, name), /^receiver: A modded crisis, .*B\. Back down$/);
  const same = prepareChoices(page([occurrence([fire(day(3, 3)), choose(day(3, 3) + 5, 0)])]).reply, all);
  assert.equal(choiceDates(shown(same, END)[0].item), '3 Mar 1938 → chosen 3 Mar 1938 · same day');
});

test('repeated firings of one event and deliveries to several countries stay separate cards', () => {
  const rows = prepareChoices(page([occurrence([fire(day(4, 1))]), occurrence([fire(day(2, 1)), choose(day(2, 2), 0)]),
    occurrence([fire(day(4, 1), 'mod | other', null, 'newsEventFired')])]).reply, all);
  assert.equal(rows.length, 3);
  assert.equal(new Set(rows.map(row => row.key)).size, 3);
  assert.deepEqual(rows.map(row => row.hour), [day(4, 1), day(4, 1), day(2, 1)]);
  assert.equal(shown(rows, END).filter(at => isOpen(at.item)).length, 2);
});

test('at the playback date a card is open or chosen as its reads show, and seeking back removes later cards', () => {
  const rows = prepareChoices(page([occurrence([fire(day(3, 1), undefined, undefined, undefined, { readHour: day(3, 2) }),
    choose(day(3, 10), 0, undefined, { readHour: day(3, 11) })])]).reply, all);
  assert.deepEqual(shown(rows, day(3, 1)), []);
  const open = shown(rows, day(3, 9))[0].item;
  assert.equal(isOpen(open), true);
  assert.equal(awaitingText(open), 'Awaiting choice · 8 days open');
  assert.equal(noChoiceText(open), 'No option chosen yet');
  assert.equal(choiceDates(open), '1 Mar 1938');
  // The choice's own date is known only once its read arrives.
  assert.equal(isOpen(shown(rows, day(3, 10) + 5)[0].item), true);
  const chosen = shown(rows, day(3, 11))[0].item;
  assert.equal(chosen.choice?.option, 0);
  assert.deepEqual(shown(rows, day(3, 11), [], true), []);
  assert.equal(shown(rows, day(3, 9), [], true).length, 1);
  // An event's immediate option is no choice.
  const immediate = prepareChoices(page([occurrence([fire(day(5, 1)), choose(day(5, 1), -1)])]).reply, all);
  assert.equal(isOpen(shown(immediate, END)[0].item), true);
  // An immediate effect whose dispatch was not recorded is no card, never one awaiting a choice.
  const orphan = page([occurrence([choose(day(5, 2), -1)], false)]).reply;
  assert.deepEqual(prepareChoices(orphan, all), []);
  assert.deepEqual(choiceDiagnostics([orphan]), []);
  // With a choice after it, the card is the choice's, dated by it.
  const chosenAfter = prepareChoices(page([occurrence([choose(day(5, 2), -1), choose(day(5, 4), 1)], false)]).reply, all);
  assert.deepEqual(chosenAfter.map(row => row.hour), [day(5, 4)]);
  assert.equal(choiceDates(shown(chosenAfter, END)[0].item), 'Chosen 4 May 1938');
  assert.deepEqual(shown(chosenAfter, day(5, 3)), [], 'nothing before the choice is read');
});

test('an open card across a break in the recording is bounded by the last read before it, and a choice keeps its own date', () => {
  const breaks = recordingBreaks({ gaps: [{ firstHour: day(6, 10), lastHour: day(6, 20) }], eventLosses: [{ afterHour: day(8, 1), hour: day(8, 3) }] });
  assert.deepEqual(breaks, [{ firstHour: day(6, 10), lastHour: day(6, 20) }, { firstHour: day(8, 1) + 1, lastHour: day(8, 3) }]);
  const rows = prepareChoices(page([occurrence([fire(day(6, 1))]), occurrence([fire(day(5, 1)), choose(day(6, 25), 1)])]).reply, all);
  const [open, closed] = shown(rows, day(7, 1), breaks);
  assert.equal(isOpen(open.item), false);
  assert.equal(choiceDates(open.item), `1 Jun 1938 · not chosen by ${'9 Jun 1938'}`);
  assert.equal(noChoiceText(open.item), 'No choice recorded');
  assert.equal(choiceTooltip(open.item, name)[0].rows.find(row => row.label === 'Chosen')?.value, 'Not recorded');
  assert.equal(choiceDates(closed.item), '1 May 1938 → chosen 25 Jun 1938 · 55 days');
  assert.equal(shown(rows, day(7, 1), breaks, true).length, 0);
  // Before the break the card is still awaiting its choice.
  assert.equal(isOpen(shown(rows, day(6, 5), breaks)[0].item), true);
});

test('kind toggles, range and country narrowing keep received news, senders and choosers', () => {
  const data = page([occurrence([fire(day(3, 1), 'mod | receiver', 'mod | sender')]), occurrence([fire(day(4, 1), 'mod | other', null, 'newsEventFired')]),
    occurrence([choose(day(5, 1), 0, 'mod | chooser')], false)]).reply;
  const news: ChoiceFilter = { ...all, kinds: new Set<EventFeedKind>(['newsEventFired']) };
  assert.deepEqual(prepareChoices(data, news).map(row => row.item.card.country), ['mod | chooser', 'mod | other']);
  const country = (tag: string): ChoiceFilter => ({ ...all, allCountries: false, country: candidate => candidate === tag });
  assert.deepEqual(prepareChoices(data, country('mod | other')).map(row => row.item.card.country), ['mod | other']);
  assert.deepEqual(prepareChoices(data, country('mod | sender')).map(row => row.item.card.country), ['mod | receiver']);
  const unopened = prepareChoices(data, country('mod | chooser'));
  assert.equal(unopened[0].item.card.fired, null);
  assert.equal(choiceDates(shown(unopened, END)[0].item), 'Chosen 1 May 1938');
  assert.deepEqual(prepareChoices(data, { ...all, firstHour: day(3, 15) }).map(row => row.hour), [day(5, 1), day(4, 1)]);
  const options = resolveOptions(OPTIONS, {});
  assert.deepEqual([options.countryEvents, options.newsEvents, options[AWAITING.value], options.artwork], [true, true, false, true]);
  assert.deepEqual([definition.defaultSize, definition.minSize, definition.category, definition.icon], [{ w: 12, h: 12 }, { w: 6, h: 8 }, 'Politics', 'ballot']);
});

test('missing titles and option texts read as unavailable and are named and counted in diagnostics', () => {
  const missing: EventFeedItem = { token: 'mod_events.2', name: null, icon: null, description: null, idea: null, options: [{ ordinal: 0, name: null }, { ordinal: 3, name: 'Known' }] };
  const unresolved: EventFeedItem = { token: 'mod_events.3', name: 'No options resolved', icon: null, description: null, idea: null };
  const data = page([occurrence([fire(day(3, 1)), choose(day(3, 2), 0)]), occurrence([fire(day(3, 3), undefined, undefined, undefined, { item: 1 }), choose(day(3, 4), 7, undefined, { item: 1 })])],
    [missing, unresolved]).reply;
  const [second, first] = shown(prepareChoices(data, all), END);
  assert.equal(choiceTitle(first.item.card), 'Title unavailable');
  assert.deepEqual(optionLines(first.item).map(line => [line.letter, line.text, line.missing, line.chosen]),
    [['A', 'Option text unavailable', true, true], ['B', 'Known', false, false]]);
  assert.deepEqual(optionLines(second.item).map(line => [line.letter, line.text, line.chosen]), [[null, 'Option text unavailable', true]]);
  assert.deepEqual(choiceDiagnostics([data]), ['Event title unavailable: mod_events.2 (1 counted)',
    'Event option text unavailable: mod_events.2, mod_events.3 (2 counted)']);
  assert.deepEqual([optionLetter(0), optionLetter(25), optionLetter(26)], ['A', 'Z', 'AA']);
});

test('malformed occurrence pages are refused', () => {
  const good = occurrence([fire(day(3, 1))]);
  assert.throws(() => page([{ ...good, edges: [] }]), /malformed/);
  assert.throws(() => page([{ ...good, edges: [{ ...good.edges[0], costs: [{ currency: 'fuel' as never, amount: '1' }] }] }]), /malformed/);
  assert.throws(() => page([{ ...good, edges: [{ ...good.edges[0], row: { ...good.edges[0].row, item: 4 } }] }]), /malformed/);
  assert.throws(() => validateScriptPage({ reply: { branchId: '1', firstHour: START, lastHour: END, occurrences: [], items: [], unavailable: [], warnings: [] }, nextCursor: '' }), /malformed/);
  const prefixed = validateScriptPage({ reply: { branchId: '1', firstHour: START, lastHour: END, occurrences: [], items: [], unavailable: [], warnings: [] },
    nextCursor: null, pending: [{ firstHour: START + 10, lastHour: END }], coverage: { firstHour: START, lastHour: START + 9, complete: false } });
  assert.deepEqual(prefixed.prefix?.pending, [{ firstHour: START + 10, lastHour: END }]);
});

test('the chooser is named at its choice, the firing countries at the firing', () => {
  const rows = prepareChoices(page([occurrence([fire(day(3, 3)), choose(day(3, 9), 1, 'mod | renamed')])]).reply, all);
  const [card] = shown(rows, END);
  const asked: [string, number][] = [];
  const dated = (tag: string, hour: number) => { asked.push([tag, hour]); return `${name(tag)} at ${hour}`; };
  const lines = choiceTooltip(card.item, dated)[0].rows;
  assert.equal(lines.find(row => row.label === 'Country')?.value, `receiver at ${day(3, 3)}`);
  assert.equal(lines.find(row => row.label === 'From')?.value, `sender at ${day(3, 3)}`);
  assert.equal(lines.find(row => row.label === 'Chosen by')?.value, `renamed at ${day(3, 9)}`);
  assert.deepEqual(asked, [['mod | receiver', day(3, 3)], ['mod | sender', day(3, 3)], ['mod | renamed', day(3, 9)]]);
});
