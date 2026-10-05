import assert from 'node:assert/strict';
import test from 'node:test';
import { sortRows } from './logic.ts';

test('ranked country table ties agree with the native UTF-8 order for mod tags', () => {
  const rows = [{ tag: '\u{10000}', value: 2n }, { tag: '\ue000', value: 2n }];
  for (const direction of ['ascending', 'descending'] as const) {
    assert.deepEqual(sortRows(rows, row => row.value, direction, row => row.tag).map(row => row.tag), ['\ue000', '\u{10000}']);
    assert.deepEqual(sortRows(rows, () => null, direction, row => row.tag).map(row => row.tag), ['\ue000', '\u{10000}']);
  }
});
