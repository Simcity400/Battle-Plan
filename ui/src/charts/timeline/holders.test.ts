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
  // A lane about one production line keeps its id; its variant's name and art change with the hour shown.
  const first = nextHolder(undefined, span('5#1', 'Gewehr 98k', 'b.png'));
  assert.deepEqual([first.current?.label, first.t], ['Gewehr 98k', 1]);
  // The same holder drawn again keeps its state.
  assert.equal(nextHolder(first, span('5#1', 'Gewehr 98k', 'b.png')), first);
  // Ending the range before the switch: same id, the earlier variant's name and art, faded in.
  const back = nextHolder(first, span('5#1', 'Infantry Equipment I', 'a.png'));
  assert.deepEqual([back.previous?.label, back.current?.label, back.current?.icon, back.t], ['Gewehr 98k', 'Infantry Equipment I', 'a.png', 0]);
  // A picture alone changing also redraws it.
  assert.notEqual(nextHolder(back, span('5#1', 'Infantry Equipment I', 'c.png')), back);
  // And a lane emptying fades out to nothing.
  assert.equal(nextHolder(back, null).current, null);
});
