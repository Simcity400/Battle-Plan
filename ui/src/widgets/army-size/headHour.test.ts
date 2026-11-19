import assert from 'node:assert/strict';
import test from 'node:test';
import { pausedKey, heldAt } from './headHour.ts';

test('paused, a race’s key moves with every hour, so a seek keeping the same rows renders the hour sought', () => {
  assert.notEqual(pausedKey('GER,ITA', 100, false), pausedKey('GER,ITA', 4_000, false));
  assert.equal(pausedKey('GER,ITA', 100.4, false), pausedKey('GER,ITA', 100.9, false));
  assert.equal(pausedKey('GER,ITA', 100, true), pausedKey('GER,ITA', 4_000, true), 'playing, only the rows render it');
});

test('rewinding within the same structure cannot retain a future hour', () => {
  assert.deepEqual(heldAt({ key: 'same', hour: 90 }, 'same', 20), { key: 'same', hour: 20 });
});
