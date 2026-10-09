import assert from 'node:assert/strict';
import test from 'node:test';
import { logCountAt, rememberLogCounts } from './logCounts.ts';

test('a live zero or partial count expires on advance and rewind, including a late native witness', () => {
  const held = rememberLogCounts(new Map(), [{ key: 'opaque | page', count: 0, completeBy: 40 }], 20);
  assert.equal(logCountAt(held.get('opaque | page'), 20, 5000), 0);
  assert.equal(logCountAt(held.get('opaque | page'), 21, 5000), 5000);
  assert.equal(logCountAt(held.get('opaque | page'), 19, 5000), 5000);
  assert.equal(logCountAt(held.get('opaque | page'), 40, 5000), 5000);
  const complete = rememberLogCounts(held, [{ key: 'opaque | page', count: 7, completeBy: 40 }], 40);
  assert.equal(logCountAt(complete.get('opaque | page'), 100, 5000), 7);
  assert.equal(logCountAt(complete.get('opaque | page'), 39, 5000), 5000);
});

test('stable fully witnessed counts preserve references and known empty filtered pages', () => {
  const samples = [{ key: 'a', count: 0, completeBy: 40 }, { key: 'b', count: 5000, completeBy: 30 }];
  const held = rememberLogCounts(new Map(), samples, 40);
  assert.equal(rememberLogCounts(held, samples, 41), held);
  assert.equal(logCountAt(held.get('a'), 41, 5000), 0);
  assert.equal(logCountAt(held.get('b'), null, 9000), 9000);
});
