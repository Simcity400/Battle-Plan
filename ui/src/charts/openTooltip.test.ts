import assert from 'node:assert/strict';
import { test } from 'node:test';
import { openTooltip } from './openTooltip.ts';
import { tooltipStore } from './tooltip.ts';

test('review 8 3: an open inspection follows the installed values, and withdraws once its content is gone', () => {
  let percent: string | null = '50%';
  const tips = openTooltip('research');
  tips.show(() => percent === null ? null : { title: 'Radar', rows: [{ label: 'Progress', value: percent }] }, 'slot:0', 10, 20);
  percent = '63%';
  tips.refresh();
  assert.deepEqual([tooltipStore.get()?.content.rows?.[0]?.value, tooltipStore.get()?.x], ['63%', 10], 'a stationary pointer sees the new install');
  percent = null;
  tips.refresh();
  assert.equal(tooltipStore.get(), null);
  tips.hide();
  percent = '70%';
  tips.refresh();
  assert.equal(tooltipStore.get(), null, 'a closed inspection is not reopened');
});
