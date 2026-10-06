import assert from 'node:assert/strict';
import test from 'node:test';
import { rankChanges, sortRows } from './logic.ts';

test('ranked country table ties agree with the native UTF-8 order for mod tags', () => {
  const rows = [{ tag: '\u{10000}', value: 2n }, { tag: '\ue000', value: 2n }];
  for (const direction of ['ascending', 'descending'] as const) {
    assert.deepEqual(sortRows(rows, row => row.value, direction, row => row.tag).map(row => row.tag), ['\ue000', '\u{10000}']);
    assert.deepEqual(sortRows(rows, () => null, direction, row => row.tag).map(row => row.tag), ['\ue000', '\u{10000}']);
  }
});

test('country ties follow the current tag while rank markers retain the country identity', () => {
  const before = [{ id: 'slot-one', tag: 'Z', value: 2n }, { id: 'slot-two', tag: 'B', value: 2n }];
  const after = [{ ...before[0], tag: 'A' }, before[1]];
  for (const direction of ['ascending', 'descending'] as const) {
    const sort = (rows: typeof before) => sortRows(rows, row => row.value, direction, row => row.id, row => row.tag);
    const initial = rankChanges(new Map(), sort(before).map(row => row.id), 0);
    const changed = rankChanges(initial, sort(after).map(row => row.id), 100);
    assert.deepEqual(sort(after).map(row => row.tag), ['A', 'B']);
    assert.equal(changed.get('slot-one')?.delta, 1);
    assert.equal(changed.get('slot-two')?.delta, -1);
    assert.equal(rankChanges(changed, ['slot-one'], 200).has('slot-two'), false);
    assert.equal(rankChanges(changed, sort(after).map(row => row.id), 2600).get('slot-one')?.delta, 0);
    assert.deepEqual(sortRows(after, () => null, direction, row => row.id, row => row.tag).map(row => row.tag), ['A', 'B']);
  }
});
