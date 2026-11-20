import assert from 'node:assert/strict';
import test from 'node:test';
import { followedRowTags, formatRaw, liveRace, raceLookTags, stateEntries, validateRankedLines, validateRankReply } from './measureRaceData.ts';
import { CELL, parseFollowed } from '../../dashboard/follow/message.ts';
import { message } from '../../dashboard/follow/messages.fixture.ts';
import type { SeriesState } from '../../generated/dashboard';

const state = (tag: string, valueRaw: string | null, missing: string | null = null): SeriesState => ({ country: tag, hour: 50, drop: null, missing,
  run: valueRaw === null ? null : { firstHour: 40, lastHour: 60, valueRaw, slot: 1, countryId: 1, startsSegment: false, segmentFirstHour: 0, firstExact: true, lastExact: true } });

test('a race at an hour ranks only states holding a value, greatest first and ties by tag, never a missing one as zero', () => {
  const entries = stateEntries([state('ITA', '500000'), state('GER', '1250000'), state('ENG', null, 'not recorded'), state('FRA', '500000')],
    5, (raw, decimals) => formatRaw(raw, decimals, 1), 50, 3);
  assert.deepEqual(entries.map(entry => [entry.tag, entry.value, entry.label, entry.hour]), [['GER', 12.5, '12.5', 50], ['FRA', 5, '5', 50], ['ITA', 5, '5', 50]]);
  assert.deepEqual(stateEntries([state('GER', '9007199254740993')], 0, raw => raw, 1, 1).map(entry => entry.label), ['9007199254740993'],
    'a raw value past a safe integer keeps its exact text');
});

test('a fixed-point raw value is written exactly, grouped, with at most the digits asked', () => {
  assert.equal(formatRaw('123456789', 5), '1,234');
  assert.equal(formatRaw('123456789', 5, 2), '1,234.56');
  assert.equal(formatRaw('-5', 5, 2), '0', 'a value showing as zero has no sign');
  assert.equal(formatRaw('-150000', 5, 1), '−1.5');
  assert.equal(formatRaw('42', 0, 3), '42');
});

test('rank and ranked chart replies that are malformed are refused', () => {
  assert.equal(validateRankReply({ branchId: '1', measure: 'free7', decimals: 0, states: [state('GER', '4')] }).states.length, 1);
  assert.throws(() => validateRankReply({ decimals: 0, states: [{ tag: 'GER', hour: 3, run: { valueRaw: 4 } }] }));
  assert.throws(() => validateRankedLines({ decimals: 0, lines: [{ tag: 'GER', membership: [{ firstHour: 5, lastHour: 2 }], missing: [], summary: null }] }));
});

test('a live race is made again only when its hour or its answer moves, and keeps its array while its rows hold', () => {
  let position = 10, version = 1, reads = 0;
  const race = liveRace({ getSnapshot: () => ({ position }) }, 100, hour => { reads++; return [{ tag: 'GER', value: hour, label: 'same', stretch: null, hour }]; },
    () => version, false);
  const first = race.entries();
  assert.equal(race.hour(), 110);
  assert.equal(race.entries(), first);
  position = 11;
  assert.equal(race.entries(), first, 'without hour-dependent rows a new hour reads nothing');
  assert.equal(reads, 1);
  version = 2;
  assert.equal(race.entries(), first, 'the same rows and labels keep the array');
  assert.equal(reads, 2);
  let woken = 0;
  const stop = race.bind(() => woken++);
  race.wake(); stop(); race.wake();
  assert.equal(woken, 1);
});

test('a race following the head names and flags its followed rows, which its asked entries are empty of', () => {
  const answer = (names: (string | null)[]) => parseFollowed(message(10, [{ id: 1, answer: { keys: names.map((_, at) => BigInt(at + 1)), columns: 1,
    values: names.map(() => 1n), states: names.map((_, at) => at === 1 ? CELL.unrecorded : CELL.value), names } }])).answers[0]!.answer;
  const top = followedRowTags(answer(['SOV', 'ENG', 'CHI']), null);
  assert.deepEqual(top, ['SOV', 'ENG', 'CHI'], 'a top N names its rows, a row without a value too');
  assert.deepEqual(followedRowTags(answer([null, null]), ['GER', 'ITA']), ['GER', 'ITA'], 'chosen countries are the tags asked, in order');
  assert.deepEqual(followedRowTags({ kind: 'notRecorded', reason: 'not here' }, null), []);
  assert.deepEqual(raceLookTags(true, top, []), ['SOV', 'ENG', 'CHI'], 'followed, the names are read for the rows painted');
  assert.equal(raceLookTags(true, null, ['GER']), undefined, 'asked, the names are read for the asked entries');
  assert.deepEqual(raceLookTags(false, top, ['GER']), ['GER'], 'the lines name their own countries');
});
