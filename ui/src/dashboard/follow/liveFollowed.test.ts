import assert from 'node:assert/strict';
import test from 'node:test';
import { answerValues, liveFollowed } from './liveFollowed.ts';
import { CELL, parseFollowed } from './message.ts';
import { message } from './messages.fixture.ts';
import type { FollowEntry } from './store';

const answer = (spec: Parameters<typeof message>[1][number]['answer']) => parseFollowed(message(10, [{ id: 1, answer: spec }])).answers[0]!.answer;

test('a followed answer gives each asked key its value, and a key without one is missing for its reason, never zero', () => {
  const rows = answer({ keys: [1n, 2n, 3n], columns: 1, values: [12_50n, 0n, 0n], states: [CELL.value, CELL.unrecorded, CELL.value], reasons: [null, 'the read failed', null] });
  const values = answerValues(rows, ['GER', 'SOV', 'ITA'], 2);
  assert.equal(values.get('GER'), 12.5);
  assert.equal(values.get('SOV'), undefined);
  assert.equal(values.missing('SOV'), 'the read failed');
  assert.equal(values.get('ITA'), 0, 'a recorded zero is a value');
  assert.equal(values.get('JAP'), undefined);
  assert.equal(values.missing('JAP'), 'Not in the answer');
  assert.equal(answerValues(undefined, ['GER']).missing('GER'), 'Not read yet');
  const refused = answerValues(answer({ notRecorded: 'not recorded in this campaign' }), ['GER']);
  assert.equal(refused.get('GER'), undefined);
  assert.equal(refused.missing('GER'), 'not recorded in this campaign');
});

test('a followed entry is read again only when its answer moves, and wakes its chart as answers land', () => {
  const watchers: (() => void)[] = [];
  const entry = { id: 1, status: 'followed' as const, answer: undefined, hour: 0, through: 0, version: 0,
    subscribe: (changed: () => void) => { watchers.push(changed); return () => {}; } };
  let made = 0;
  const live = liveFollowed(entry as FollowEntry, held => { made++; return held.version; });
  assert.equal(live.read(), 0);
  assert.equal(live.read(), 0);
  assert.equal(made, 1);
  live.subscribe(() => {});
  assert.equal(watchers.length, 1);
  entry.version = 1;
  assert.equal(live.read(), 1);
  assert.equal(made, 2);
});
