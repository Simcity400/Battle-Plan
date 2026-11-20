import assert from 'node:assert/strict';
import test from 'node:test';
import { followedRow, keyId, keyedTileParams, keyedTileReply, reachedKeyLines, validateKeyedStates, validateSeriesParts } from './keyedSeriesData.ts';
import { parseFollowed } from './follow/message.ts';
import { message } from './follow/messages.fixture.ts';
import type { TileReply } from './tiles/tile';

const run = (firstHour: number, lastHour: number, valueRaw: string) => ({ firstHour, lastHour, valueRaw, firstExact: true, lastExact: false,
  startsSegment: false, slot: 1, countryId: 1, segmentFirstHour: firstHour });
const keys = [{ tag: 'ENG', part: '7' }, { tag: 'ENG', part: '12' }];

test('a tile reply for parts names each line by its key, and one for other keys is refused', () => {
  const reply = { binHours: 6, index: 0, firstHour: 0, immutable: true, filled: null, decimals: 3, rawValues: true, hourBytes: 1, bins: '',
    lines: keys.map(key => ({ country: key.tag, part: key.part, missing: [], parts: [] })) } as unknown as TileReply & { lines: { country: string; part: string }[] };
  assert.deepEqual(keyedTileReply(reply, keys).lines.map(line => line.country), keys.map(keyId));
  assert.throws(() => keyedTileReply(reply, [...keys].reverse()), /malformed/, 'lines in another order');
  assert.throws(() => keyedTileReply(reply, [{ tag: 'ENG', part: '7' }]), /malformed/, 'another number of lines');
  assert.notEqual(keyId({ tag: 'A', part: 'BC' }), keyId({ tag: 'AB', part: 'C' }), 'a key id never runs owner and part together');
});

test('reached lines keep each key apart, its runs once each in hour order, its drops and why an hour has no value', () => {
  const states = validateKeyedStates({ branchId: 'b', decimals: 3, states: [
    { country: 'ENG', part: '12', hour: 30, run: run(20, 40, '5000'), drop: null, missing: null },
    { country: 'ENG', part: '7', hour: 50, run: run(45, 60, '2000'), drop: { hour: 45, beforeRaw: '3000', afterRaw: '2000' }, missing: null },
    { country: 'ENG', part: '7', hour: 10, run: run(0, 20, '3000'), drop: null, missing: null },
    { country: 'ENG', part: '7', hour: 55, run: run(45, 60, '2000'), drop: null, missing: null },
    { country: 'ENG', part: '12', hour: 70, run: null, drop: null, missing: 'The yards were not recorded' },
  ] });
  const [seven, twelve] = reachedKeyLines(keys, states);
  assert.deepEqual(seven.runs.map(item => [item.firstHour, item.valueRaw]), [[0, '3000'], [45, '2000']]);
  assert.deepEqual(seven.drops.map(item => item.hour), [45]);
  assert.deepEqual([...twelve.missing], [[70, 'The yards were not recorded']], 'a missing value is its reason, never zero');
  assert.deepEqual(reachedKeyLines(keys, undefined).map(line => line.runs.length), [0, 0]);
});

test('states without a part and parts lists that repeat a part or name another owner are refused', () => {
  assert.throws(() => validateKeyedStates({ branchId: 'b', decimals: 3, states: [{ country: 'ENG', hour: 1, run: null, drop: null, missing: null }] }), /malformed/);
  const owner = { country: 'ENG', parts: [{ part: '7', id: 'battleship', firstHour: 0, lastHour: 90 }, { part: '12', firstHour: 40, lastHour: 50 }],
    missing: [{ firstHour: 91, lastHour: 99, reason: 'No country holds ENG' }] };
  const reply = (owners: unknown[]) => ({ branchId: 'b', partBits: 32, owners });
  assert.deepEqual(validateSeriesParts(reply([owner]), 'ENG'), { tag: 'ENG', partBits: 32, missing: owner.missing,
    parts: [owner.parts[0], { ...owner.parts[1], id: null }] }, 'a part without a content id has none');
  assert.throws(() => validateSeriesParts(reply([owner]), 'GER'), /malformed/);
  assert.throws(() => validateSeriesParts(reply([owner, owner]), 'ENG'), /malformed/, 'another owner beside it');
  assert.throws(() => validateSeriesParts(reply([{ ...owner, parts: [owner.parts[0], owner.parts[0]] }]), 'ENG'), /malformed/);
  assert.throws(() => validateSeriesParts(reply([{ ...owner, parts: [{ ...owner.parts[0], lastHour: -1 }] }]), 'ENG'), /malformed/);
  assert.throws(() => validateSeriesParts({ ...reply([owner]), partBits: 0 }, 'ENG'), /malformed/, 'no part bits');
});

test('tile lines name each key, and the lines of a tag share the spans they are drawn over', () => {
  assert.deepEqual(keyedTileParams(keys, undefined), { lines: keys.map(({ tag, part }) => ({ country: tag, part })) });
  const spans = [{ firstHour: 0, lastHour: 9 }];
  const within = { [keyId(keys[0])]: spans, [keyId(keys[1])]: spans };
  assert.deepEqual(keyedTileParams(keys, within), { lines: keys.map(({ tag, part }) => ({ country: tag, part })), within: [['ENG', spans]] });
  assert.throws(() => keyedTileParams(keys, { [keyId(keys[0])]: spans }), /one set of spans/, 'one key of a tag drawn whole beside another clipped');
});

test('the followed parts of an owner are found by tag and the part in the low bits of the row key', () => {
  // ENG's owner key 5 holds parts 7 and 12, GER's owner key 5 too (another series row); GER at 6 holds no rows.
  const key = (owner: bigint, part: bigint) => (owner << 32n) | part;
  const answer = parseFollowed(message(10, [{ id: 1, answer: { keys: [key(5n, 7n), key(5n, 12n), key(5n, 7n), 0n], columns: 1,
    values: [70n, 120n, 1n, 0n], names: ['ENG', 'ENG', 'GER', 'FRA'], reasons: [null, null, null, 'FRA holds no row'] } }])).answers[0]!.answer;
  assert.equal(followedRow(answer, keys[0], 32), 0);
  assert.equal(followedRow(answer, keys[1], 32), 1);
  assert.equal(followedRow(answer, { tag: 'GER', part: '7' }, 32), 2);
  assert.equal(followedRow(answer, { tag: 'ENG', part: '3' }, 32), -1, 'a part the head holds no row of');
  assert.equal(followedRow(answer, { tag: 'FRA', part: '0' }, 32), -1, 'an owner holding no row is never its part 0');
  assert.equal(followedRow(undefined, keys[0], 32), -1);
});
