import { answerShape } from './message.ts';
import { answerBody } from './messages.fixture.ts';
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { decode, encode, ExtData } from '@msgpack/msgpack';
import { ReplyDecoder } from '../../draw/replyDecoder.ts';
import { CELL, cellState, cellValue, rowKey, rowName, rowReason } from './message.ts';
import { parseDatedPanels, parseFollowed } from '../../test-support/followMessage.ts';
import { datedPanel, message } from './messages.fixture.ts';

const GOLDEN = '464f4c3301000000d204000003000000000000000000000007000000b0040000d000000000000000010000000300000001000000050000006400000000000000650000000000000000000000000000000000000000000000000000000000000000000000000000004e61bc0000000000000000000000000000000000000000000001030000000000ffffffff02000000040000000000000000000000010000000300000000000000000000000300000006000000160000001900000032000000474552534f56534f5620686173206e6f2076616c75654954414954412069732068656c64206279206e6f20636f756e74727900000000000008000000d20400004000000000000000000000000000000000000000010000000000000027000000436f756e747269657320776173206e6f74207265636f72646564206174207468697320686f75720009000000d204000058000000000000000200000001000000020000000000000005000000000000004000000000000000feffffffffffffff09000000000000004c040000000000000000000000000000ffffffff00000000ffffffff000000000000000000000000';

test('the bytes the service writes read back as it wrote them', () => {
  const bytes = new Uint8Array(GOLDEN.match(/../g)!.map(pair => parseInt(pair, 16)));
  const parsed = parseFollowed(bytes);
  assert.equal(parsed.hour, 1234);
  assert.equal(parsed.reset, true);
  assert.deepEqual(parsed.answers.map(({ id, hour }) => [id, hour]), [[7, 1200], [8, 1234], [9, 1234]]);
  const [series, unrecorded, log] = parsed.answers.map(({ answer }) => answer);
  assert.ok(series!.kind === 'rows');
  assert.equal(series.canonical, false, 'the absent row of key 0 comes after 101: not in key order');
  assert.deepEqual([0, 1, 2].map(row => rowName(series, row)), ['GER', 'SOV', 'ITA']);
  assert.equal(cellValue(series, 0), 12_345_678n);
  assert.deepEqual([0, 1, 2].map(at => cellState(series, at)), [CELL.value, CELL.missing, CELL.noRow]);
  assert.deepEqual([0, 1, 2].map(row => rowReason(series, row)), [null, 'SOV has no value', 'ITA is held by no country']);
  assert.deepEqual(unrecorded, { kind: 'notRecorded', reason: 'Countries was not recorded at this hour' });
  assert.ok(log!.kind === 'log');
  assert.equal(rowKey(log, 0), (1n << 70n) + 5n);
  assert.deepEqual([cellValue(log, 0), cellValue(log, 1)], [-2n, 9n]);
});

test('seventy thousand texts are each read back by their own row, in any order', () => {
  const rows = 70_000;
  const keys = Array.from({ length: rows }, (_, row) => BigInt(row));
  const reasons = keys.map((_, row) => row % 2 === 0 ? `reason ${row}` : null), names = keys.map((_, row) => row % 2 === 0 ? null : `name ${row}`);
  const [{ answer }] = parseFollowed(message(1, [{ id: 1, answer: { keys, columns: 1, values: keys.map(() => 0n), states: keys.map(() => CELL.missing), reasons, names } }])).answers;
  assert.ok(answer!.kind === 'rows');
  assert.equal(answer.canonical, true);
  for (let row = rows - 1; row >= 0; row -= 7) {
    if (rowReason(answer, row) !== reasons[row] || rowName(answer, row) !== names[row]) assert.fail(`row ${row}: ${rowReason(answer, row)}, ${rowName(answer, row)}`);
  }
});

test('a dated panel reads the native MessagePack bytes', () => {
  const bytes = datedPanel(1, 7, { rows: [7] });
  assert.deepEqual([...bytes.slice(40, 48)], [0x81, 0xa4, 114, 111, 119, 115, 0x91, 7]);
  assert.deepEqual(parseDatedPanels(bytes).panels[0]!.panel?.data, { rows: [7] });
  const body = bytes.subarray(40, 48);
  assert.deepEqual(new ReplyDecoder().decode(body), decode(body));
});

test('lane reply strings preserve MessagePack values, binary, extensions and noncanonical string headers', () => {
  const reader = new ReplyDecoder();
  const samples = [
    encode([null, false, 0, 1.25, { nested: ['é漢', '🟩', '\uFEFFtag', '', 'x'.repeat(256)] }]),
    encode([new Uint8Array([1, 2, 3]), new Date(100), new ExtData(12, new Uint8Array([0xa3, 2, 3]))]),
    encode(['yrljbjuz', 'jsldzojd', 'yrljbjuz']),
    ...[0xd9, 0xda, 0xdb, 0xc4, 0xc5, 0xc6].map((tag, i) =>
      Uint8Array.of(tag, ...Array((1 << (i % 3)) - 1).fill(0), 3, 0xc0, 0xaf, 0xff)),
    Uint8Array.of(0x92, 0xa2, 0xc3, 0xa9, 0xa1, 0xc3),
    Uint8Array.of(0x92, 0xa1, 0xc3, 1),
    Uint8Array.of(0x92, 0xa1, 0xc3, 2),
  ];
  for (const body of samples) {
    const padded = new Uint8Array(7 + body.length);
    padded.set(body, 7);
    assert.deepEqual(reader.decode(padded.subarray(7)), decode(body));
  }
});

test('1,000 decodes build each distinct short string once per lane', t => {
  const reader = new ReplyDecoder();
  const bytes = encode({ tag: 'ABC', values: ['ABC', '1245', 'counted', 'tag'] });
  const built = t.mock.method(String, 'fromCharCode');
  const first = reader.decode(bytes);
  for (let i = 1; i < 1000; i++) assert.deepEqual(reader.decode(bytes), first);
  assert.equal(built.mock.callCount(), 5);
  assert.deepEqual(new ReplyDecoder().decode(bytes), first);
  assert.equal(built.mock.callCount(), 10);
  for (const [i, tag] of [0xd9, 0xda, 0xdb].entries()) {
    assert.equal(reader.decode(Uint8Array.of(tag, ...Array((1 << i) - 1).fill(0), 3, 65, 66, 67)), 'ABC');
  }
  assert.equal(built.mock.callCount(), 10);
  const long = encode('x'.repeat(32));
  reader.decode(long);
  reader.decode(long);
  assert.equal(built.mock.callCount(), 12);
});

test('the lane interns string values only while a current request uses them', t => {
  const reader = new ReplyDecoder();
  const built = t.mock.method(String, 'fromCharCode');
  const values = encode(['yrljbjuz', 'jsldzojd', 'jsldzojd']);
  assert.deepEqual(reader.decode(values, 1), ['yrljbjuz', 'jsldzojd', 'jsldzojd']);
  assert.equal(built.mock.callCount(), 2, 'the second occurrence of a value is served by the lane table');
  reader.decode(encode('jsldzojd'), 2);
  reader.decode(encode(0), 1);
  assert.equal(built.mock.callCount(), 2, 'a second request still uses the colliding value');
  reader.decode(values, 1);
  assert.equal(built.mock.callCount(), 3, 'only the retired collision entry was rebuilt');
  reader.forget(1);
  reader.forget(2);
  assert.throws(() => reader.decode(Uint8Array.of(0x92, 0xa1, 65), 1));
  assert.equal(reader.decode(encode('A'), 1), 'A');
  assert.equal(built.mock.callCount(), 5, 'a partial reply does not keep its strings');
  reader.decode(values, 2);
  assert.equal(built.mock.callCount(), 7, 'closing the last request released its strings');
});

test('review 10: an answer\'s shape names its rows and columns, not their order', () => {
  const a = answerBody({ keys: [1n, 2n, 3n], columns: 1, values: [10n, 20n, 30n] });
  const swapped = answerBody({ keys: [2n, 1n, 3n], columns: 1, values: [20n, 10n, 30n] });
  const other = answerBody({ keys: [1n, 2n, 4n], columns: 1, values: [10n, 20n, 40n] });
  assert.equal(answerShape(a), answerShape(swapped), 'a rank swap with the same members commits nothing');
  assert.notEqual(answerShape(a), answerShape(other), 'a new member is a new shape');
});
