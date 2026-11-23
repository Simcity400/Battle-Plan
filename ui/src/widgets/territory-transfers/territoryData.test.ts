import assert from 'node:assert/strict';
import test from 'node:test';
import { validatePeaceDealsReply } from '../../dashboard/peaceDealsData.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import type { WidgetSpec } from '../../dashboard/model';
import { chosenConference, conferenceChoiceId, conferenceToken, countryBalances, decodeTransfers, involving, netVictoryPoints, transferConferenceLabel, transferCutoff,
  transferDate, transferEmptyMessage, transferGapHint, transferGaps, transferHow, transferIsBy, transferScope, transfersAt, visibleConferences } from './territoryData.ts';
import { definition, OPTIONS } from './definition.ts';
import { deal, holder, olderBridge, reply } from './transfers.fixture.ts';
import type { Move } from './transfers.fixture.ts';
import { assertLinearWork, measureWork } from '../../test-support/work.ts';

const loser = (hour: number) => holder(1, 'LOS', hour);
const winner = (hour: number) => holder(2, 'WIN', hour);
const freed = (hour: number) => holder(3, 'NEW', hour);
const other = (hour: number) => holder(4, 'OTH', hour);
const move = (state: number, from: Move['from'], to: Move['to'], after: number, extra: Partial<Move> = {}): Move => ({ state, from, to, before: after - 1, after, ...extra });

function settlement() {
  return deal('peace', [
    move(1, loser(100), winner(101), 101, { vp: '5' }),
    move(2, loser(100), winner(101), 101, { vp: '-2' }),
    move(3, loser(100), freed(101), 101, { vp: '7', released: true }),
  ]);
}

test('metadata offers the shared country chip over the whole world and keeps old settings', () => {
  assert.equal(definition.version, 2);
  assert.equal((definition as { countrySelection?: boolean }).countrySelection, undefined);
  assert.equal(definition.countries.all, true);
  assert.equal(definition.defaultCountries.kind, 'all');
  const old = { id: 'w', kind: 'territory-transfers', configVersion: 1, options: { measure: 'victoryPoints', colour: 'taker', show: 'parts' } } as unknown as WidgetSpec;
  assert.deepEqual(definition.migrate(1, old).options, old.options);
  const show = OPTIONS.find(option => option.key === 'show')!;
  assert.deepEqual(show.kind === 'choice' && show.choices.map(choice => choice.label), ['Countries', 'Transfers', 'Country transfers']);
  for (const removed of ['measure', 'colour']) assert.equal(OPTIONS.some(option => option.key === removed), false);
});

test('released states join the occurrences with both holders, their own former owner and VP', () => {
  const data = decodeTransfers(validatePeaceDealsReply(reply([settlement()])));
  const released = data.all.find(occurrence => occurrence.state === 3)!;
  assert.equal(released.released, true);
  assert.equal(released.from?.tag, 'LOS');
  assert.equal(released.from?.key, '[1,1001]');
  assert.equal(released.to.key, '[3,1003]');
  assert.equal(released.victoryPoints, 7n);
  assert.equal(released.derived, true);
  assert.equal(data.all.filter(occurrence => !occurrence.released).length, 2);
});

test('a reply from an older bridge still lists released states, without victory points', () => {
  const older = olderBridge(reply([settlement()]));
  assert.doesNotThrow(() => validatePeaceDealsReply(older));
  const data = decodeTransfers(older);
  const released = data.all.find(occurrence => occurrence.state === 3)!;
  assert.equal(released.released, true);
  assert.equal(released.from?.key, '[1,1001]');
  assert.equal(released.to.tag, 'NEW');
  assert.equal(released.victoryPoints, null);
  const [balance] = countryBalances(data.all).filter(row => row.side.tag === 'NEW');
  assert.equal(balance.gained, 1);
  assert.equal(netVictoryPoints(balance), null);
  const shared = olderBridge(reply([deal('shared', [move(1, loser(100), winner(101), 101), move(5, other(100), winner(101), 101),
    move(3, loser(100), freed(101), 101, { released: true }), move(4, other(100), freed(101), 101, { released: true })])]));
  const unnamed = decodeTransfers(shared).all.filter(occurrence => occurrence.released);
  assert.deepEqual(unnamed.map(occurrence => occurrence.from), [null, null]);
  assert.equal(countryBalances(unnamed).find(row => row.side.tag === 'NEW')!.unattributed, 2);
});

test('balances net both sides, count repeated states and never sum unknown victory points', () => {
  const data = decodeTransfers(reply([
    deal('first', [move(1, loser(100), winner(101), 101, { vp: '5' }), move(2, loser(100), winner(101), 101, { vp: null })]),
    deal('second', [move(1, winner(200), loser(201), 201, { vp: '5' }), move(9, other(200), winner(201), 201, { vp: '3' })]),
  ]));
  const rows = new Map(countryBalances(data.all).map(row => [row.side.tag, row]));
  const win = rows.get('WIN')!, los = rows.get('LOS')!;
  assert.deepEqual([win.gained, win.lost, los.gained, los.lost], [3, 1, 1, 2]);
  assert.equal(win.gainedVictoryPoints, null);
  assert.equal(netVictoryPoints(win), null);
  assert.equal(rows.get('OTH')!.lostVictoryPoints, 3n);
  assert.equal(netVictoryPoints(rows.get('OTH')!), -3n);
  assert.deepEqual([...win.from.values()].map(part => [part.side.tag, part.count]), [['LOS', 2], ['OTH', 1]]);
  assert.deepEqual([win.first, win.last], [101, 201]);
  assert.equal(involving(data.all, win.key).length, 4);
});

test('a renamed holder stays one row under its newest name; a reused tag stays separate', () => {
  const renamed = holder(1, 'NEWNAME', 300);
  const reused = holder(7, 'LOS', 400, 9007);
  const data = decodeTransfers(reply([
    deal('a', [move(1, loser(100), winner(101), 101)]), deal('b', [move(2, renamed, winner(301), 301)]), deal('c', [move(3, reused, winner(401), 401)]),
  ]));
  const losers = countryBalances(data.all).filter(row => row.lost);
  assert.deepEqual(losers.map(row => [row.key, row.side.tag, row.lost]).sort(), [['[1,1001]', 'NEWNAME', 2], ['[7,9007]', 'LOS', 1]]);
});

test('playback reaches each transfer at its date and supplying read; Live waits for each VP read', () => {
  const late = deal('late', [move(1, loser(100), winner(101), 101, { vp: '4', vpHour: 120 })], { captured: { hour: 101, publicationHour: 110, winners: [], losers: [], actions: [], transfers: [], subjects: [] } });
  const data = decodeTransfers(reply([late]));
  const at = (hour: number, campaign = false) => transfersAt(data.all, transferCutoff(data.hours, hour), -Infinity, campaign);
  assert.equal(at(105).length, 0);
  assert.equal(at(110)[0].victoryPoints, null);
  assert.equal(at(110)[0].victoryPointsPending, true);
  assert.equal(at(120)[0].victoryPoints, 4n);
  assert.equal(at(110, true)[0].victoryPoints, 4n);
  assert.equal(transfersAt(data.all, 200, 101).length, 1);
  assert.equal(transfersAt(data.all, 200, 102).length, 0);
  assert.equal(transferScope(data, 'late').length, 1);
  assert.equal(transferScope(data, 'missing').length, 0);
});

test('only hourly settlement, ownership and war gaps can hide a transfer', () => {
  const daily = reply([], { conferences: [{ firstHour: 101, lastHour: 123 }, { firstHour: 125, lastHour: 147 }], subjects: [{ firstHour: 100, lastHour: 200 }] });
  assert.deepEqual(transferGaps(daily, 100, 200), []);
  assert.equal(transferGapHint(transferGaps(daily, 100, 200)), null);
  const missing = reply([], { ownership: [{ firstHour: 150, lastHour: 160 }], wars: [{ firstHour: 158, lastHour: 170 }], settlements: [{ firstHour: 190, lastHour: 210 }] });
  assert.deepEqual(transferGaps(missing, 100, 200), [{ firstHour: 150, lastHour: 170 }, { firstHour: 190, lastHour: 200 }]);
  assert.equal(transferGapHint(transferGaps(missing, 100, 200)), `2 stretches not fully recorded between ${formatDateTime(150)} and ${formatDateTime(200)}.`);
  assert.equal(transferEmptyMessage(false), 'No territory transfers yet.');
  assert.equal(transferEmptyMessage(true), 'No territory transfers for these countries yet.');
});

test('how comes from the settlement: release, new subject and the content’s own action name', () => {
  const peace = settlement();
  peace.items.push({ derived: true, dates: { beforeHour: 100, afterHour: 101 }, winner: 'WIN', action: { kind: 'newSubject', key: null, name: null },
    change: { kind: 'newSubject', overlord: 'WIN', subject: 'NEW', autonomyState: 'mod_level', startHour: 101 } });
  peace.captured = { hour: 101, publicationHour: 101, winners: [], losers: [], subjects: [], transfers: [], actions: [
    { derived: false, country: 2, countryTag: 'WIN', turn: 1, ordinal: 1, token: 9, tokenName: null, nameKey: 'mod_take', name: 'Annex Province Group', nameCountry: null, taker: 2, giver: 1, negotiator: 2,
      takerTag: 'WIN', giverTag: 'LOS', negotiatorTag: 'WIN', status: 0, cost: 0, form: 0, state: 1, flags: null, countryOperand: null, countryOperandTag: null, puppetAuxiliary: null, ship: null, ratio: null },
    { derived: false, country: 2, countryTag: 'WIN', turn: 1, ordinal: 2, token: 10, tokenName: 'puppet', nameKey: null, name: 'Puppet {COUNTRY}', nameCountry: 'NEW', taker: 2, giver: 1, negotiator: 2,
      takerTag: 'WIN', giverTag: 'LOS', negotiatorTag: 'WIN', status: 0, cost: 0, form: 2, state: null, flags: null, countryOperand: 3, countryOperandTag: 'NEW', puppetAuxiliary: null, ship: null, ratio: null },
  ] };
  const data = decodeTransfers(reply([peace]));
  const of = (state: number) => transferHow(data.all.find(occurrence => occurrence.state === state)!, data.deals.get('peace'));
  assert.deepEqual(of(1), { released: false, subjectOf: null, action: 'Annex Province Group' });
  assert.deepEqual(of(2), { released: false, subjectOf: null, action: null });
  assert.deepEqual(of(3), { released: true, subjectOf: 'WIN', action: null });
});

test('dates are plain in rows; only a comparison of two reads is "by"', () => {
  const exact = deal('exact', [move(1, loser(100), winner(101), 101, { derived: false, before: 101 } as Partial<Move>)]);
  const data = decodeTransfers(reply([exact, settlement()]));
  const exactOccurrence = data.all.find(occurrence => occurrence.deal === 'exact')!;
  assert.equal(exactOccurrence.derived, false);
  assert.equal(transferIsBy(exactOccurrence), false);
  assert.equal(transferIsBy(data.all.find(occurrence => occurrence.deal === 'peace')!), true);
  assert.doesNotMatch(transferDate(101), /by|·/i);
});

test('conference choices bind campaign and branch and name an unrecorded war plainly', () => {
  const peace = settlement();
  assert.equal(transferConferenceLabel(peace), `by ${formatDateTime(101)} · War name not recorded`);
  const token = conferenceToken('campaign', 'branch', 'peace');
  assert.equal(conferenceChoiceId(token, 'campaign', 'branch'), 'peace');
  assert.equal(conferenceChoiceId(token, 'campaign', 'other'), null);
  assert.equal(chosenConference(visibleConferences([peace], 101), ''), peace);
  assert.equal(chosenConference(visibleConferences([peace], 100), ''), null);
});

test('balances scale linearly to 5,000 countries without a chosen cap', () => {
  assertLinearWork(count => {
    const moves: Move[] = [];
    for (let id = 0; id < count; id++) moves.push(move(id, holder(id, `T${id}`, 100), holder((id + 1) % count, `T${(id + 1) % count}`, 101), 101));
    let rows: ReturnType<typeof countryBalances> = [];
    const work = measureWork(decodeTransfers(reply([deal('world', moves)])).all, transfers => { rows = countryBalances(transfers); });
    assert.equal(rows.length, count);
    assert.ok(rows.every(row => row.gained === 1 && row.lost === 1 && netVictoryPoints(row) === 0n));
    return work;
  }, 'balancing transfers among countries');
});
