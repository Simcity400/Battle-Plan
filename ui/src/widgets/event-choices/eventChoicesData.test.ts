import assert from 'node:assert/strict';
import test from 'node:test';
import { engineHour } from '../../dashboard/calendar.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { buildRowSourceFromHead, validateGenericTailReply } from '../../families/table/rowSource.ts';
import { awaitingText, choiceAt, choiceCard, choiceDates, choiceText, choiceTitle, isOpen, noChoiceText, optionLetter, optionLines } from './eventChoicesData.ts';
import { eventChoicesProjection } from './eventChoicesProjection.ts';
import type { EventDetailSubject } from './EventDetailWindow.tsx';
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

test('a card is dated by its firing, opens at its read, and shows its choice only from the read that shows it', () => {
  const fired = day(3, 3), chosen = day(3, 5);
  const card = choiceCard(occurrence([fire(fired), choose(chosen, 1)]));
  assert.deepEqual([card?.first.row.hour, card?.country, card?.sender], [fired, 'POL', 'GER']);
  assert.equal(choiceAt(card, fired - 1, []), null, 'not read yet');
  const open = choiceAt(card, fired + 30, [])!;
  assert.deepEqual([isOpen(open), awaitingText(open), noChoiceText(open)], [true, 'Awaiting choice · 1 day open', 'No option chosen yet']);
  const done = choiceAt(card, chosen, [])!;
  assert.deepEqual(optionLines(done).map(line => [line.letter, line.text, line.chosen]), [['A', 'Stand firm', false], ['B', 'Back down', true]]);
  assert.match(choiceDates(done), /→ chosen .* · 2 days/);
});

test('a break after the firing with no choice recorded is a pause, never a choice', () => {
  const fired = day(3, 3);
  const card = choiceCard(occurrence([fire(fired)]));
  const paused = choiceAt(card, day(4, 1), [{ firstHour: fired + 10, lastHour: fired + 20 }])!;
  assert.deepEqual([isOpen(paused), noChoiceText(paused)], [false, 'No choice recorded']);
  assert.match(choiceDates(paused), /not chosen by/);
});

test('a choice whose firing was not recorded is its own card, and immediate effects alone are none', () => {
  const alone = choiceCard(occurrence([choose(day(5, 1), 0, 'ITA')], false));
  assert.deepEqual([alone?.fired, alone?.country], [null, 'ITA']);
  assert.equal(choiceCard(occurrence([choose(day(5, 1), -1)], false)), null);
  assert.equal(choiceAt(choiceCard(occurrence([choose(day(5, 1), -1)], false)), day(6, 1), []), null);
});

test('titles and options the content could not resolve fall back cleanly; a chosen option not listed is its own line', () => {
  const unnamed = choiceCard(occurrence([fire(day(3, 3)), choose(day(3, 4), 5)]));
  const card = unnamed!;
  const missing = { ...card, item: { ...authored(), name: null } };
  assert.equal(choiceTitle(missing), 'Title not recorded');
  const lines = optionLines(choiceAt(unnamed, day(3, 4), [])!);
  assert.deepEqual(lines.at(-1), { key: 'chosen', letter: null, text: 'Option text not recorded', missing: true, chosen: true });
  assert.deepEqual([optionLetter(0), optionLetter(25), optionLetter(26)], ['A', 'Z', 'AA']);
});

test('an event the recording cannot title reads "Title not recorded"; a gap in its text becomes [not recorded]', () => {
  const why = 'event fra.1 has no title in its content';
  const untitled = edge('scriptEventFired', day(1, 6), ['GER', 'FRA'], { item: { ...authored(), name: null, nameMissing: why } as EventFeedItem });
  const at = choiceAt(choiceCard(occurrence([untitled])), day(1, 7), [])!;
  assert.equal(choiceTitle(at.card), 'Title not recorded');
  assert.deepEqual(optionLines(at).map(line => line.text), ['Stand firm', 'Back down'], 'its authored options still show');
  const gapped = edge('scriptEventFired', day(1, 6), ['GER', 'FRA'], { item: { ...authored(), name: 'Recruit in \uFFFC', gaps: 'it names a highlighted state the recording does not hold' } as EventFeedItem });
  const shown = choiceAt(choiceCard(occurrence([gapped])), day(1, 7), [])!;
  assert.equal(choiceText(shown.card), 'Recruit in [not recorded]');
});

test('event choices projection turns blank authored title or option text into notRecorded and tracks awaiting day boundaries', () => {
  const fired = day(3, 3);
  const chosen = day(3, 4);
  const blankItem: EventFeedItem = { token: 'blank.1', name: '   ', icon: null, description: null, options: [{ ordinal: 0, name: '   ' }] };
  const blankOcc = occurrence([
    edge('scriptEventFired', fired, ['GER', 'POL'], { item: blankItem }),
    edge('eventOptionSelected', chosen, ['POL', null], { item: blankItem }, { option: 0 }),
  ]);
  const openOcc = occurrence([fire(fired, 'FRA', 'GER')]);
  const head = {
    branchId: '1',
    log: 'event-choices',
    columns: [],
    baseOrdinal: '0',
    firstOrdinal: '0',
    offset: '0',
    total: '2',
    rows: [
      { hour: fired, key: blankOcc.id, ordinal: '0', cells: [], occurrence: blankOcc },
      { hour: fired, key: openOcc.id, ordinal: '1', cells: [], occurrence: openOcc },
    ],
    facetTotals: [],
    missing: [],
    unavailable: [],
    warnings: [],
  };
  const atHour = chosen + 6;
  const source = buildRowSourceFromHead('test:choices', head, undefined, atHour)!;
  const readView = createBoundedReadView({ hour: atHour, rows: new Map([['main', source]]) });
  const model = eventChoicesProjection.project(readView, {
    events: ['country'],
    awaiting: false,
    artwork: true,
    breaks: [],
  });
  assert.equal(model.titleSummary, '2 events');
  assert.equal(model.nextChangeAt, (Math.floor(atHour / 24) + 1) * 24, 'nextChangeAt is the next calendar day, where the days-open count changes');
  const blankRow = model.rows.find(r => r.key === blankOcc.id)!;
  assert.equal(blankRow.cells.item?.value.kind, 'notRecorded');
  assert.equal(blankRow.cells.choice?.value.kind, 'notRecorded');
  assert.equal(blankRow.cells.choice?.inlinePrefix, 'A');
  const openMeta = model.rows.find(r => r.key === openOcc.id)!.meta as EventDetailSubject;
  assert.equal(openMeta.firedHour, fired);
  assert.equal(openMeta.asOfHour, atHour);

  const productSource = buildRowSourceFromHead('test:choices-status', {
    ...head,
    total: '2',
    counts: [{ key: 'awaiting', total: '1' }],
    rows: [
      { hour: fired, key: openOcc.id, ordinal: '0', live: true, status: 'awaiting', occurrence: openOcc },
      { hour: fired, key: blankOcc.id, ordinal: '1', live: false, status: 'notRecorded', occurrence: occurrence([fire(fired, 'ITA', 'GER')]) },
    ],
  }, undefined, atHour)!;
  const awaitingModel = eventChoicesProjection.project(
    createBoundedReadView({ hour: atHour, rows: new Map([['main', productSource]]) }),
    { events: ['country'], awaiting: true, artwork: false, breaks: [] },
  );
  assert.equal(awaitingModel.titleSummary, '1 awaiting');
  assert.equal(awaitingModel.rows[0]?.live, true);
  assert.equal(awaitingModel.rows[1]?.live, false);
  assert.equal(awaitingModel.rows[1]?.cells.choice?.value.kind, 'notRecorded');
  assert.equal((awaitingModel.rows[1]?.meta as EventDetailSubject).awaiting, false);
});

test('an awaiting choice on a page frozen at an earlier hour counts its days open at that hour, and does not wake playback', () => {
  const fired = day(3, 3);
  const card = occurrence([fire(fired)]);
  const reply = (at: number) => ({ branchId: '1', total: '1', offset: '0', live: '1', counts: [{ key: 'awaiting', total: '1' }], nextChange: null,
    rows: [{ hour: fired, key: card.id, observed: fired, ordinal: '0', live: true, status: 'awaiting' as const, occurrence: card }], missing: [], unavailable: [], warnings: [], at });
  const frozenAt = fired + 30, shownAt = fired + 100;
  const pages = { 0: { hour: frozenAt, offset: 0, limit: 256, raw: reply(frozenAt), reply: validateGenericTailReply(reply(frozenAt), frozenAt) } };
  const source = buildRowSourceFromHead('test:choices-frozen', reply(shownAt), undefined, shownAt, pages)!;
  const model = eventChoicesProjection.project(createBoundedReadView({ hour: shownAt, rows: new Map([['main', source]]) }), { events: ['country'], awaiting: false, artwork: false, breaks: [] });
  const meta = model.rows[0]!.meta as EventDetailSubject;
  assert.equal(meta.asOfHour, frozenAt, 'the row is read as of the frozen hour');
  const row = model.rows[0]!;
  const extra = typeof row.tooltipExtra === 'function' ? row.tooltipExtra() : row.tooltipExtra;
  assert.ok(JSON.stringify(extra).includes('1 day open'), `one day open at the frozen hour, not four: ${JSON.stringify(extra)}`);
  assert.equal(model.nextChangeAt ?? Infinity, Infinity, 'a frozen row does not wake playback to advance');
});
