import assert from 'node:assert/strict';
import test from 'node:test';
import { answerValues, handoffLives, liveFollowed } from './liveFollowed.ts';
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
  const entry: FollowEntry = { id: 1, status: 'followed', answer: undefined, hour: 0, through: 0, version: 0 };
  const watchers: (() => void)[] = [];
  let made = 0;
  const live = liveFollowed({ watch: (_entry, changed) => { watchers.push(changed); return () => {}; } }, entry, held => { made++; return held.version; });
  assert.equal(live.read(), 0);
  assert.equal(live.read(), 0);
  assert.equal(made, 1);
  live.subscribe(() => {});
  assert.equal(watchers.length, 1);
  entry.version = 1;
  assert.equal(live.read(), 1);
  assert.equal(made, 2);
});

test('a set of follows asked again for new members stays drawn from the last answered set until each new one answers, never the fallback', () => {
  const store = {}, other = {};
  const rows = answer({ keys: [1n], columns: 1, values: [5n] });
  const live = (n: number) => ({ read: () => n, subscribe: () => () => {} });
  const item = (status: FollowEntry['status'] | 'off', answered: boolean, n: number) => ({ status, live: status === 'off' ? undefined : live(n),
    entry: status === 'off' ? null : { id: n, status, answer: answered ? rows : undefined, hour: 0, through: 0, version: 0 } as FollowEntry });
  const first = handoffLives([item('followed', true, 1), item('followed', true, 2)], true, null, [store, '1']);
  assert.deepEqual(first.lives.map(source => source?.read()), [1, 2], 'every one answered: drawn from them');
  const waiting = handoffLives([item('pending', false, 3), item('followed', false, 4)], true, first.held, [store, '1']);
  assert.deepEqual(waiting.lives.map(source => source?.read()), [1, 2], 'until each new one answers, the last answered set is drawn');
  assert.equal(waiting.held, first.held);
  const landed = handoffLives([item('followed', true, 3), item('followed', true, 4)], true, waiting.held, [store, '1']);
  assert.deepEqual(landed.lives.map(source => source?.read()), [3, 4], 'then the new set');
  assert.ok(handoffLives([item('pending', false, 5), item('unavailable', false, 6)], true, landed.held, [store, '1']).lives.every(source => source === undefined),
    'a new follow refused: none, so the widget asks as before');
  assert.equal(handoffLives([item('pending', false, 5), item('unavailable', false, 6)], true, landed.held, [store, '1']).held, null, 'and the old set is let go');
  assert.ok(handoffLives([item('off', false, 0), item('off', false, 0)], true, landed.held, [store, '1']).lives.every(source => source === undefined),
    'off screen: none, and an old set is never drawn when it comes back');
  assert.ok(handoffLives([item('pending', false, 5), item('pending', false, 6)], true, landed.held, [other, '1']).lives.every(source => source === undefined),
    'another store or timeline never draws the old one\'s values');
  assert.ok(handoffLives([item('pending', false, 5), item('pending', false, 6)], false, landed.held, [store, '1']).lives.every(source => source === undefined),
    'the ranked members not followed: none');
});
