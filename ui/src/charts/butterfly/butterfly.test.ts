import { test } from 'node:test';
import assert from 'node:assert/strict';
import { BUTTERFLY_HEADER, butterflyLabelWidth, butterflyLayout, butterflyMinimumHeight, butterflyMinimumWidth, butterflyNameX, butterflyRowViewport, butterflyScale, butterflySurface,
  butterflyTextWidths, butterflyTicks, ellipsizeLabel, formatRatio } from './layout.ts';
import { FLAG, MIN_LABEL, NAME_PAD, NAME_X, VALUE_GAP } from '../race/draw.ts';
import { barThickness, MIN_SLOT, RACE_AXIS, raceLayout } from '../race/race.ts';
import { revealRow } from '../../components/countryList.ts';
import { nextRows } from '../rowMotion.ts';
import { approachAcrossGap } from '../easing.ts';

test('names sit left of one centred axis, with each side’s exact value and the ratio clear of each other', () => {
  const l = butterflyLayout(640, 360, 10, 150, 70, 65);
  assert.equal(l.labelWidth, 150);
  // The axis is midway between the name column and the ratio column's room.
  assert.equal(l.axis - l.labelWidth, l.ratioX - 65 - 2 * VALUE_GAP - l.axis);
  // A full left bar's value ends after the names; a full right bar's before the ratio.
  assert(l.axis - l.half - VALUE_GAP - 70 > l.labelWidth);
  assert(l.axis + l.half + VALUE_GAP + 70 < l.ratioX - 65);
  assert(l.barHeight < l.slot);
  assert.equal(butterflyLayout(120, 100, 0, 120, 70, 65).half, 0);
  // Without a ratio column the right side reaches the chart's edge.
  assert(butterflyLayout(640, 360, 10, 150, 70, 0).half > l.half);
});
test('the chart scrolls sideways only once each bar would be shorter than its least length', () => {
  const minimum = butterflyMinimumWidth(150, 70, 65);
  assert(butterflyLayout(minimum, 360, 10, 150, 70, 65).half >= 24 - 1e-9);
  assert(butterflyLayout(minimum - 10, 360, 10, 150, 70, 65).half < 24);
});
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

test('the name column is the race’s: flag, name and padding, at most a third of the chart', () => {
  const names = ['Italy', 'Chinese Soviet Republic', 'United Kingdom'];
  const measure = (text: string) => Array.from(text).length * 7;
  const longest = Math.max(...names.map(measure));
  const column = butterflyLabelWidth(800, longest);
  assert.equal(column, NAME_X + longest + NAME_PAD);
  assert.equal(butterflyNameX(), NAME_X);
  for (const name of names) assert.equal(ellipsizeLabel(name, column - NAME_X - NAME_PAD, measure), name);
  const withoutFlags = butterflyLabelWidth(800, longest, false);
  assert.equal(withoutFlags, FLAG.x + longest + NAME_PAD);
  assert.equal(butterflyNameX(false), FLAG.x);
  for (const name of names) assert.equal(ellipsizeLabel(name, withoutFlags - FLAG.x - NAME_PAD, measure), name);
  const narrow = butterflyLabelWidth(500, longest);
  assert.equal(narrow, 160);
  const truncated = ellipsizeLabel(names[1], narrow - NAME_X - NAME_PAD, measure);
  assert(truncated.endsWith('…')); assert(measure(truncated) <= narrow - NAME_X - NAME_PAD);
  assert.equal(butterflyLabelWidth(800, 0), MIN_LABEL);
  assert.equal(ellipsizeLabel('𝄞𝄞𝄞', 14, measure), '𝄞…');
  assert.equal(ellipsizeLabel('name', 2, measure), '');
});

test('rows take the race’s height and bar thickness for the same room and slots', () => {
  for (const [height, slots] of [[274, 10], [600, 2], [600, 3], [400, 10], [100, 30]]) {
    const layout = butterflyLayout(800, height, slots, 200, 60, 50);
    // The race keeps 8 px above its rows; the butterfly keeps its headings there.
    const race = raceLayout(800, height - BUTTERFLY_HEADER + 8, slots, 200, 60);
    assert.equal(layout.slot, race.slot);
    assert.equal(layout.barHeight, race.thickness);
    assert.equal(layout.barHeight, barThickness(layout.slot));
  }
  assert.equal(butterflyLayout(800, 100, 30, 200, 60, 50).slot, MIN_SLOT);
});

test('3, 10 and 30 rows fit or scroll only at the race’s least row height', () => {
  for (const count of [3, 10, 30]) {
    const minimum = butterflyMinimumHeight(count), height = Math.max(282, minimum);
    const layout = butterflyLayout(800, height, count, 200, 60, 50);
    assert(layout.top + count * layout.slot + RACE_AXIS <= height + 0.001);
    assert(layout.slot >= MIN_SLOT);
    assert.equal(minimum > 282, count === 30);
  }
  // The gap note leaves less room, but ten least-height rows still fit.
  assert(butterflyMinimumHeight(10) <= 255);
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
    // Home reveals the first row below the header after End scrolled to the last.
    const firstScroll = revealRow(0, scrollTop, rowsHeight, layout.slot);
    assert.equal(firstScroll, 0);
    assert.equal(butterflyRowViewport(layout.top, surface.canvasHeight, firstScroll).start, layout.top);
  }
});

test('column headings and the value axis keep their room while rows scroll through a short viewport', () => {
  const layout = butterflyLayout(800, 700, 30, 200, 60, 50);
  const first = butterflyRowViewport(layout.top, 274, 0);
  const scrolled = butterflyRowViewport(layout.top, 274, 100);
  assert.equal(scrolled.start - 100, first.start);
  assert.equal(scrolled.height, first.height);
  assert.equal(scrolled.end - 100, 274 - RACE_AXIS);
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

test('a missing value’s text keeps its room with values off, so it never runs into the names', () => {
  const items = [{ left: null, right: 5, leftText: 'Unknown', rightText: '5', ratioText: '—' }];
  assert.deepEqual(butterflyTextWidths(items, text => text.length, 'Ratio', false, false), { valueWidth: 7, ratioWidth: 0 });
});
