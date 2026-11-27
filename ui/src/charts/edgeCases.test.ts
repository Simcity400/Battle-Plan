import test from 'node:test';
import assert from 'node:assert/strict';
import { parseColor, toHex } from '../shell/appearance/color.ts';
import { nudgeApart } from './palette.ts';
import { layoutEndLabels } from './labels.ts';
import { linearTicks } from './valueAxis.ts';

test('ticks on large values stay distinct', () => {
  assert.deepEqual(linearTicks(1_000_000_000_000, 1_000_000_000_004, 5),
    [1_000_000_000_000, 1_000_000_000_001, 1_000_000_000_002, 1_000_000_000_003, 1_000_000_000_004]);
  assert.deepEqual(linearTicks(0, 1, 5), [0, 0.2, 0.4, 0.6, 0.8, 1]);
});

test('two identical colours on a mid-grey surface still come out distinct', () => {
  const green = parseColor('#008300')!;
  const [a, b] = nudgeApart([green, green], parseColor('#777777')!);
  assert.notEqual(toHex(a), toHex(b));
});

test('ticks for a tiny range at a huge value end instead of looping', () => {
  assert.deepEqual(linearTicks(1e17, 1e17 + 10), [1e17, 1e17 + 10]);
});

test('labels that cannot all fit start at the top instead of rising above the plot', () => {
  const labels = Array.from({ length: 10 }, (_, index) => ({ id: String(index), y: 50, height: 14 }));
  const placed = layoutEndLabels(labels, 0, 60);
  assert.ok([...placed.values()].every(y => y >= 7));
});
