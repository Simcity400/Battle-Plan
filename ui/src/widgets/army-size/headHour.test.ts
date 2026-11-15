import assert from 'node:assert/strict';
import test from 'node:test';
import { paced, pausedKey } from './headHour.ts';
import { inStructureSlot } from '../../dashboard/structurePace.ts';

test('a structural key renders at once paused, and while playing only in the slot’s task however fast it churns', () => {
  let held = paced(null, 'GER,ITA', 100, true);
  assert.deepEqual(held, { key: 'GER,ITA', hour: 100 }, 'the first key is taken at once');
  assert.equal(paced(held, 'GER,ITA', 115, true), held, 'the same key, the same object: no render');
  let renders = 0;
  // Rows churning every frame (15 hours a frame) between two slots.
  for (let frame = 1; frame <= 14; frame++) {
    const next = paced(held, frame % 2 ? 'GER,SOV' : 'GER,ITA', 100 + frame * 15, true);
    if (next !== held) renders++;
    held = next;
  }
  assert.equal(renders, 0, 'playing, nothing is taken between slots');
  inStructureSlot(() => { held = paced(held, 'GER,SOV', 400, true); });
  assert.deepEqual(held, { key: 'GER,SOV', hour: 400 }, 'the slot’s task takes the newest key');
  const paused = paced(held, 'ENG', 2000, false);
  assert.equal(paused.key, 'ENG', 'paused, a change renders at once');
  assert.equal(paused.hour, 2000, 'at the hour it changed');
});

test('paused, a race’s key moves with every hour, so a seek keeping the same rows renders the hour sought', () => {
  assert.notEqual(pausedKey('GER,ITA', 100, false), pausedKey('GER,ITA', 4_000, false));
  assert.equal(pausedKey('GER,ITA', 100.4, false), pausedKey('GER,ITA', 100.9, false));
  assert.equal(pausedKey('GER,ITA', 100, true), pausedKey('GER,ITA', 4_000, true), 'playing, only the rows render it');
});
