import assert from 'node:assert/strict';
import test from 'node:test';
import { logStatesBy } from './logStates.ts';
import type { LogRow } from './logData';

interface Item { name: string; status: string }
const order = (a: LogRow<Item>, b: LogRow<Item>) => b.hour - a.hour || (a.item.name < b.item.name ? -1 : a.item.name > b.item.name ? 1 : 0);
const row = (name: string, hour: number, status: string): LogRow<Item> => ({ key: name, id: name, hour, item: { name, status } });
const rowsAt = (hour: number) => [
  hour < 110 ? row('a', 90, 'under way') : row('a', 110, 'ended'),
  ...hour >= 105 ? [row('b', 105, 'under way')] : [],
  ...hour < 108 ? [row('c', 95, 'under way')] : [],
  row('d', 80, hour >= 103 && hour < 106 ? 'y' : 'x'),
].sort(order);
const state = (shown: LogRow<Item>) => shown.item.status;
const shownAt = (rows: readonly LogRow<Item>[], hour: number) =>
  rows.filter(item => Math.max(item.hour, item.revealedHour ?? item.hour) <= hour && (item.leftHour ?? Infinity) > hour);

test('a block holds each row\'s states, and at every hour shows exactly that hour\'s rows in its order', () => {
  const rows = logStatesBy([100, 103, 105, 106, 108, 110], rowsAt, () => null, () => null, state, order);
  for (let hour = 100; hour <= 120; hour++) {
    assert.deepEqual(shownAt(rows, hour).map(item => [item.item.name, item.item.status]), rowsAt(hour).map(item => [item.item.name, item.item.status]), `at ${hour}`);
  }
  assert.equal(new Set(rows.map(item => item.key)).size, rows.length, 'a state that comes back has a key of its own');
  assert.ok(rows.every(item => item.id === item.item.name), 'every state carries its row\'s id, so the list draws them as one row');
  const opened = rows.filter(item => item.revealedHour === undefined).map(item => item.item.name).sort();
  assert.deepEqual(opened, ['a', 'c', 'd'], 'the first hour\'s rows show from their own reveal');
});

test('a state keeps its key in the next block, so it does not enter again', () => {
  const first = logStatesBy([100, 103, 105, 106, 108, 110], rowsAt, () => null, () => null, state, order);
  const next = logStatesBy([110], rowsAt, () => null, () => null, state, order);
  const keyOf = (rows: readonly LogRow<Item>[], name: string, status: string) => rows.filter(item => item.item.name === name && item.item.status === status).map(item => item.key);
  assert.deepEqual(keyOf(next, 'a', 'ended'), keyOf(first, 'a', 'ended'));
  assert.deepEqual(keyOf(next, 'b', 'under way'), keyOf(first, 'b', 'under way'));
});

test('a block read whole only at its first hour, then only the rows that change, holds the same states as one read whole at every hour', () => {
  const hours = [100, 103, 105, 106, 108, 110];
  const changing = new Map([[103, ['d']], [105, ['b']], [106, ['d']], [108, ['c']], [110, ['a']]]);
  const reads: string[] = [];
  const rowOf = (name: string, hour: number) => { reads.push(`${name}@${hour}`); return rowsAt(hour).find(item => item.item.name === name) ?? null; };
  const whole = (hour: number) => { reads.push(`*@${hour}`); return rowsAt(hour); };
  const by = logStatesBy(hours, whole, hour => changing.get(hour) ?? [], rowOf, state, order);
  const all = logStatesBy(hours, rowsAt, () => null, () => null, state, order);
  const shape = (rows: readonly LogRow<Item>[]) => rows.map(item => [item.key, item.id, item.revealedHour ?? null, item.leftHour ?? null]);
  assert.deepEqual(shape(by), shape(all));
  assert.deepEqual(reads, ['*@100', 'd@103', 'b@105', 'd@106', 'c@108', 'a@110'], 'the first hour whole, then one read per change');
  reads.length = 0;
  assert.deepEqual(shape(logStatesBy(hours, whole, hour => hour === 106 ? null : changing.get(hour) ?? [], rowOf, state, order)), shape(all));
  assert.ok(reads.includes('*@106') && !reads.includes('d@106'));
});
