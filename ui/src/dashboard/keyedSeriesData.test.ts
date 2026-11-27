import assert from 'node:assert/strict';
import test from 'node:test';
import { followedRow, heldParts, keyId, keyedTileParams, keyedTileReply, validateKeyedStates, validateSeriesParts } from './keyedSeriesData.ts';
import { parseFollowed } from '../test-support/followMessage.ts';
import { message } from './follow/messages.fixture.ts';
import type { TileReply } from './tiles/tile';
import { gatedWindow } from '../test-support/gated.ts';

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

test('validateKeyedStates normalises country keys and preserves missing and notApplicable fields', () => {
  const states = validateKeyedStates({ branchId: 'b', decimals: 3, states: [
    { country: 'ENG', part: '12', hour: 30, run: run(20, 40, '5000'), drop: null, missing: null },
    { country: 'ENG', part: '7', hour: 50, run: run(45, 60, '2000'), drop: { hour: 45, beforeRaw: '3000', afterRaw: '2000' }, missing: null },
    { country: 'ENG', part: '12', hour: 70, run: null, drop: null, missing: 'The yards were not recorded' },
    { country: 'ENG', part: '7', hour: 75, run: null, drop: null, missing: 'No fuel requested', notApplicable: true },
  ] });
  assert.equal(states.states.length, 4);
  assert.equal(states.states[0]!.tag, 'ENG');
  assert.equal(states.states[2]!.missing, 'The yards were not recorded');
  assert.equal(states.states[3]!.notApplicable, true);
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
  assert.throws(() => validateSeriesParts(reply([{ ...owner, parts: [owner.parts[0], owner.parts[0]] }]), 'ENG'), /malformed/, 'a part listed twice over the same hours');
  assert.equal(validateSeriesParts(reply([{ ...owner, parts: [{ ...owner.parts[0], lastHour: 9 }, { ...owner.parts[0], firstHour: 20 }] }]), 'ENG').parts.length, 2, 'a part held over disjoint spans is listed once per span');
  assert.throws(() => validateSeriesParts(reply([{ ...owner, parts: [{ ...owner.parts[0], lastHour: -1 }] }]), 'ENG'), /malformed/);
  assert.throws(() => validateSeriesParts({ ...reply([owner]), partBits: 0 }, 'ENG'), /malformed/, 'no part bits');
});

test('a part held across disjoint runs is listed once, with each run kept apart', () => {
  const held = heldParts([{ part: '7', id: null, firstHour: 0, lastHour: 9 }, { part: '3', id: 'tank', firstHour: 5, lastHour: 6 }, { part: '7', id: 'ship', firstHour: 20, lastHour: 29 },
    { part: '7', id: null, firstHour: 10, lastHour: 12 }]);
  assert.deepEqual(held, [{ part: '7', id: 'ship', firstHour: 0, lastHour: 29, spans: [{ firstHour: 0, lastHour: 12 }, { firstHour: 20, lastHour: 29 }] },
    { part: '3', id: 'tank', firstHour: 5, lastHour: 6, spans: [{ firstHour: 5, lastHour: 6 }] }]);
});

const bound = (first: number, last: number) => gatedWindow(first, last);

test('tile lines name each key, and the lines of a tag share the spans they are drawn over', () => {
  assert.deepEqual(keyedTileParams(keys, undefined, bound), { lines: keys.map(({ tag, part }) => ({ country: tag, part })) });
  const spans = [{ firstHour: 0, lastHour: 9 }];
  const within = { [keyId(keys[0])]: spans, [keyId(keys[1])]: spans };
  assert.deepEqual(keyedTileParams(keys, within, bound), { lines: keys.map(({ tag, part }) => ({ country: tag, part })), within: [['ENG', spans]] });
  assert.throws(() => keyedTileParams(keys, { [keyId(keys[0])]: spans }, bound), /one set of spans/, 'one key of a tag drawn whole beside another clipped');
});

test('the followed parts of an owner are found by tag and the part in the low bits of the row key', () => {
  const key = (owner: bigint, part: bigint) => (owner << 32n) | part;
  const answer = parseFollowed(message(10, [{ id: 1, answer: { keys: [key(5n, 7n), key(5n, 12n), key(5n, 7n), 0n], columns: 1,
    values: [70n, 120n, 1n, 0n], names: ['ENG', 'ENG', 'GER', 'FRA'], reasons: [null, null, null, 'FRA holds no row'] } }])).answers[0]!.answer;
  assert.equal(followedRow(answer, keys[0], 32), 0);
  assert.equal(followedRow(answer, keys[1], 32), 1);
  assert.equal(followedRow(answer, { tag: 'GER', part: '7' }, 32), 2);
  assert.equal(followedRow(answer, { tag: 'ENG', part: '3' }, 32), -1, 'a part the head holds no row of');
  assert.equal(followedRow(answer, { tag: 'FRA', part: '0' }, 32), -1, 'an owner holding no row is never its part 0');
  assert.equal(followedRow(undefined, keys[0], 32), -1);

  const many = Array.from({ length: 1500 }, (_, at) => ({ tag: `C${at}`, part: '1' }));
  const wide = parseFollowed(message(10, [{ id: 1, answer: { keys: many.map((_, at) => key(BigInt(at + 1), 1n)), columns: 1,
    values: many.map(() => 1n), names: many.map(item => item.tag) } }])).answers[0]!.answer as { rows: number };
  const rows = wide.rows;
  let read = 0;
  Object.defineProperty(wide, 'rows', { get: () => { read++; return rows; } });
  for (const [at, item] of many.entries()) assert.equal(followedRow(wide as never, item, 32), at);
  assert.ok(read <= 3 * rows, `the answer is indexed once for every key, never scanned per key (${read} row reads for ${rows} rows)`);
});
