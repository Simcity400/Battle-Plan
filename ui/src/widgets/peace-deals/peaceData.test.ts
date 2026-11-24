import assert from 'node:assert/strict';
import test from 'node:test';
import { deepFreeze } from '../../test-support/frozen.ts';
import { peaceBlockRows, peaceChangeHours, peacePlayback } from './peaceData.ts';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { TooltipContent } from '../../charts/tooltip';
import { tooltipText } from '../../charts/tooltip.ts';
import { peaceDealHour, peaceDealKnownAt, peaceDealLabel, validatePeaceDealsReply } from '../../dashboard/peaceDealsData.ts';
import { validateStateNamesReply } from '../../dashboard/stateNamesData.ts';
import { contentKey, storableToken } from '../../dashboard/options.ts';
import { chipsByValue, derivedNote, filterPeaceLog, peaceActionChoices, peaceActionName, peaceActions, peaceActionTotals, peaceDescription, peaceGapHint, peaceGapsIn, peaceRowsAt, peaceTooltipRows, peaceWanted, preparePeaceLog } from './peaceData.ts';
import { definition } from './definition.ts';
import { resolveWindow } from '../../dashboard/model.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import type { PeaceCapturedAction, PeaceDeal, PeaceDealsReply, PeaceItem } from '../../generated/dashboard';


const item = (state = 1, afterHour = 20): PeaceItem => ({ derived: true, dates: { beforeHour: afterHour - 3, afterHour }, winner: 'winner/opaque',
  action: { kind: 'stateTransfer', key: 'mod_action', name: 'Modded action' },
  change: { kind: 'stateTransfer', state, from: 'loser:opaque', to: 'winner/opaque', fromCountry: { slot: 1, countryId: 101, tag: 'loser:opaque', hour: afterHour - 3 }, toCountry: { slot: 2, countryId: 102, tag: 'winner/opaque', hour: afterHour }, victoryPoints: '3', victoryPointsReadHour: afterHour } });
function deal(id = 'conference', hour = 20): PeaceDeal {
  return { id, derived: true, observation: 'recordedConference', conferenceId: id, warName: null, firstObservedHour: hour - 5,
    lastObservedHour: hour - 1, currentTurn: 2, finalized: null, firstFinalizedHour: null, endHour: hour, endIsBy: true,
    firstAbsentHour: hour, participantsDerived: true, winners: ['winner/opaque'], losers: ['loser:opaque'], items: [item(1, hour)],
    captured: null, captureUnavailable: null, transfers: { nodes: [], links: [] }, limitations: [] };
}
function reply(deals = [deal()]): PeaceDealsReply {
  return { branchId: '1', derived: true, deals, transfers: { nodes: [], links: [] }, warnings: [],
    unobserved: { settlements: [], conferences: [], ownership: [], wars: [], subjects: [], governments: [], existence: [], victoryPoints: [] } };
}
function capturedAction(state = 4): PeaceCapturedAction {
  return { derived: false, country: 1, countryTag: 'winner/opaque', turn: 1, ordinal: state, token: 900,
    tokenName: 'opaque-action', nameKey: 'MODDED_KEY', name: 'A game action', nameCountry: null, taker: 1, takerTag: 'winner/opaque',
    giver: 2, giverTag: 'loser:opaque', negotiator: 1, negotiatorTag: 'winner/opaque', status: 1, cost: 10, form: 1,
    state, flags: null, countryOperand: null, countryOperandTag: null, puppetAuxiliary: null, ship: null, ratio: null };
}

test('peace playback retains visible content until another deal or capture refusal becomes visible', () => {
  const first = Object.assign(deal('first', 20), { captureUnavailable: 'Capture unavailable', captureUnavailableHour: 25 });
  const log = deepFreeze(preparePeaceLog(deepFreeze(reply([first, deal('next', 30)])))), playback = peacePlayback(log, -Infinity);
  for (const hour of [19, 20, 24, 25, 29, 30, 50, 24]) assert.deepEqual(playback.at(hour), peaceRowsAt(log, hour));
  const visible = playback.at(20);
  assert.equal(playback.at(24), visible);
  assert.notEqual(playback.at(25), visible);
  assert.equal(playback.at(25), playback.at(29));
  assert.equal(playback.at(30), playback.at(50));
});

test('following, a block draws the states of each conference once, and its clock shows at every hour exactly the cards of that hour', () => {
  const first = Object.assign(deal('first', 20), { captureUnavailable: 'Capture unavailable', captureUnavailableHour: 25 });
  const log = deepFreeze(preparePeaceLog(deepFreeze(reply([first, deal('next', 30)]))));
  const changes = peaceChangeHours(log, -Infinity);
  assert.ok(changes.includes(25) && changes.every((hour, index) => index === 0 || changes[index - 1] < hour), `the capture refusal is a change: ${changes}`);
  const block = peaceBlockRows(log, -Infinity, 19, 50, changes);
  for (let hour = 19; hour <= 50; hour++) {
    const shown = block.filter(row => Math.max(row.hour, row.revealedHour ?? row.hour) <= hour && (row.leftHour === undefined || row.leftHour > hour));
    assert.deepEqual(shown.map(row => [row.id, row.item.deal]), peaceRowsAt(log, hour).map(row => [row.key, row.item.deal]), `the cards at ${hour}`);
  }
  assert.equal(new Set(block.map(row => row.key)).size, block.length, 'each state its own key');
});

test('one card per conference newest first, Live cutoff, whole campaign and honest dates', () => {
  const data = reply([deal('old', 10), deal('new', 30)]);
  const log = preparePeaceLog(data);
  assert.deepEqual(log.rows.map(row => row.hour), [30, 10]);
  assert.equal(peaceRowsAt(log, 9).length, 0);
  assert.deepEqual(peaceRowsAt(log, 20).map(row => row.hour), [10]);
  assert.equal(peaceRowsAt(log, null).length, 2);
  assert.match(peaceDealLabel(data.deals[0]), /^by .*War name not recorded$/);
  assert.match(peaceDescription(log.rows[0]), /Estimated between/);
  assert.equal(peaceDealHour(data.deals[1]), 30);
});

test('action categories filter whole conferences before playback and keep the log gaps', () => {
  const mixed = deal('mixed', 20);
  mixed.items.push({ ...item(2), action: { kind: 'stateTransfer', key: 'mod action,[unrecorded]', name: 'Another modded action' } });
  const future = deal('future', 40);
  future.items = [{ ...item(3, 40), action: mixed.items[1].action }];
  const data = reply([deal('other', 10), mixed, future]);
  data.unobserved.ownership = [{ firstHour: 12, lastHour: 18 }];
  const log = preparePeaceLog(data);
  const category = peaceActionChoices(log).find(choice => choice.label === 'Another modded action')!;
  assert.ok(storableToken(category.value));
  const filtered = filterPeaceLog(log, [contentKey(category.value)]);
  assert.deepEqual(filtered.rows.map(row => row.item.deal.id), ['future', 'mixed']);
  assert.equal(filtered.gaps, log.gaps);
  const card = filtered.rows[1];
  assert.equal(card, log.rows.find(row => row.item.deal.id === 'mixed'));
  assert.equal(card.item.actions.length, 2);
  assert.deepEqual(peaceRowsAt(filtered, 15), []);
  assert.deepEqual(peaceRowsAt(filtered, 20).map(row => row.hour), [20]);
  assert.equal(filterPeaceLog(log, []), log);
});

test('unrecorded category includes unnamed actions and empty failed captures without colliding with content keys', () => {
  const empty = deal('empty', 30); empty.items = []; empty.captureUnavailable = 'Unread finalizer operands';
  const unnamed = deal('unnamed', 20); unnamed.items[0].action = { kind: 'stateTransfer', key: null, name: null };
  const named = deal('named', 10); named.items[0].action = { kind: 'stateTransfer', key: '["unrecorded"]', name: 'A content-defined action' };
  const exactEmpty = deal('exactEmpty', 40); exactEmpty.items = []; exactEmpty.derived = false;
  exactEmpty.captured = { hour: 40, publicationHour: 40, winners: [1], losers: [2], actions: [], transfers: [], subjects: [] };
  const log = preparePeaceLog(reply([empty, unnamed, named, exactEmpty]));
  const choices = peaceActionChoices(log);
  const unknown = choices.find(choice => choice.label === 'Action not recorded')!;
  const known = choices.find(choice => choice.label === 'A content-defined action')!;
  const zero = choices.find(choice => choice.label === 'No actions recorded')!;
  assert.notEqual(unknown.value, known.value);
  assert.deepEqual(filterPeaceLog(log, [contentKey(unknown.value)]).rows.map(row => row.item.deal.id), ['empty', 'unnamed']);
  assert.deepEqual(filterPeaceLog(log, [contentKey(known.value)]).rows.map(row => row.item.deal.id), ['named']);
  assert.deepEqual(filterPeaceLog(log, [contentKey(zero.value)]).rows.map(row => row.item.deal.id), ['exactEmpty']);
});
test('derived groups keep game action labels, every state, opaque tags and unknown takers', () => {
  const card = deal();
  card.items = Array.from({ length: 12 }, (_, state) => item(state));
  card.items.push({ ...item(20), winner: null, action: { kind: 'stateTransfer', key: null, name: null } });
  const actions = peaceActions(card);
  assert.equal(actions.length, 2);
  assert.equal(actions[0].name, 'Modded action');
  assert.equal(actions[0].chips.length, 12);
  assert.equal(actions[1].winner, null);
  assert.equal(actions[1].name, null);
  assert.match(derivedNote(card), /between Hour 17 and Hour 20/);
});
test('fitted card lists retain every participant and target in the full tooltip', () => {
  const card = deal();
  card.winners = Array.from({ length: 80 }, (_, i) => `winner:${i}`);
  card.losers = Array.from({ length: 90 }, (_, i) => `loser:${i}`);
  card.items = Array.from({ length: 100 }, (_, state) => item(state));
  const rows = peaceTooltipRows({ deal: card, actions: peaceActions(card), hour: 20, knownAt: 20, by: true }, () => tag => ({ name: tag }), chip => `state:${chip.state}`);
  const labels = new Set(rows.map(row => row.label));
  for (const name of [...card.winners, ...card.losers, ...card.items.map(entry => `state:${entry.change.kind === 'stateTransfer' ? entry.change.state : ''}`)]) {
    assert.ok(labels.has(name), name);
  }
});
test('exact actions group by taker and native action, preserve country targets and automatic outcomes', () => {
  const card = deal();
  card.derived = false;
  card.endIsBy = false;
  card.items = [];
  card.captured = { hour: 19, publicationHour: 20, winners: [1], losers: [2], actions: [capturedAction(4), capturedAction(5),
    { ...capturedAction(), ordinal: 10, state: null, countryOperand: 3, countryOperandTag: 'new-country', takerTag: null }],
    transfers: [{ derived: false, hour: 19, state: 7, taker: 1, to: 'winner/opaque', giver: null, from: null, fromCountry: null, toCountry: null, automatic: true }], subjects: [] };
  const actions = peaceActions(card);
  assert.equal(actions.length, 3);
  assert.deepEqual(actions[0].chips.map(chip => chip.state), [4, 5]);
  assert.equal(actions[1].winner, null);
  assert.equal(actions[1].chips[0].tag, 'new-country');
  assert.equal(actions[2].chips[0].state, 7);
  assert.equal(derivedNote(card), '');
  assert.equal(peaceDealHour(card), 19);
  assert.ok(!peaceDealLabel(card).startsWith('by '));
  validatePeaceDealsReply(reply([card]));
});
test('captured subject results use exact country chips without inventing a named action or borrowing its operand', () => {
  const card = deal();
  card.derived = false;
  card.items = [];
  card.captured = { hour: 19, publicationHour: 20, winners: [1], losers: [2], transfers: [],
    actions: [{ ...capturedAction(), state: null, countryOperand: 3, countryOperandTag: 'original-operand' }],
    subjects: [
      { derived: false, overlord: 1, subject: 30, overlordTag: 'winner/opaque', subjectTag: 'newly-allocated' },
      { derived: false, overlord: 31, subject: 32, overlordTag: null, subjectTag: null },
    ] };
  const rows = peaceActions(card);
  assert.equal(rows.length, 3);
  assert.equal(peaceActionName(rows[0]), 'A game action');
  assert.equal(rows[0].chips[0].tag, 'original-operand');
  assert.equal(peaceActionName(rows[1]), 'Outcome; action unknown');
  assert.equal(rows[1].winner, 'winner/opaque');
  assert.deepEqual(rows[1].chips.map(chip => [chip.tag, chip.hour, chip.derived, chip.note]),
    [['newly-allocated', 19, false, 'Subject']]);
  assert.equal(rows[2].winner, null);
  assert.equal(rows[2].chips[0].tag, undefined);
  assert.equal(rows[2].chips[0].missing, 'Subject country not recorded');
});

test('a war-end outcome joins the one recorded conference at its hour naming its winners and losers, with its states and victory points', () => {
  const conference = deal('conference', 20); conference.derived = false; conference.items = []; conference.endIsBy = false;
  conference.captured = { hour: 20, publicationHour: 20, winners: [2], losers: [1], actions: [], subjects: [],
    transfers: [{ derived: false, hour: 20, state: 1, taker: 1, to: 'winner/opaque', giver: null, from: null, fromCountry: null, toCountry: null, automatic: true },
      { derived: false, hour: 20, state: 2, taker: 1, to: 'winner/opaque', giver: null, from: null, fromCountry: null, toCountry: null, automatic: true }] };
  const outcome = deal('outcome', 20); outcome.observation = 'conferenceNotRecorded'; outcome.conferenceId = null;
  outcome.items = [item(1, 20), { ...item(2, 20), change: { ...item(2, 20).change, victoryPoints: '7' } as PeaceItem['change'] }];
  const elsewhere = deal('elsewhere', 30); elsewhere.observation = 'conferenceNotRecorded'; elsewhere.conferenceId = null;
  const log = preparePeaceLog(reply([elsewhere, conference, outcome]));
  assert.deepEqual(log.rows.map(row => row.item.deal.id), ['elsewhere', 'conference']);
  const card = log.rows[1].item;
  assert.equal(card.warEnd, true);
  assert.equal(log.rows[0].item.warEnd, undefined);
  assert.equal(card.actions.length, 1);
  assert.equal(peaceActionName(card.actions[0]), 'Automatic transfer');
  assert.deepEqual(peaceActionTotals(card.actions[0]), { states: 2, vp: 10n, warScore: null });
  assert.deepEqual(chipsByValue(card.actions[0].chips).map(chip => chip.state), [2, 1]);
  assert.match(peaceDescription(log.rows[1]), /read from ownership when the war ended/);
  const reversed = { ...outcome, winners: outcome.losers, losers: outcome.winners };
  assert.equal(preparePeaceLog(reply([conference, reversed])).rows.length, 2);
  const twin = { ...conference, id: 'twin' };
  assert.equal(preparePeaceLog(reply([conference, twin, outcome])).rows.length, 3);
});
test('an outcome fitting two conferences stays its own card whichever outcome joined first', () => {
  const conference = (id: string, winners: string[], losers: string[]) => {
    const card = deal(id, 20); card.derived = false; card.items = []; card.endIsBy = false; card.winners = winners; card.losers = losers;
    card.captured = { hour: 20, publicationHour: 20, winners: [], losers: [], actions: [], subjects: [], transfers: [] };
    return card;
  };
  const outcome = (id: string, winners: string[], losers: string[]) => {
    const card = deal(id, 20); card.observation = 'conferenceNotRecorded'; card.conferenceId = null; card.winners = winners; card.losers = losers;
    return card;
  };
  const a = conference('a', ['A', 'B'], ['X', 'Y']), b = conference('b', ['B'], ['Y']);
  const first = outcome('first', ['A'], ['X']), second = outcome('second', ['B'], ['Y']);
  for (const deals of [[a, b, first, second], [a, b, second, first]]) {
    const rows = preparePeaceLog(reply(deals)).rows;
    assert.deepEqual(rows.map(row => row.item.deal.id).sort(), ['a', 'b', 'second']);
    assert.equal(rows.find(row => row.item.deal.id === 'a')?.item.warEnd, true);
    assert.equal(rows.find(row => row.item.deal.id === 'b')?.item.warEnd, undefined);
  }
  const third = outcome('third', ['B'], ['X']);
  assert.deepEqual(preparePeaceLog(reply([a, b, first, third])).rows.map(row => row.item.deal.id).sort(), ['a', 'b']);
});
test('war score sums the captured actions; victory points total only when every state records them', () => {
  const card = deal(); card.derived = false; card.items = [item(4, 19)];
  card.captured = { hour: 19, publicationHour: 20, winners: [1], losers: [2], actions: [capturedAction(4), capturedAction(5)], transfers: [], subjects: [] };
  const [row] = peaceActions(card);
  assert.deepEqual(row.chips.map(chip => [chip.state, chip.vp]), [[4, '3'], [5, null]]);
  assert.deepEqual(peaceActionTotals(row), { states: 2, vp: null, warScore: 20 });
  assert.deepEqual(chipsByValue(row.chips).map(chip => chip.state), [4, 5]);
  const rows = peaceTooltipRows({ deal: card, actions: [row], hour: 20, knownAt: 20, by: false }, () => tag => ({ name: tag }), chip => `State ${chip.state}`);
  assert.equal(rows.find(entry => entry.label === 'War score')?.value, '20');
  assert.equal(rows.find(entry => entry.label === 'State 4')?.value, '3 VP');
  assert.equal(rows.find(entry => entry.label === 'State 5')?.value, undefined);
});

test('unresolved country operands never borrow giver tags, and captured automatic outcomes precede later fallback', () => {
  const card = deal();
  card.derived = false;
  card.captured = { hour: 19, publicationHour: 20, winners: [1], losers: [2], actions: [
    { ...capturedAction(), state: null, countryOperand: 99, countryOperandTag: null },
    { ...capturedAction(), ordinal: 10, state: -1 }],
    transfers: [{ derived: false, hour: 19, state: 1, taker: 1, to: 'winner/opaque', giver: null, from: null, fromCountry: null, toCountry: null, automatic: true }], subjects: [] };
  const actions = peaceActions(card);
  assert.equal(actions[0].chips.length, 2);
  assert.ok(actions[0].chips.every(chip => chip.tag == null && chip.missing === 'Target not recorded'));
  assert.equal(actions[1].chips[0].state, 1);
  assert.equal(actions[1].chips[0].hour, 19);
  assert.equal(actions[1].chips[0].derived, false);
  assert.equal(actions.length, 2);
});
const rosterWindows = (days: number) => Array.from({ length: days }, (_, day) => ({ firstHour: day * 24 + 22, lastHour: day * 24 + 44 }));
test('hours between daily conference roster reads are cadence, never log rows or a gap hint', () => {
  const data = reply([]);
  data.unobserved.conferences = rosterWindows(30);
  const log = preparePeaceLog(data);
  assert.deepEqual(log.rows, []);
  assert.deepEqual(log.gaps, []);
  assert.deepEqual(peaceRowsAt(log, 400), []);
  assert.deepEqual(peaceRowsAt(log, null), []);
  assert.equal(peaceGapHint(peaceGapsIn(log.gaps, 400)), null);
  const cards = reply([deal('old', 100), deal('new', 300)]);
  cards.unobserved.conferences = rosterWindows(30);
  assert.deepEqual(peaceRowsAt(preparePeaceLog(cards), 400).map(row => row.item.deal.id), ['new', 'old']);
});
test('hourly input gaps merge into one hint clipped to the shown time without exposing later dates', () => {
  const data = reply([]);
  data.unobserved.settlements = [{ firstHour: 10, lastHour: 12 }];
  data.unobserved.wars = [{ firstHour: 20, lastHour: 30 }];
  data.unobserved.ownership = [{ firstHour: 25, lastHour: 45 }, { firstHour: 60, lastHour: 61 }];
  data.unobserved.conferences = [{ firstHour: 11, lastHour: 70 }];
  const log = preparePeaceLog(data);
  assert.deepEqual(log.gaps, [{ firstHour: 10, lastHour: 12 }, { firstHour: 20, lastHour: 45 }, { firstHour: 60, lastHour: 61 }]);
  assert.deepEqual(peaceRowsAt(log, 50), [], 'gaps are never log rows');
  assert.deepEqual(peaceGapsIn(log.gaps, 9), []);
  assert.deepEqual(peaceGapsIn(log.gaps, 25), [{ firstHour: 10, lastHour: 12 }, { firstHour: 20, lastHour: 25 }]);
  assert.deepEqual(peaceGapsIn(log.gaps, 50, 22), [{ firstHour: 22, lastHour: 45 }]);
  assert.deepEqual(peaceGapsIn(log.gaps, 59, 46), []);
  assert.equal(peaceGapHint(peaceGapsIn(log.gaps, 15)), 'Not fully recorded between Hour 10 and Hour 12.');
  assert.equal(peaceGapHint(peaceGapsIn(log.gaps, 25)), '2 stretches not fully recorded between Hour 10 and Hour 25.');
});
test('names and flags are requested at each conference or chip hour, only drawn cards', () => {
  const card = deal(); card.items.push(item(2, 18));
  const rows = preparePeaceLog(reply([card, deal('other', 40)])).rows;
  const wanted = peaceWanted(rows.slice(1));
  assert.deepEqual(wanted.states, [{ hour: 20, state: 1 }, { hour: 18, state: 2 }]);
  assert.ok(wanted.countries.every(country => country.hour === 20));
  assert.deepEqual(wanted.countries.map(country => country.tag), ['winner/opaque', 'loser:opaque']);
});
test('validation rejects incompatible shape, duplicate conferences, unknown endpoints and partial VP sums', () => {
  const data = reply();
  assert.equal(validatePeaceDealsReply(data), data);
  assert.throws(() => validatePeaceDealsReply({ ...data, unobserved: undefined }), /malformed/);
  assert.throws(() => validatePeaceDealsReply({ ...data, deals: [deal(), deal()] }), /duplicate/);
  const fromCountry = { slot: 1, countryId: 101, tag: 'opaque', hour: 1 };
  const toCountry = { slot: 2, countryId: 102, tag: 'opaque2', hour: 2 };
  const graph = { nodes: [{ id: 'a', country: fromCountry, side: 'source' }, { id: 'b', country: toCountry, side: 'target' }],
    links: [{ id: 'ab', source: 'a', target: 'b', stateCount: 2, victoryPoints: '3', states: [
      { state: 1, fromCountry, toCountry, dates: { beforeHour: 1, afterHour: 2 }, victoryPoints: '3', victoryPointsReadHour: 2 },
      { state: 2, fromCountry, toCountry, dates: { beforeHour: 1, afterHour: 2 }, victoryPoints: null, victoryPointsReadHour: null }] }] };
  assert.throws(() => validatePeaceDealsReply({ ...data, transfers: graph }), /malformed/);
  graph.links[0].victoryPoints = null as unknown as string;
  validatePeaceDealsReply({ ...data, transfers: graph });
  assert.throws(() => validatePeaceDealsReply({ ...data, transfers: { ...graph,
    links: [{ ...graph.links[0], stateCount: 0, states: [], victoryPoints: '0' }] } }), /malformed/);
  graph.links[0].target = 'missing';
  assert.throws(() => validatePeaceDealsReply({ ...data, transfers: graph }), /malformed/);
});
test('state labels preserve missing values and read hours', () => {
  const data = { places: [], states: [{ stateId: 4, hour: 20, name: null, missing: 'Not recorded' }], warnings: [] };
  assert.equal(validateStateNamesReply(data), data);
  assert.throws(() => validateStateNamesReply({ ...data, states: [{ ...data.states[0], hour: null }] }), /malformed/);
  assert.throws(() => validateStateNamesReply({ ...data, states: [{ stateId: 4, hour: 20, name: 0, missing: null }] }), /malformed/);
});

function world(conferences: number): PeaceDealsReply {
  const cards = Array.from({ length: conferences }, (_, i) => {
    const card = deal(`conference:${i}`, i + 10);
    card.winners = [`mod-country:${i * 2}`, `mod-country:${i * 2 + 1}`];
    card.losers = [`mod-country:${conferences * 2 + Math.floor(i / 2)}`];
    card.items = [item(i, i + 10)]; card.items[0].winner = card.winners[0];
    return card;
  });
  return reply(cards);
}
test('5,000-country / 2,000-conference log retains every card and only drawn cards ask for names', () => {
  const data = world(2_000);
  const log = preparePeaceLog(data);
  assert.equal(log.rows.length, 2_000);
  const countries = new Set(data.deals.flatMap(card => [...card.winners, ...card.losers]));
  assert.equal(countries.size, 5_000);
  assert.equal(peaceWanted(log.rows.slice(0, 10)).states.length, 10);
});


test('Pin and inclusive range keep event dates but wait for the supplying read', () => {
  assert.deepEqual(definition.timeModes, ['campaign', 'follow', 'pin', 'range']);
  const exact = deal('exact', 112); exact.derived = false; exact.endIsBy = false;
  exact.captured = { hour: 112, publicationHour: 136, winners: [], losers: [], actions: [], transfers: [], subjects: [] };
  const derived = deal('derived', 124); derived.endIsBy = false; derived.firstAbsentHour = 125;
  const data = reply([deal('before', 111), exact, derived, deal('end', 147), deal('future', 148)]);
  data.unobserved.conferences = [{ firstHour: 100, lastHour: 160 }];
  data.unobserved.wars = [{ firstHour: 100, lastHour: 160 }];
  const log = preparePeaceLog(data);
  assert.equal(peaceDealKnownAt(exact), 136); assert.equal(peaceDealKnownAt(derived), 125);
  assert.ok(!peaceRowsAt(log, 135).some(row => row.item.deal.id === 'exact'));
  const branch = { id: '7', sessionId: '9', firstHour: 100, lastHour: 200, main: true, fork: null, joins: [], sampleBoundary: 'Sample origin unknown', gaps: [] };
  const track: BoundTrack = { id: 'main', roleId: 'primary', label: 'Campaign', branchIndex: 0, anchorOffset: null, axisOffset: 0, selectedTag: null, branch,
    source: { id: 'source', path: 'synthetic', revision: '1', metadata: { path: 'synthetic', revision: '1', name: 'Synthetic', format: 1, captureMode: 'full', campaignKey: 'c0ffee',
      branches: [branch], countries: [], playerCountryId: null, contentVerified: false, partial: null, warnings: [], buildWarnings: [] } } };
  const widget: WidgetSpec = { id: 'peace', kind: 'peace-deals', configVersion: 1, title: 'Peace', trackIds: ['main'], countries: { kind: 'all' },
    time: { mode: 'pin', windows: { main: { firstOffset: 36, lastOffset: 36 } } } };
  const pinWindow = resolveWindow(widget, track, [track], 199)!;
  assert.deepEqual(pinWindow, { firstHour: 136, lastHour: 136 });
  const pinned = peaceRowsAt(log, pinWindow.lastHour);
  assert.equal(pinned.find(row => row.item.deal.id === 'exact')?.hour, 112);
  const rangeWindow = resolveWindow({ ...widget, time: { mode: 'range', firstOffset: 12, lastOffset: 47 } }, track, [track], 199)!;
  assert.deepEqual(rangeWindow, { firstHour: 112, lastHour: 147 });
  const range = peaceRowsAt(log, rangeWindow.lastHour, rangeWindow.firstHour);
  assert.deepEqual(range.map(row => row.hour), [147, 124, 112]);
  assert.deepEqual(peaceGapsIn(log.gaps, rangeWindow.lastHour, rangeWindow.firstHour), [{ firstHour: 112, lastHour: 147 }]);
  assert.deepEqual(peaceRowsAt(log, 112, 112), []);
});

test('failed and omitted finalizers wait for their manifest without hiding earlier fallback evidence', () => {
  for (const reason of ['Guarded fault', 'One omitted operand']) {
    const failed = Object.assign(deal('never sampled', 112), { firstObservedHour: 112, lastObservedHour: 112, currentTurn: null,
      endHour: null, endIsBy: false, firstAbsentHour: null, winners: [], losers: [], items: [], captureUnavailable: reason,
      captureObservedHour: 136, captureUnavailableHour: 136, limitations: [reason] });
    const fallback = Object.assign(deal('sampled fallback', 112), { captureUnavailable: reason, captureUnavailableHour: 136, limitations: ['Native fallback limitation', reason] });
    const data = reply([failed, fallback]);
    assert.equal(validatePeaceDealsReply(data), data);
    assert.equal(peaceDealKnownAt(failed), 136);
    assert.equal(peaceDealKnownAt(fallback), 112);
    assert.throws(() => validatePeaceDealsReply(reply([{ ...failed, captureObservedHour: '136' } as unknown as PeaceDeal])), /malformed/);
    const log = preparePeaceLog(data);
    assert.ok(!peaceRowsAt(log, 112).some(row => row.item.deal.id === 'never sampled'));
    const earlier = peaceRowsAt(log, 135);
    assert.equal(earlier.length, 1);
    const card = earlier[0].item;
    assert.equal(card.deal.id, 'sampled fallback');
    assert.equal(card.deal.captureUnavailable, null);
    assert.deepEqual(card.deal.limitations, ['Native fallback limitation']);
    const reached = peaceRowsAt(log, 136);
    assert.equal(reached.length, 2);
    assert.ok(reached.every(row => row.item.deal.captureUnavailable === reason));
    assert.equal(reached.find(row => row.item.deal.id === 'never sampled')?.hour, 112);
  }
});


test('full peace rows retain dated flag status and refused state names beyond fitting', () => {
  const conference = deal();
  const card = { deal: conference, actions: peaceActions(conference), hour: 20, knownAt: 20, by: true };
  card.actions[0].chips.push({ key: 'country-target', hour: 19, tag: 'target', derived: false });
  const rows = peaceTooltipRows(card, hour => tag => ({ name: `${tag}@${hour}`, flagReason: `native flag refusal@${hour}`, flagPending: tag === 'target' }),
    chip => chip.state == null ? `target@${chip.hour}` : `State ${chip.state}`);
  for (const tag of [...conference.winners, ...conference.losers]) assert.equal(rows.find(row => row.label === `${tag}@20`)?.countryFlag?.flagReason, 'native flag refusal@20');
  assert.equal(rows.find(row => row.value === 'Modded action')?.countryFlag?.flagReason, 'native flag refusal@20');
  assert.deepEqual(rows.find(row => row.label === 'target@19')?.countryFlag,
    { name: 'target@19', flagReason: 'native flag refusal@19', flagPending: true });
  assert.equal(rows.find(row => row.label === 'State 1')!.value, '3 VP');
});


test('actual Peace HTML: refusal metadata, an empty state without cadence rows and one gap note', async () => {
  const host = globalThis as typeof globalThis & { __peaceFixture?: unknown; __peaceTips?: TooltipContent[] };
  const replacements: Record<string, string> = {
    '/dashboard/usePeaceDeals.ts': 'export const usePeaceDeals = () => globalThis.__peaceFixture;',
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = () => ({ labelAt: hour => tag => ({ name: tag,
      flagPending: tag === 'winner/opaque', flagReason: 'Native flag refusal at ' + hour }) });`,
    '/dashboard/useStateNamesAt.ts': `export const useStateNamesAt = () => () => ({ name: null, missing: 'Unsupported native mod state-name form' });`,
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ font: "synthetic" });',
    '/charts/log/LogList.tsx': `import { createElement } from 'react'; export default props => {
      globalThis.__peaceTips = props.rows.flatMap(props.tooltip);
      return createElement('div', null, ...(props.rows.length ? props.rows.map(props.renderRow) : [props.empty])); };`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'peace-refusal-fixture', enforce: 'pre',
      transform(_source, id) { return Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1]; } }] });
  try {
    const { default: PeaceDealsWidget } = await vite.ssrLoadModule('/ui/src/widgets/peace-deals/PeaceDealsWidget.tsx');
    const track = { id: 'track', source: { id: 'source', revision: '1' }, branch: { id: '1', firstHour: 0, lastHour: 30, gaps: [] }, axisOffset: 0 };
    const render = (data: PeaceDealsReply, time: unknown = { mode: 'campaign' }) => {
      host.__peaceFixture = { track, data };
      return renderToStaticMarkup(createElement(PeaceDealsWidget, { widget: { trackIds: ['track'], time, options: { chips: false } },
        runtime: { client: {}, tracks: [track], clock: { getHour: () => 20 } }, active: true, onDiagnostics() {} } as unknown as WidgetProps));
    };
    const html = render(reply());
    assert.match(html, /role="img" aria-label="Flag loading"/, 'mounted taker uses the canonical accessible flag');
    assert.doesNotMatch(html, /data-tooltip="[^"]*Unsupported native mod state-name form/, 'a fitted-away state hides its internal refusal');
    assert.match(html, /Flag unavailable/, 'fitted-away participants retain flag refusal text');
    const spoken = host.__peaceTips!.map(tooltipText).join('; ');
    assert.doesNotMatch(spoken, /Native flag refusal/);
    assert.doesNotMatch(spoken, /Unsupported native mod state-name form/);
    const quiet = reply([]);
    quiet.unobserved.conferences = rosterWindows(2);
    const empty = render(quiet);
    assert.match(empty, /No peace conferences yet\./);
    assert.doesNotMatch(empty, /Not recorded|not fully recorded|may be missing/, 'roster cadence is neither a row nor a note');
    const missed = reply([]);
    missed.unobserved.ownership = [{ firstHour: 5, lastHour: 6 }, { firstHour: 25, lastHour: 26 }];
    const noted = render(missed, { mode: 'range', firstOffset: 0, lastOffset: 20 });
    assert.equal(noted.match(/Some conferences may be missing\./g)?.length, 1);
    assert.match(noted, /data-tooltip="Not fully recorded between Hour 5 and Hour 6\."/, 'a later gap stays out of the shown time');
    assert.match(noted, /No peace conferences yet\./);
  } finally { await vite.close(); delete host.__peaceFixture; delete host.__peaceTips; }
});

test('a puppet demand is named with the country it puppets, at its hour, one row each', () => {
  const card = deal();
  card.derived = false;
  card.items = [];
  const puppet = (ordinal: number, tag: string): PeaceCapturedAction => ({ ...capturedAction(), ordinal, state: null, tokenName: 'puppet', nameKey: null,
    name: 'Puppet {COUNTRY}', nameCountry: tag, form: 2, countryOperand: ordinal, countryOperandTag: tag });
  card.captured = { hour: 19, publicationHour: 20, winners: [1], losers: [2], transfers: [], subjects: [], actions: [puppet(30, 'SLO'), puppet(31, 'CRO')] };
  validatePeaceDealsReply(reply([card]));
  const rows = peaceActions(card);
  const named = (at: { hour: number; tag: string }) => `${at.tag === 'SLO' ? 'Slovakia' : 'Croatia'} at ${at.hour}`;
  assert.deepEqual(rows.map(row => peaceActionName(row, named)), ['Puppet Slovakia at 19', 'Puppet Croatia at 19']);
  assert.deepEqual(rows.map(row => peaceActionName(row)), ['Puppet', 'Puppet'], 'without a country name the label stands without it, never its placeholder');
  const log = preparePeaceLog(reply([card]));
  assert.deepEqual(peaceActionChoices(log).map(choice => choice.label), ['Puppet']);
  assert.ok(peaceWanted(log.rows).countries.some(at => at.tag === 'SLO' && at.hour === 19));
  const tooltip = peaceTooltipRows(log.rows[0].item, () => tag => ({ name: tag === 'SLO' ? 'Slovakia' : tag }), chip => chip.key);
  assert.ok(tooltip.some(row => row.value === 'Puppet Slovakia'));
});
