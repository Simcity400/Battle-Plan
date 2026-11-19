import assert from 'node:assert/strict';
import test from 'node:test';
import { liveKey } from '../../charts/live.ts';
import { ZERO_DIVISOR, liveSection, repliedCells } from './liveCells.ts';
import type { LiveSection } from './liveCells.ts';

/** Two countries' power (group 0: Countries, column 2, keys 100 and 101) and one's coal (group 1: two columns of key 7). */
const section = (groups: LiveSection['groups']): LiveSection => ({
  groups,
  rows: ['100', '101'],
  kinds: [{ id: 'power', op: 'value', scale: 0.001 }, { id: 'coal', op: 'sum', scale: 1 }, { id: 'net', op: 'difference', scale: 1 }, { id: 'share', op: 'ratio', scale: 100 }],
  cells: [
    0, 0, 1, 0, 0, 0,
    1, 0, 1, 0, 1, 0,
    0, 1, 2, 1, 0, 0, 1, 0, 1,
    0, 2, 2, 1, 0, 0, 1, 0, 1,
    0, 3, 2, 0, 0, 0, 0, 1, 0,
  ],
});
const countries = { table: 2, columns: [2], keys: ['100', '101'] };
const coal = { table: 52, columns: [4, 5], keys: ['7'] };

test('a request\'s reply gives each cell its value by its formula, and a missing one its reason, never zero', () => {
  const live = repliedCells({ live: section([{ ...countries, values: [5000, 'No live row has this key'] }, { ...coal, values: [900, -100] }]) });
  const values = live.read();
  assert.equal(values.get(liveKey('100', 'power')), 5);
  assert.equal(values.get(liveKey('101', 'power')), undefined);
  assert.equal(values.missing(liveKey('101', 'power')), 'No live row has this key');
  assert.equal(values.get(liveKey('100', 'coal')), 800, 'a sum');
  assert.equal(values.get(liveKey('100', 'net')), 1000, 'a difference');
  assert.equal(values.get(liveKey('100', 'share')), undefined, 'a ratio over a missing divisor');
  assert.equal(values.missing(liveKey('100', 'share')), 'No live row has this key');
  assert.equal(values.missing(liveKey('100', 'nothing')), 'Not a live cell of this reply');
  const zero = repliedCells({ live: section([{ ...countries, values: [5000, 0] }, { ...coal, values: [1, 1] }]) }).read();
  assert.equal(zero.missing(liveKey('100', 'share')), ZERO_DIVISOR);
  assert.equal(repliedCells({ live: section([{ ...countries, values: [5000, 0] }, { ...coal, unavailable: 'CountryResources is not in this playhead\'s files' }]) }).read()
    .missing(liveKey('100', 'coal')), 'CountryResources is not in this playhead\'s files');
  assert.equal(liveSection({ warnings: [] }), null, 'a reply naming no cells');
  assert.throws(() => liveSection({ live: { ...section([{ ...countries, values: [1, 2] }]), cells: [0, 0, 1, 5, 0, 0] } }), /source no group holds/);
  assert.throws(() => liveSection({ live: section([{ ...countries, follow: 7 }, { ...coal, values: [1, 1] }]) }), /malformed/, 'a UI id is never a group\'s');
});

test('a reply naming no cells keeps the same reading', () => {
  const none = repliedCells({ warnings: [] });
  assert.equal(none.read(), none.read(), 'a reply naming no cells');
});
