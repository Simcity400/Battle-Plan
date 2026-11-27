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

test('a break after the firing with no choice recorded reports the gap reason, never inventing a pause', () => {
  const fired = day(3, 3);
  const card = choiceCard(occurrence([fire(fired)]));
  const overflow = choiceAt(card, day(4, 1), [{ firstHour: fired + 10, lastHour: fired + 20, reason: 'collector queue overflow' }])!;
  assert.deepEqual([isOpen(overflow), noChoiceText(overflow)], [false, 'Choice not recorded (collector queue overflow)']);
  assert.match(choiceDates(overflow), /not chosen by/);
  const unrecordedReason = choiceAt(card, day(4, 1), [{ firstHour: fired + 10, lastHour: fired + 20 }])!;
  assert.deepEqual([isOpen(unrecordedReason), noChoiceText(unrecordedReason)], [false, 'Choice not recorded (gap reason not recorded)']);
  const paused = choiceAt(card, day(4, 1), [{ firstHour: fired + 10, lastHour: fired + 20, reason: 'recording paused' }])!;
  assert.deepEqual([isOpen(paused), noChoiceText(paused)], [false, 'Choice not recorded (recording paused)']);
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
  assert.deepEqual(lines.at(-1), { key: 'chosen', ordinal: null, letter: null, text: 'Option text not recorded', missing: true, chosen: true });
  assert.deepEqual([optionLetter(0), optionLetter(25), optionLetter(26)], ['A', 'Z', 'AA']);
});

test('an event the recording cannot title reads "Title not recorded"; a gap in its text shows its reason in place', () => {
  const why = 'event fra.1 has no title in its content';
  const untitled = edge('scriptEventFired', day(1, 6), ['GER', 'FRA'], { item: { ...authored(), name: null, nameMissing: why } as EventFeedItem });
  const at = choiceAt(choiceCard(occurrence([untitled])), day(1, 7), [])!;
  assert.equal(choiceTitle(at.card), 'Title not recorded');
  assert.deepEqual(optionLines(at).map(line => line.text), ['Stand firm', 'Back down'], 'its authored options still show');
  const gapped = edge('scriptEventFired', day(1, 6), ['GER', 'FRA'], {
    item: {
      ...authored(),
      name: 'Build in \uFFFC',
      gaps: [{ field: 'title', key: 'FROM.GetName', reason: 'it names a highlighted state the recording does not hold' }],
    } as EventFeedItem,
  });
  const shown = choiceAt(choiceCard(occurrence([gapped])), day(1, 7), [])!;
  assert.equal(choiceText(shown.card), 'Build in [not recorded: FROM.GetName: it names a highlighted state the recording does not hold]');
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
  assert.deepEqual(
    awaitingModel.rows[1]?.cells.choice?.value,
    { kind: 'notRecorded', reason: 'Choice not recorded' },
    'a lost-read notRecorded choice without a recording pause does not claim recording paused',
  );
  assert.equal((awaitingModel.rows[1]?.meta as EventDetailSubject).awaiting, false);

  const pausedModel = eventChoicesProjection.project(
    createBoundedReadView({ hour: atHour, rows: new Map([['main', productSource]]) }),
    { events: ['country'], awaiting: true, artwork: false, breaks: [{ firstHour: fired + 4, lastHour: fired + 8, reason: 'Recording paused' }] },
  );
  assert.deepEqual(
    pausedModel.rows[1]?.cells.choice?.value,
    { kind: 'notRecorded', reason: 'Choice not recorded (recording paused)' },
    'a break marked as a pause uses the recording-paused wording',
  );
  assert.equal((pausedModel.rows[1]?.meta as EventDetailSubject).noChoiceReason, 'Choice not recorded (recording paused)');

  const overflowModel = eventChoicesProjection.project(
    createBoundedReadView({ hour: atHour, rows: new Map([['main', productSource]]) }),
    { events: ['country'], awaiting: true, artwork: false, breaks: [{ firstHour: fired + 4, lastHour: fired + 8, reason: 'collector queue overflow' }] },
  );
  assert.deepEqual(
    overflowModel.rows[1]?.cells.choice?.value,
    { kind: 'notRecorded', reason: 'Choice not recorded (collector queue overflow)' },
    'a non-pause gap reports its own recorded reason instead of claiming recording paused',
  );
  assert.equal((overflowModel.rows[1]?.meta as EventDetailSubject).noChoiceReason, 'Choice not recorded (collector queue overflow)');

  const blankReasonModel = eventChoicesProjection.project(
    createBoundedReadView({ hour: atHour, rows: new Map([['main', productSource]]) }),
    { events: ['country'], awaiting: true, artwork: false, breaks: [{ firstHour: fired + 4, lastHour: fired + 8 }] },
  );
  assert.deepEqual(
    blankReasonModel.rows[1]?.cells.choice?.value,
    { kind: 'notRecorded', reason: 'Choice not recorded (gap reason not recorded)' },
    'a break with no recorded reason says the gap reason is not recorded',
  );
  assert.equal((blankReasonModel.rows[1]?.meta as EventDetailSubject).noChoiceReason, 'Choice not recorded (gap reason not recorded)');

  const afterBreaks = fired + 120;
  const serverReasonSource = buildRowSourceFromHead('test:choices:server-reason', {
    ...head,
    total: '2',
    counts: [{ key: 'awaiting', total: '1' }],
    rows: [
      { hour: fired, key: openOcc.id, ordinal: '0', live: true, status: 'awaiting', occurrence: openOcc },
      { hour: fired, key: blankOcc.id, ordinal: '1', live: false, status: 'notRecorded', notRecordedReason: 'event feed read in part', notRecordedHour: fired + 48, occurrence: occurrence([fire(fired, 'ITA', 'GER')]) },
    ],
  }, undefined, afterBreaks)!;
  const serverReasonModel = eventChoicesProjection.project(
    createBoundedReadView({ hour: afterBreaks, rows: new Map([['main', serverReasonSource]]) }),
    { events: ['country'], awaiting: true, artwork: false, breaks: [{ firstHour: fired + 96, lastHour: fired + 120, reason: 'Recording paused' }] },
  );
  assert.deepEqual(
    serverReasonModel.rows[1]?.cells.choice?.value,
    { kind: 'notRecorded', reason: 'Choice not recorded (event feed read in part)' },
    'a later pause does not overwrite the earlier break reason reported by the server',
  );
  assert.equal((serverReasonModel.rows[1]?.meta as EventDetailSubject).noChoiceReason, 'Choice not recorded (event feed read in part)');
  const serverTiming = serverReasonModel.rows[1]?.tooltipExtra;
  const serverTimingRows = (typeof serverTiming === 'function' ? serverTiming() : serverTiming) ?? [];
  assert.deepEqual(
    serverTimingRows.find(row => row.label === 'Timing')?.value,
    { kind: 'known', value: '3 Mar 1938 · not chosen by 4 Mar 1938' },
    'timing tooltip uses the first-break hour before the later pause',
  );
  const futureBreak = eventChoicesProjection.validateTail!({
    ...head,
    total: '1',
    rows: [
      { hour: fired, key: blankOcc.id, ordinal: '0', live: false, status: 'notRecorded', notRecordedReason: 'event feed read in part', notRecordedHour: fired + 48, occurrence: occurrence([fire(fired, 'ITA', 'GER')]) },
    ],
  }, fired + 24);
  assert.match(futureBreak.rowRefusals?.get(0) ?? '', /after displayed hour/, 'a break hour past the displayed hour is rejected');

  const lossAtReadModel = eventChoicesProjection.project(
    createBoundedReadView({ hour: atHour, rows: new Map([['main', productSource]]) }),
    { events: ['country'], awaiting: true, artwork: false, breaks: [{ firstHour: fired - 2, lastHour: fired, reason: 'The recorder was detached.' }] },
  );
  assert.deepEqual(
    lossAtReadModel.rows[1]?.cells.choice?.value,
    { kind: 'notRecorded', reason: 'Choice not recorded (The recorder was detached.)' },
    'a firing read at the ending hour of an event-loss interval reports that detachment reason',
  );
  assert.equal((lossAtReadModel.rows[1]?.meta as EventDetailSubject).noChoiceReason, 'Choice not recorded (The recorder was detached.)');
  const lossTiming = lossAtReadModel.rows[1]?.tooltipExtra;
  const lossTimingRows = (typeof lossTiming === 'function' ? lossTiming() : lossTiming) ?? [];
  assert.deepEqual(
    lossTimingRows.find(row => row.label === 'Timing')?.value,
    { kind: 'known', value: '3 Mar 1938' },
    'a loss ending at the firing hour does not claim the choice stayed open through that hour',
  );
});

test('an awaiting choice on a page frozen at an earlier hour counts its days open at that hour, and does not wake playback', () => {
  const fired = day(3, 3);
  const card = occurrence([fire(fired)]);
  const reply = (at: number) => ({ branchId: '1', total: '1', offset: '0', live: '1', counts: [{ key: 'awaiting', total: '1' }], nextChange: null,
    rows: [{ hour: fired, key: card.id, observed: fired, ordinal: '0', live: true, status: 'awaiting' as const, occurrence: card }], missing: [], unavailable: [], warnings: [], at });
  const frozenAt = fired + 30, shownAt = fired + 100;
  const gappedReply = (at: number) => ({
    ...reply(at),
    missing: [
      { firstHour: fired + 10, lastHour: fired + 50, reason: 'Unread span crossing frozen hour' },
      { firstHour: fired + 60, lastHour: fired + 90, reason: 'Future unread span after frozen hour' },
    ],
  });
  const pages = { 0: { hour: frozenAt, offset: 0, limit: 256, raw: gappedReply(frozenAt), reply: validateGenericTailReply(gappedReply(frozenAt), frozenAt) } };
  const source = buildRowSourceFromHead('test:choices-frozen', gappedReply(shownAt), undefined, shownAt, pages)!;
  const model = eventChoicesProjection.project(createBoundedReadView({ hour: shownAt, rows: new Map([['main', source]]) }), { events: ['country'], awaiting: false, artwork: false, breaks: [] });
  const meta = model.rows[0]!.meta as EventDetailSubject;
  assert.equal(meta.asOfHour, frozenAt, 'the row is read as of the frozen hour');
  const row = model.rows[0]!;
  const extra = typeof row.tooltipExtra === 'function' ? row.tooltipExtra() : row.tooltipExtra;
  assert.ok(JSON.stringify(extra).includes('1 day open'), `one day open at the frozen hour, not four: ${JSON.stringify(extra)}`);
  assert.equal(model.nextChangeAt ?? Infinity, Infinity, 'a frozen row does not wake playback to advance');
  const gapSlots = (model.slots ?? []).filter(s => s.kind === 'gap');
  assert.equal(gapSlots.length, 1, 'future gaps after the frozen page hour are excluded');
  assert.equal(gapSlots[0]!.lastHour, frozenAt, 'gaps overlapping the frozen page hour clip to readAt');
  assert.equal(model.titleSummaryPartial, 'Count from recorded hours');

  const futureOnlyReply = (at: number) => ({
    ...reply(at),
    missing: [{ firstHour: fired + 60, lastHour: fired + 90, reason: 'Future unread span after frozen hour' }],
  });
  const futurePages = { 0: { hour: frozenAt, offset: 0, limit: 256, raw: futureOnlyReply(frozenAt), reply: validateGenericTailReply(futureOnlyReply(frozenAt), frozenAt) } };
  const futureSource = buildRowSourceFromHead('test:choices-frozen-future', futureOnlyReply(shownAt), undefined, shownAt, futurePages)!;
  const futureModel = eventChoicesProjection.project(createBoundedReadView({ hour: shownAt, rows: new Map([['main', futureSource]]) }), { events: ['country'], awaiting: false, artwork: false, breaks: [] });
  assert.equal((futureModel.slots ?? []).filter(s => s.kind === 'gap').length, 0);
  assert.equal(futureModel.titleSummaryPartial, undefined, 'a frozen page with only future gaps does not mark its count partial');
});

test('S3: a malformed event choice row becomes a refused slot with its reason instead of disappearing or reporting no matches', () => {
  const fired = day(3, 3);
  const atHour = fired + 12;
  const goodOcc = occurrence([fire(fired, 'POL', 'GER')]);
  const badOcc = { id: 'bad:1', opened: true, edges: [] };
  const mixedHead = {
    branchId: '1',
    total: '2',
    offset: '0',
    live: '1',
    counts: [{ key: 'awaiting', total: '2' }],
    rows: [
      { hour: fired, key: goodOcc.id, observed: fired, ordinal: '0', live: true, status: 'awaiting', occurrence: goodOcc },
      { hour: fired, key: 'bad:1', observed: fired, ordinal: '1', live: false, status: 'awaiting', occurrence: badOcc },
    ],
    missing: [],
    nextChange: null,
    unavailable: [],
    warnings: [],
  };
  const mixedSource = buildRowSourceFromHead('test:choices-mixed-bad', mixedHead, undefined, atHour, undefined, eventChoicesProjection.validateTail)!;
  const mixedModel = eventChoicesProjection.project(
    createBoundedReadView({ hour: atHour, rows: new Map([['main', mixedSource]]) }),
    { events: ['country'], awaiting: false, artwork: false, breaks: [] },
  );
  assert.equal(mixedModel.rows.length, 1);
  assert.equal(mixedModel.rows[0]?.key, goodOcc.id);
  assert.equal(mixedModel.slots?.length, 2);
  assert.equal(mixedModel.slots?.[0]?.kind, 'data');
  assert.deepEqual(mixedModel.slots?.[1], { kind: 'refused', key: 'refused:1', reason: 'The occurrence card is malformed.' });
  assert.equal(mixedModel.titleSummary, '2 events');
  assert.equal(mixedModel.totalRows, 2);
  assert.equal(mixedModel.empty, undefined);

  const onlyBadHead = {
    ...mixedHead,
    total: '1',
    counts: [{ key: 'awaiting', total: '1' }],
    rows: [{ hour: fired, key: 'bad:1', observed: fired, ordinal: '0', live: false, status: 'awaiting', occurrence: badOcc }],
  };
  const onlyBadSource = buildRowSourceFromHead('test:choices-only-bad', onlyBadHead, undefined, atHour, undefined, eventChoicesProjection.validateTail)!;
  const onlyBadModel = eventChoicesProjection.project(
    createBoundedReadView({ hour: atHour, rows: new Map([['main', onlyBadSource]]) }),
    { events: ['country'], awaiting: false, artwork: false, breaks: [] },
  );
  assert.equal(onlyBadModel.rows.length, 0);
  assert.deepEqual(onlyBadModel.slots, [{ kind: 'refused', key: 'refused:0', reason: 'The occurrence card is malformed.' }]);
  assert.equal(onlyBadModel.titleSummary, '1 event');
  assert.equal(onlyBadModel.totalRows, 1);
  assert.equal(onlyBadModel.empty, undefined, 'a tail with one malformed row never claims no events matched');

  const nullAndFutureEdgeOcc = occurrence([
    edge('scriptEventFired', fired, ['GER', 'POL'], { item: null }),
    choose(atHour + 5, 0, 'POL'),
  ]);
  const nullAndFutureSource = buildRowSourceFromHead('test:choices-null-and-future-edge', {
    ...mixedHead,
    total: '2',
    counts: [],
    rows: [
      { hour: fired, key: 'null:choice', observed: fired, ordinal: '0', live: false, status: null, occurrence: null },
      { hour: fired, key: nullAndFutureEdgeOcc.id, observed: fired, ordinal: '1', live: false, status: 'awaiting', occurrence: nullAndFutureEdgeOcc },
    ],
  }, undefined, atHour, undefined, eventChoicesProjection.validateTail)!;
  const nullAndFutureModel = eventChoicesProjection.project(
    createBoundedReadView({ hour: atHour, rows: new Map([['main', nullAndFutureSource]]) }),
    { events: ['country'], awaiting: false, artwork: false, breaks: [] },
  );
  assert.equal(nullAndFutureModel.slots?.length, 2);
  assert.deepEqual(nullAndFutureModel.slots?.[0], { kind: 'refused', key: 'refused:0', reason: 'Event choice was not recorded.' });
  assert.equal(nullAndFutureModel.slots?.[1]?.kind, 'refused');
  assert.match(nullAndFutureModel.slots?.[1]?.kind === 'refused' ? nullAndFutureModel.slots[1].reason : '', /after displayed hour/);
});
