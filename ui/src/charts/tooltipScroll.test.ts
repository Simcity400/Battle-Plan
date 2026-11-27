import assert from 'node:assert/strict';
import test from 'node:test';
import { handleActiveTooltipKey, handleActiveTooltipWheel, hideTooltip, measureTooltipScroll, scrollTooltip, showTooltip, tooltipStore, tooltipText } from './tooltip.ts';
import { tooltipRowOffsets, tooltipRowWindow, tooltipScrollTop } from './tooltipScroll.ts';

test('a measured scroll viewport can reach every tooltip row without a fixed count', () => {
  const rows = Array.from({ length: 5_000 }, (_, index) => ({ label: `Row ${index}` }));
  const offsets = tooltipRowOffsets(rows);
  assert.deepEqual(tooltipRowWindow(offsets, 0, 170), { first: 0, last: 10 });
  assert.deepEqual(tooltipRowWindow(offsets, 0, 680), { first: 0, last: 40 });
  assert.deepEqual(tooltipRowWindow(offsets, Infinity, 170), { first: 4_990, last: 5_000 });
  assert.equal(tooltipScrollTop(offsets, -50, 170), 0);
  assert.deepEqual(tooltipRowWindow(offsets, 0, 0), { first: 0, last: 0 });
  const measured = tooltipRowOffsets(rows, new Map([[0, 100], [1, 50]]));
  assert.deepEqual(tooltipRowWindow(measured, 100, 50), { first: 1, last: 2 });
});

test('wheel and page scrolling retain complete content, respect ownership and preserve position as facts arrive', () => {
  const content = { title: 'All rows', scrollable: true, scrollId: 'item', rows: [{ label: 'First' }, { label: 'Last' }] };
  showTooltip('scroll-test', content, 10, 10, 'initial');
  assert.equal(scrollTooltip('other', 100), false);
  assert.equal(scrollTooltip('scroll-test', 100), true);
  assert.equal(tooltipStore.get()!.scrollTop, 100);
  measureTooltipScroll('scroll-test', 75);
  scrollTooltip('scroll-test', 1, true);
  assert.equal(tooltipStore.get()!.scrollTop, 175, 'page movement uses measured row height rather than the window');
  assert.match(tooltipText(tooltipStore.get()!.content), /First; Last/);
  showTooltip('scroll-test', { ...content, title: 'Resolved name' }, 10, 10, 'facts arrived');
  assert.equal(tooltipStore.get()!.scrollTop, 175);
  assert.equal(scrollTooltip('scroll-test', -200), true);
  assert.equal(tooltipStore.get()!.scrollTop, 0);
  scrollTooltip('scroll-test', 20);
  showTooltip('scroll-test', { ...content, scrollId: 'another item' }, 10, 10, 'another');
  assert.equal(tooltipStore.get()!.scrollTop, 0);
  showTooltip('scroll-test', { title: 'Existing non-scrolling tooltip', rows: [] }, 10, 10, 'default');
  assert.equal(scrollTooltip('scroll-test', 100), false);
  assert.equal(handleActiveTooltipWheel('scroll-test', 40, 200), false);
  assert.equal(handleActiveTooltipKey('scroll-test', 'PageDown', false, 200), false);
  hideTooltip('scroll-test');
});

test('active wheel and keyboard helpers scroll an overflowing tooltip to its tail without stealing edge or other-widget events', () => {
  const rows = Array.from({ length: 16 }, (_, index) => ({ label: `Gap ${index}`, value: `key_${index}: reason` }));
  showTooltip('scroll-input', { title: 'Gaps', scrollable: true, scrollId: 'gaps', rows }, 10, 10, 'gaps');
  measureTooltipScroll('scroll-input', 120);
  const offsets = tooltipRowOffsets(rows);
  const maxScroll = offsets[offsets.length - 1]! - 120;
  assert.equal(handleActiveTooltipWheel('other-widget', 40, maxScroll), false, 'wheel over another widget is never consumed');
  assert.equal(handleActiveTooltipWheel(null, 40, maxScroll), false, 'wheel outside any owning widget is never consumed');
  assert.equal(handleActiveTooltipKey('other-widget', 'PageDown', false, maxScroll), false, 'PageDown in another widget is never consumed');
  assert.equal(handleActiveTooltipWheel('scroll-input', -40, maxScroll), false, 'at the top, upward wheel passes through');
  assert.equal(handleActiveTooltipKey('scroll-input', 'PageDown', false, maxScroll), true);
  assert.equal(tooltipStore.get()!.scrollTop, 120);
  assert.equal(handleActiveTooltipWheel('scroll-input', maxScroll, maxScroll), true);
  assert.equal(tooltipStore.get()!.scrollTop, maxScroll);
  assert.equal(handleActiveTooltipWheel('scroll-input', 40, maxScroll), false, 'at the bottom, downward wheel passes through');
  const win = tooltipRowWindow(offsets, tooltipStore.get()!.scrollTop!, 120);
  assert.equal(win.last, 16, 'scrolling reaches the final row');
  hideTooltip('scroll-input');
});

test('long native header and trailing text share the viewport without hiding the complete row list', () => {
  const offsets = tooltipRowOffsets(Array.from({ length: 5_000 }, (_, index) => ({ label: `Native row ${index}` })));
  const leading = 1_200;
  const trailing = 500;
  const height = 100;
  assert.deepEqual(tooltipRowWindow(offsets, 0, height, leading, trailing), { first: 0, last: 0 }, 'only the long header is currently visible');
  assert.deepEqual(tooltipRowWindow(offsets, leading - 50, height, leading, trailing), { first: 0, last: 3 });
  assert.deepEqual(tooltipRowWindow(offsets, leading, height, leading, trailing), { first: 0, last: 6 });
  assert.deepEqual(tooltipRowWindow(offsets, leading + offsets.at(-1)! - height, height, leading, trailing), { first: 4_994, last: 5_000 });
  const bottom = tooltipScrollTop(offsets, Infinity, height, leading, trailing);
  assert.equal(bottom, leading + offsets.at(-1)! + trailing - height, 'the entire footer remains reachable');
  assert.deepEqual(tooltipRowWindow(offsets, bottom, height, leading, trailing), { first: 5_000, last: 5_000 });
});
