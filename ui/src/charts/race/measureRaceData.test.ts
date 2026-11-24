import assert from 'node:assert/strict';
import test from 'node:test';
import { followedRace, followedRowTags, formatRaw, liveRace, raceLookTags, rankEntries, rankProvenInterval, validateRankedLines, validateRankReply } from './measureRaceData.ts';
import { CELL } from '../../dashboard/follow/message.ts';
import { parseFollowed } from '../../test-support/followMessage.ts';
import { message } from '../../dashboard/follow/messages.fixture.ts';
import type { SeriesRankReply, SeriesState } from '../../generated/dashboard';

const state = (tag: string, valueRaw: string | null, missing: string | null = null): SeriesState => ({ country: tag, hour: 50, drop: null, missing,
  run: valueRaw === null ? null : { firstHour: 40, lastHour: 60, valueRaw, slot: 1, countryId: 1, startsSegment: false, segmentFirstHour: 0, firstExact: true, lastExact: true } });

const rank = (states: readonly SeriesState[], decimals: number, from = 50, until = 50, refused: string | null = null): SeriesRankReply => ({
  branchId: '1', hour: 50, from, until, refused, measures: [{ name: 'free7', decimals }], rows: states.map(held => ({ country: held.country, slot: null, readings: [held] })),
});

test('a race at an hour keeps the product order, drawing only rows holding a value, never a missing one as zero', () => {
  const entries = rankEntries(rank([state('GER', '1250000'), state('ITA', '500000'), state('FRA', '500000'), state('ENG', null, 'not recorded')], 5),
    (raw, decimals) => formatRaw(raw, decimals, 1), 3);
  assert.deepEqual(entries.map(entry => [entry.tag, entry.value, entry.label, entry.hour]), [['GER', 12.5, '12.5', 50], ['ITA', 5, '5', 50], ['FRA', 5, '5', 50]]);
  assert.deepEqual(rankEntries(rank([state('GER', '9007199254740993')], 0), raw => raw, 1).map(entry => entry.label), ['9007199254740993'],
    'a raw value past a safe integer keeps its exact text');
});

test('a rank holds only the hours its reply proves, never its winners\' value runs', () => {
  assert.deepEqual(rankProvenInterval(rank([state('GER', '4')], 0), 50), [50, 50], 'the winner runs 40..60, but an omitted country may overtake at 51');
  assert.deepEqual(rankProvenInterval(rank([], 0, 50, 50, 'Countries not recorded at this hour'), 50), [50, 50], 'a refused top holds its hour only');
  assert.deepEqual(rankProvenInterval(rank([state('GER', '4')], 0, 45, 58), 50), [45, 58]);
});

test('a fixed-point raw value is written exactly, grouped, with at most the digits asked', () => {
  assert.equal(formatRaw('123456789', 5), '1,234');
  assert.equal(formatRaw('123456789', 5, 2), '1,234.56');
  assert.equal(formatRaw('-5', 5, 2), '0', 'a value showing as zero has no sign');
  assert.equal(formatRaw('-150000', 5, 1), '−1.5');
  assert.equal(formatRaw('42', 0, 3), '42');
});

test('rank and ranked chart replies that are malformed are refused', () => {
  assert.equal(validateRankReply(rank([state('GER', '4')], 0)).rows.length, 1);
  assert.throws(() => validateRankReply({ decimals: 0, states: [{ tag: 'GER', hour: 3, run: { valueRaw: 4 } }] }));
  assert.throws(() => validateRankReply({ ...rank([state('GER', '4')], 0), from: 51 }), 'a reply must hold its own hour');
  assert.throws(() => validateRankedLines({ decimals: 0, lines: [{ tag: 'GER', membership: [{ firstHour: 5, lastHour: 2 }], missing: [], summary: null }] }));
});

test('a live race is made again only when its hour or its answer moves, and keeps its array while its rows hold', () => {
  let position = 10, version = 1, reads = 0;
  const race = liveRace({ getSnapshot: () => ({ position }) }, 100, hour => { reads++; return [{ tag: 'GER', value: 1, label: 'same', stretch: null, hour }]; },
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

test('review 5 finding 13: a held race names the hour of the reading it shows, never the clock\'s', () => {
  const entry = { answer: undefined, through: 40 };
  const race = liveRace({ getSnapshot: () => ({ position: 90 }) }, 0, hour => followedRace(entry, null, null, hour, 5, 0, raw => raw).entries, () => 1, false,
    hour => followedRace(entry, null, null, hour, 0, 0, raw => raw).shown);
  assert.equal(race.hour(), 40, 'the overlay, label, tooltip and row action date the held reading');
});

test('review 6 B: a live race shows new split values even when the tags and totals read the same', () => {
  let version = 1, parts = [{ id: 'a', value: 1 }, { id: 'b', value: 2 }];
  const race = liveRace({ getSnapshot: () => ({ position: 10 }) }, 0, hour => [{ tag: 'GER', value: 3, label: '3', stretch: null, hour, parts }], () => version, false);
  const first = race.entries();
  version = 2; parts = [{ id: 'a', value: 2 }, { id: 'b', value: 1 }];
  assert.notEqual(race.entries(), first, 'the changed split is a new picture');
});
