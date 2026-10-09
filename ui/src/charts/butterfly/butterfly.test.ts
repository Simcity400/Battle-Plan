import { test } from 'node:test';
import assert from 'node:assert/strict';
import { butterflyLabelWidth, butterflyLayout, butterflyMinimumHeight, butterflyRowViewport, butterflyScale, butterflySurface, butterflyTextWidths, ellipsizeLabel, formatRatio } from './layout.ts';
import { revealRow } from '../../components/countryList.ts';
import { nextRows } from '../rowMotion.ts';
import { approachAcrossGap } from '../easing.ts';

test('symmetric layout keeps exact values apart from labels and ratio', () => {
  const l = butterflyLayout(640, 360, 10, 12, 120, 70, 65);
  assert.equal(l.right - l.left, 120);
  assert.equal((l.right + l.left) / 2, 320);
  assert(l.left - l.half >= 70);
  assert(l.right + l.half + 70 < l.ratioX - 65);
  assert(l.barHeight < l.slot);
  assert.equal(butterflyLayout(120, 100, 0, 12, 120, 70, 65).half, 0);
});
test('scale fits the largest shown side, with missing distinct from zero', () => {
  assert.equal(butterflyScale([{ id: 'x', left: 100, right: 20 }, { id: 'y', left: null, right: 400 }]), 420);
  assert.equal(butterflyScale([{ id: 'x', left: null, right: 0 }]), 1);
  assert.equal(butterflyScale([{ id: 'correction', left: -400, right: 20 }]), 420);
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

test('caller label measurement includes optional flag room and fits the label column', () => {
  const names = ['Italy', 'Chinese Soviet Republic', 'United Kingdom'];
  const measure = (text: string) => Array.from(text).length * 7;
  const longest = Math.max(...names.map(measure));
  const column = butterflyLabelWidth(800, 12, longest);
  assert.equal(column, longest + 36);
  for (const name of names) assert.equal(ellipsizeLabel(name, column - 36, measure), name);
  const withoutFlags = butterflyLabelWidth(800, 12, longest, false);
  assert.equal(withoutFlags, column - 24);
  for (const name of names) assert.equal(ellipsizeLabel(name, withoutFlags - 12, measure), name);
  const narrow = butterflyLabelWidth(500, 12, longest);
  assert.equal(narrow, 160);
  const truncated = ellipsizeLabel(names[1], narrow - 36, measure);
  assert(truncated.endsWith('…')); assert(measure(truncated) <= narrow - 36);
  assert.equal(ellipsizeLabel('𝄞𝄞𝄞', 14, measure), '𝄞…');
  assert.equal(ellipsizeLabel('name', 2, measure), '');
});

test('two or three countries keep the approved row spacing in a tall card', () => {
  for (const count of [2, 3]) {
    const layout = butterflyLayout(800, 600, count, 12, 200, 60, 50);
    assert.equal(layout.slot, 42);
    assert.equal(layout.barHeight, 20);
  }
  assert.equal(butterflyLayout(800, 100, 30, 12, 200, 60, 50).slot, 21.6);
});

test('3, 10 and 30 rows fit or scroll only at the minimum slot height', () => {
  for (const count of [3, 10, 30]) {
    const minimum = butterflyMinimumHeight(count, 12), height = Math.max(282, minimum);
    const layout = butterflyLayout(800, height, count, 12, 200, 60, 50);
    assert(layout.top + count * layout.slot <= height + 0.001);
    assert(layout.barHeight <= 20); assert(layout.barHeight >= 15);
    assert.equal(minimum > 282, count === 30);
  }
  const defaultCard = butterflyLayout(800, 274, 10, 12, 200, 60, 50);
  assert(defaultCard.barHeight >= 16 && defaultCard.barHeight < 17);
  // The gap note leaves less room, but ten minimum-height slots still fit.
  assert(butterflyMinimumHeight(10, 12) <= 255);
});

test('5000 rows retain their full scroll extent with a viewport-sized canvas and keyboard access to the last row', () => {
  for (const count of [3, 10, 30, 5000]) {
    const surface = butterflySurface(count, 12, 274);
    assert.equal(surface.canvasHeight, 274);
    const layout = butterflyLayout(800, surface.contentHeight, count, 12, 200, 60, 50);
    const lastY = layout.top + (count - 0.5) * layout.slot;
    const rowsHeight = butterflyRowViewport(layout.top, surface.canvasHeight, 0).height;
    const scrollTop = Math.max(0, revealRow((lastY - layout.top) / layout.slot - 0.5, 0, rowsHeight, layout.slot));
    const visible = butterflyRowViewport(layout.top, surface.canvasHeight, scrollTop);
    assert(lastY - layout.slot / 2 >= visible.start - 0.001);
    assert(lastY + layout.slot / 2 <= visible.end + 0.001);
    assert(scrollTop <= surface.contentHeight - surface.canvasHeight + 0.001);
    assert.equal(scrollTop === 0, count <= 10);
    // Home reveals the first row below the header after End scrolled to the last.
    const firstScroll = revealRow(0, scrollTop, rowsHeight, layout.slot);
    assert.equal(firstScroll, 0);
    assert.equal(butterflyRowViewport(layout.top, surface.canvasHeight, firstScroll).start, layout.top);
  }
});

test('column headers keep their room while rows scroll through a short viewport', () => {
  const layout = butterflyLayout(800, 700, 30, 12, 200, 60, 50);
  const first = butterflyRowViewport(layout.top, 274, 0);
  const scrolled = butterflyRowViewport(layout.top, 274, 100);
  assert.equal(scrolled.start - 100, first.start);
  assert.equal(scrolled.height, first.height);
  assert.equal(scrolled.end - 100, 274);
  const firstRowTop = layout.top - 100;
  assert(firstRowTop < layout.top);
  assert(firstRowTop + 100 < scrolled.start);
});

test('caller text fitting has no argument-count limit for large item lists', () => {
  function* items() {
    for (let i = 0; i < 150000; i++) yield { leftText: String(i), rightText: 'exact caller value', ratioText: '2 : 1' };
  }
  assert.deepEqual(butterflyTextWidths(items(), text => text.length, 'Ratio', true, true), { valueWidth: 18, ratioWidth: 5 });
  assert.deepEqual(butterflyTextWidths(items(), text => text.length, 'Ratio', false, false), { valueWidth: 0, ratioWidth: 0 });
});
