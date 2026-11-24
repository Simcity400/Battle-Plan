import assert from 'node:assert/strict';
import test from 'node:test';
import { CELL, parseFollowed } from '../../dashboard/follow/message.ts';
import { message } from '../../dashboard/follow/messages.fixture.ts';
import { followedEntries, rowsOf } from './liveArmy.ts';

const answer = (rows: [string, bigint, number?][]) => parseFollowed(message(10, [{ id: 1, answer: { keys: rows.map((_, at) => BigInt(at + 1)), columns: 1,
  values: rows.map(([, value]) => value), states: rows.map(([, , state]) => state ?? CELL.value), names: rows.map(([tag]) => tag) } }])).answers[0]!.answer;

test('a followed answer is ranked as today ranks a reply: greatest first, ties by tag, unrecorded rows and non-members left out', () => {
  const rows = answer([['SOV', 5n], ['ENG', 7n], ['GER', 7n], ['ITA', 9n, CELL.unrecorded], ['FRA', 3n]]);
  assert.deepEqual(followedEntries(rows, null, { membership: null }, 10, 3, 'divisions', 0).map(entry => [entry.tag, entry.value, entry.label, entry.hour]),
    [['ENG', 7, '7', 10], ['GER', 7, '7', 10], ['SOV', 5, '5', 10]]);
  const members = new Map([['GER', [{ firstHour: 0, lastHour: 20 }]], ['SOV', [{ firstHour: 30, lastHour: 40 }]], ['FRA', [{ firstHour: 0, lastHour: 99 }]],
    ['ENG', [{ firstHour: 50, lastHour: 60 }]]]);
  assert.deepEqual(followedEntries(rows, null, { membership: members as never }, 10, 5, 'divisions', 0).map(entry => entry.tag), ['GER', 'FRA']);
  assert.deepEqual(followedEntries(answer([['GER', 123_456_789n]]), null, { membership: null }, 10, 1, 'strength', 5).map(entry => [entry.value, entry.label]),
    [[1234.56789, '1,234.56789']]);
  assert.deepEqual(followedEntries({ kind: 'notRecorded', reason: 'not here' }, null, { membership: null }, 10, 3, 'divisions', 0), []);
  assert.equal(rowsOf(followedEntries(rows, null, { membership: null }, 10, 3, 'divisions', 0)), 'ENG,GER,SOV');
});

