import { answerShape } from './message.ts';
import { answerBody } from './messages.fixture.ts';
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { CELL, cellState, cellValue, rowKey, rowName, rowReason } from './message.ts';
import { parseDatedPanels, parseFollowed } from '../../test-support/followMessage.ts';
import { datedPanel, datedRecord, message } from './messages.fixture.ts';

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
});

test('a stopped panel is read as a failure with its reason, and a refused one stays a refusal by the recording', () => {
  assert.deepEqual(parseDatedPanels(datedRecord(4, 31, 7, 'the recording file was closed')).panels[0]!.panel, { refused: null, stopped: 'the recording file was closed' });
  assert.deepEqual(parseDatedPanels(datedRecord(4, 31, 0, 'Countries was not recorded at this hour')).panels[0]!.panel, { refused: 'Countries was not recorded at this hour' });
});

test('review 10: an answer\'s shape names its rows and columns, not their order', () => {
  const a = answerBody({ keys: [1n, 2n, 3n], columns: 1, values: [10n, 20n, 30n] });
  const swapped = answerBody({ keys: [2n, 1n, 3n], columns: 1, values: [20n, 10n, 30n] });
  const other = answerBody({ keys: [1n, 2n, 4n], columns: 1, values: [10n, 20n, 40n] });
  assert.equal(answerShape(a), answerShape(swapped), 'a rank swap with the same members commits nothing');
  assert.notEqual(answerShape(a), answerShape(other), 'a new member is a new shape');
});
