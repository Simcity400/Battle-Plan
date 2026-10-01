import test from 'node:test';
import assert from 'node:assert/strict';
import { END_LABELS_OPTION, LINE_AXIS_OPTIONS, LINE_TIME_OPTIONS } from './lineChartOptions.ts';
import { resolveOptions } from './options.ts';

test('shared line settings resolve playback, end labels and axes without changing balance defaults', () => {
  const fields = [...LINE_TIME_OPTIONS, END_LABELS_OPTION, ...LINE_AXIS_OPTIONS];
  assert.deepEqual(resolveOptions(fields, {}), { playback: 'draw-in', afterPlayhead: false, windowDays: 365, endLabels: true, yMin: null, yMax: null, log: false });
  assert.deepEqual(resolveOptions(fields, { playback: 'cursor', afterPlayhead: true, yMin: -10, yMax: 20, log: false }),
    { playback: 'cursor', afterPlayhead: true, windowDays: 365, endLabels: true, yMin: -10, yMax: 20, log: false });
  const window = fields.find(field => field.key === 'windowDays')!;
  assert.equal(window.visibleWhen?.({ playback: 'window' }), true);
  assert.equal(window.visibleWhen?.({ playback: 'cursor' }), false);
});
