import { test } from 'node:test';
import assert from 'node:assert/strict';
import { butterflyPartAt, butterflyPartSpans, butterflyLayout, butterflyRowViewport, butterflyScale, butterflySurface, butterflyTextWidths, butterflyTicks, formatRatio } from './layout.ts';
import { revealRow } from '../../components/countryList.ts';
import { nextRows } from '../rowMotion.ts';
import { approachAcrossGap } from '../easing.ts';

test('scale fits the largest shown side, with missing distinct from zero', () => {
  assert.equal(butterflyScale([{ id: 'x', left: 100, right: 20 }, { id: 'y', left: null, right: 400 }]), 420);
  assert.equal(butterflyScale([{ id: 'x', left: null, right: 0 }]), 1);
  assert.equal(butterflyScale([{ id: 'correction', left: -400, right: 20 }]), 420);
});
test('each side’s value axis starts at zero and keeps its labels apart', () => {
  const ticks = butterflyTicks(13.6e6, 84);
  assert.deepEqual(ticks, [0, 10e6]);
  for (const [scale, half] of [[13.6e6, 84], [4.62e6, 300], [1, 500], [9e15, 120]]) {
    const drawn = butterflyTicks(scale, half);
    assert.equal(drawn[0], 0);
    for (let i = 1; i < drawn.length; i++) {
      assert(drawn[i] <= scale);
      assert((drawn[i] - drawn[i - 1]) / scale * half >= 56);
    }
  }
  assert.deepEqual(butterflyTicks(13.6e6, 40), [0], 'a side too narrow for a second label keeps only zero');
  assert.deepEqual(butterflyTicks(1, 0), [0]);
});
test('ratio formatting keeps zero and unknown denominators missing and exact large integers', () => {
  assert.equal(formatRatio(185, 100), '1.85 : 1');
  assert.equal(formatRatio(0n, 7n), '0.00 : 1');
  assert.equal(formatRatio(1, 0), '—');
  assert.equal(formatRatio(null, 100), '—');
  assert.equal(formatRatio(100, null), '—');
  assert.equal(formatRatio(9223372036854775807n, 9223372036854775807n), '1.00 : 1');
  assert.equal(formatRatio(199n, 200n), '1.00 : 1');
  assert.equal(formatRatio(-185n, 100n), '-1.85 : 1');
});
test('row motion preserves identities and does not ease across missing values', () => {
  const items = [{ id: 'z', left: null, right: 100 }, { id: 'b', left: 1, right: 2 }, { id: 'a', left: 2, right: 1 }];
  const before = nextRows([], items.slice(1).map(item => ({ key: item.id, item })), 0, 450, true);
  const after = nextRows(before, [...items.slice(1)].reverse().map(item => ({ key: item.id, item })), 10, 450);
  assert.equal(after[0].key, 'a'); assert.equal(after[0].phase, 'staying');
  assert.equal(approachAcrossGap(10, null, 16, 180, false), null);
  assert.equal(approachAcrossGap(10, 100, 16, 180, true), 100);
});




test('5000 rows retain their full scroll extent with a viewport-sized canvas and keyboard access to the last row', () => {
  for (const count of [3, 10, 30, 5000]) {
    const surface = butterflySurface(count, 274);
    assert.equal(surface.canvasHeight, 274);
    const layout = butterflyLayout(800, surface.contentHeight, count, 200, 60, 50);
    const lastY = layout.top + (count - 0.5) * layout.slot;
    const rowsHeight = butterflyRowViewport(layout.top, surface.canvasHeight, 0).height;
    const scrollTop = Math.max(0, revealRow((lastY - layout.top) / layout.slot - 0.5, 0, rowsHeight, layout.slot));
    const visible = butterflyRowViewport(layout.top, surface.canvasHeight, scrollTop);
    assert(lastY - layout.slot / 2 >= visible.start - 0.001);
    assert(lastY + layout.slot / 2 <= visible.end + 0.001);
    assert(scrollTop <= surface.contentHeight - surface.canvasHeight + 0.001);
    assert.equal(scrollTop === 0, count <= 10);
    const firstScroll = revealRow(0, scrollTop, rowsHeight, layout.slot);
    assert.equal(firstScroll, 0);
    assert.equal(butterflyRowViewport(layout.top, surface.canvasHeight, firstScroll).start, layout.top);
  }
});


test('caller text fitting has no argument-count limit for large item lists', () => {
  function* items() {
    for (let i = 0; i < 150000; i++) yield { leftText: String(i), rightText: 'exact caller value', ratioText: '2 : 1' };
  }
  assert.deepEqual(butterflyTextWidths(items(), text => text.length, 'Ratio', true, true), { valueWidth: 18, ratioWidth: 5 });
  assert.deepEqual(butterflyTextWidths(items(), text => text.length, 'Ratio', false, false), { valueWidth: 0, ratioWidth: 0 });
});

test('a missing value’s text keeps its room with values off, so it never runs into the names', () => {
  const items = [{ left: null, right: 5, leftText: 'Unknown', rightText: '5', ratioText: '—' }];
  assert.deepEqual(butterflyTextWidths(items, text => text.length, 'Ratio', false, false), { valueWidth: 7, ratioWidth: 0 });
});

test('a side bar is shared by its parts in proportion and the pointer finds its part', () => {
  const parts = [{ id: 'a', value: 3, color: '#111' }, { id: 'b', value: -2, color: '#222' }, { id: 'c', value: 1, color: '#333', hatched: true }];
  const spans = butterflyPartSpans(parts, 100);
  assert.deepEqual(spans, [{ from: 0, to: 75 }, { from: 75, to: 75 }, { from: 75, to: 100 }], 'a negative part takes no length');
  assert.deepEqual(butterflyPartSpans([], 100), []);
  assert.deepEqual(butterflyPartSpans([{ id: 'z', value: 0, color: '#000' }], 100), [{ from: 0, to: 0 }]);
  assert.deepEqual(butterflyPartAt(-10, spans, []), { side: 'left', index: 0 });
  assert.deepEqual(butterflyPartAt(80, [], spans), { side: 'right', index: 2 });
  assert.equal(butterflyPartAt(120, [], spans), null, 'past the bar');
  assert.equal(butterflyPartAt(-10, [], spans), null, 'a side without parts');
});
