import assert from 'node:assert/strict';
import test from 'node:test';
import { nextHolder } from './layout.ts';

const span = (id: string, label: string, icon?: string) => ({ id, firstHour: 0, lastHour: 0, label, icon });

test('ordinary lanes update a holder name and picture immediately while preserving its fade', () => {
  const first = nextHolder(undefined, span('5#1', 'Earlier name', 'a.png'));
  const previous = span('4#1', 'Previous holder');
  first.previous = previous;
  first.t = 0.4;
  const updated = nextHolder(first, span('5#1', 'Name at this hour', 'b.png'), false);
  assert.deepEqual([updated.current?.label, updated.current?.icon, updated.t], ['Name at this hour', 'b.png', 0.4]);
  assert.equal(updated.previous, previous);
  const changed = nextHolder(updated, span('6#1', 'Another holder'), false);
  assert.deepEqual([changed.previous?.id, changed.current?.id, changed.t], ['5#1', '6#1', 0]);
});

test('the left column redraws a holder whose name or picture changes under the same id', () => {
  const first = nextHolder(undefined, span('5#1', 'Gewehr 98k', 'b.png'));
  assert.deepEqual([first.current?.label, first.t], ['Gewehr 98k', 1]);
  assert.equal(nextHolder(first, span('5#1', 'Gewehr 98k', 'b.png')), first);
  const back = nextHolder(first, span('5#1', 'Infantry Equipment I', 'a.png'));
  assert.deepEqual([back.previous?.label, back.current?.label, back.current?.icon, back.t], ['Gewehr 98k', 'Infantry Equipment I', 'a.png', 0]);
  assert.notEqual(nextHolder(back, span('5#1', 'Infantry Equipment I', 'c.png')), back);
  assert.equal(nextHolder(back, null).current, null);
});

test('a native country holder updates pending/refusal without restarting its picture fade', () => {
  const pending = { ...span('country', 'Synthetic country'), countryFlag: { flagPending: true } };
  const state = nextHolder(undefined, pending);
  const unavailable = { ...pending, countryFlag: { flagPending: false, flagReason: 'Recorded mod flag is missing.' } };
  const completed = nextHolder(state, unavailable);
  assert.equal(completed, state);
  assert.equal(completed.current?.countryFlag?.flagReason, 'Recorded mod flag is missing.');
  assert.equal(completed.current?.countryFlag?.flagPending, false);
  assert.equal(completed.t, 1);
  const loaded = nextHolder(completed, { ...pending, countryFlag: { flag: 'synthetic-flag.png' } });
  assert.equal(loaded.previous?.countryFlag?.flagReason, 'Recorded mod flag is missing.');
  assert.equal(loaded.current?.countryFlag?.flag, 'synthetic-flag.png');
  assert.equal(loaded.t, 0);
});
