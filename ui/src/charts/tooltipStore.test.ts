import assert from 'node:assert/strict';
import test from 'node:test';
import { hideTooltip, refreshTooltip, showTooltip, tooltipStore } from './tooltip.ts';

test('playback refresh updates current content without hiding or moving the tooltip', () => {
  const owner = 'table';
  const initial = { title: 'Before', rows: [] };
  const current = { title: 'After', rows: [] };
  showTooltip(owner, initial, 12, 34);
  const published: unknown[] = [];
  const unsubscribe = tooltipStore.subscribe(() => published.push(tooltipStore.get()));
  try {
    refreshTooltip(owner, () => current);
    assert.deepEqual(published, [{ owner, content: current, x: 12, y: 34 }]);
    refreshTooltip(owner, () => current);
    assert.equal(published.length, 1);
  } finally { unsubscribe(); hideTooltip(owner); }
});

test('a retained focus cannot reclaim another widget tooltip and builds nothing', () => {
  const owner = 'focused-table';
  const other = 'pointer-chart';
  showTooltip(owner, { title: 'Focused row', rows: [] }, 12, 34);
  showTooltip(other, { title: 'Pointer point', rows: [] }, 56, 78);
  const before = tooltipStore.get();
  let built = 0;
  let published = 0;
  const unsubscribe = tooltipStore.subscribe(() => published++);
  try {
    refreshTooltip(owner, () => { built++; return { title: 'New row data', rows: [] }; });
    hideTooltip(owner);
    assert.equal(tooltipStore.get(), before);
    assert.equal(built, 0);
    assert.equal(published, 0);
    hideTooltip(other);
    refreshTooltip(owner, () => { built++; return { title: 'New row data', rows: [] }; });
    assert.equal(built, 0);
    assert.equal(tooltipStore.get(), null);
  } finally { unsubscribe(); hideTooltip(owner); hideTooltip(other); }
});

test('missing refreshed content clears only the current owner', () => {
  const owner = 'removed-row';
  showTooltip(owner, { title: 'Row', rows: [] }, 12, 34);
  refreshTooltip(owner, () => null);
  assert.equal(tooltipStore.get(), null);
});
