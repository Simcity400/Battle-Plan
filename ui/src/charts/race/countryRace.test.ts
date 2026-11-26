import assert from 'node:assert/strict';
import test from 'node:test';
import { countryRaceSegments, heldStackSegment } from './countryRace.ts';
import { buildStack, stackedSegments } from '../stack.ts';
import { fitTooltip } from '../tooltip.ts';

test('country mix expands shared stack samples without inventing intermediate values', () => {
  assert.deepEqual(heldStackSegment({ hours: [1, 3], bands: [{ low: [0, 0], high: [2, 8] }] }),
    { hours: [1, 3, 3], bands: [{ low: [0, 0, 0], high: [2, 2, 8] }] });
});

test('countryRaceSegments uses half-pixel thinning and preserves signed peaks and gaps', () => {
  const stack = buildStack([[{ firstHour: 0, lastHour: 1, value: 2 }, { firstHour: 2, lastHour: 2, value: 12 },
    { firstHour: 3, lastHour: 4, value: -8 }, { firstHour: 8, lastHour: 10, value: 3, breakBefore: true }]]);
  const segments = countryRaceSegments(stack, 1, 10, 8);
  assert.deepEqual(segments, stackedSegments(stack, 1, 10, 4).map(heldStackSegment));
  assert.ok(segments.some(segment => segment.bands[0].high.includes(12)));
  assert.ok(segments.some(segment => segment.bands[0].low.includes(-8)));
  assert.equal(segments.length, 2);
  assert.deepEqual(countryRaceSegments(stack, 5, 7, 8), []);
});

test('shared fitting keeps supplied totals and reads the full compacted tooltip aloud', () => {
  const full = { title: 'Country', rows: [{ label: 'Level', value: '10' }, { label: 'Ceiling', value: '20' },
    ...Array.from({ length: 10 }, (_, index) => ({ label: `Part ${index}`, value: '1' })), { label: 'Net', value: '2' }] };
  const compact = { ...full, rows: [full.rows[0], full.rows.at(-1)!] };
  const fitted = fitTooltip([full, compact], 120, { keep: 1 });
  assert.deepEqual(fitted.rows.map(row => row.label), ['Level', 'Net']);
  assert.match(fitted.spoken!, /Ceiling 20; Part 0 1/);
  const noTotal = fitTooltip([full], 90, { keep: 0 });
  assert.ok(!noTotal.rows.some(row => row.label === 'Net'));
});
