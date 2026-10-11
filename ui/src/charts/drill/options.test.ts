import test from 'node:test';
import assert from 'node:assert/strict';
import { LOG_OPTIONS } from '../log/logOptions.ts';
import { SUNBURST_OPTIONS } from '../sunburst/sunburstOptions.ts';
import { DATE_OVERLAY_OPTION } from '../race/options.ts';
import { DRILL_OPTION } from './drillState.ts';
import { resolveOptions } from '../../dashboard/options.ts';

test('shared display settings retain saved values with concise labels and hints', () => {
  assert.equal(DRILL_OPTION.hint, undefined);
  assert.equal(DATE_OVERLAY_OPTION.label, 'Date in the corner');
  assert.equal(DATE_OVERLAY_OPTION.hint, undefined);
  const saved = { rowAnimation: false, months: true, chips: false };
  assert.deepEqual(resolveOptions(LOG_OPTIONS, saved), { density: 'comfortable', ...saved });
  for (const key of ['rowAnimation', 'months', 'chips']) assert.equal(LOG_OPTIONS.find(field => field.key === key)!.hint, undefined);
  const settings = { growth: 'fill', sliceLabels: false, legend: false, breadcrumb: false, dimming: false };
  assert.deepEqual(resolveOptions(SUNBURST_OPTIONS, settings), settings);
  assert.equal(SUNBURST_OPTIONS.find(field => field.key === 'breadcrumb')!.label, 'Path');
  assert.equal(SUNBURST_OPTIONS.find(field => field.key === 'growth')!.hint, 'Share shows proportions; Fill shows progress.');
  for (const key of ['sliceLabels', 'legend', 'breadcrumb', 'dimming']) assert.equal(SUNBURST_OPTIONS.find(field => field.key === key)!.hint, undefined);
});
