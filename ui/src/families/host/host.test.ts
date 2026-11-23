import assert from 'node:assert/strict';
import test from 'node:test';
import { handleFamilyKeyDown, type FamilyNavTuple } from './keyboard.ts';
import { reconcileFocusedKey } from './mirror.ts';
import { parseDefaultRowHit } from './pointer.ts';
import { computeScrollGeometry, logicalToPhysicalTop, physicalToLogicalTop } from './scroll.ts';
import { reportFamilyShownHour } from './shownHour.ts';

test('compressed scroll geometry preserves linear range and maps large extents accurately', () => {
  const linear = computeScrollGeometry(2400, 400, 8_000_000);
  assert.equal(linear.compressed, false);
  assert.equal(linear.physicalHeight, 2400);
  assert.equal(physicalToLogicalTop(600, linear), 600);
  assert.equal(logicalToPhysicalTop(600, linear), 600);

  const huge = computeScrollGeometry(40_000_000, 500, 8_000_000);
  assert.equal(huge.compressed, true);
  assert.equal(huge.physicalHeight, 8_000_000);
  assert.equal(physicalToLogicalTop(0, huge), 0);
  assert.equal(physicalToLogicalTop(huge.physicalHeight - huge.viewportHeight, huge), huge.logicalHeight - huge.viewportHeight);
  const midPhysical = (huge.physicalHeight - huge.viewportHeight) / 2;
  const midLogical = physicalToLogicalTop(midPhysical, huge);
  assert.ok(Math.abs(midLogical - (huge.logicalHeight - huge.viewportHeight) / 2) < 1);
  assert.ok(Math.abs(logicalToPhysicalTop(midLogical, huge) - midPhysical) < 1);
});

test('reconcileFocusedKey respects fallbackToFirst flag', () => {
  assert.deepEqual(reconcileFocusedKey('b', ['a', 'b', 'c'], false), { key: 'b', present: true, index: 1 });
  assert.deepEqual(reconcileFocusedKey('evicted', ['a', 'b'], false), { key: null, present: false, index: -1 });
  assert.deepEqual(reconcileFocusedKey('evicted', ['a', 'b'], true), { key: 'a', present: false, index: 0 });
});

test('handleFamilyKeyDown avoids activating row 0 when focus key was evicted and fallbackToFirstOnActivate is false', () => {
  const nav: FamilyNavTuple[] = [
    ['row-a', 'Alpha', 0, -1, '', null],
    ['row-b', 'Beta', 1, -1, '', null],
  ];
  const activated: string[] = [];
  const focusedOrdinals: number[] = [];
  let prevented = false;
  const mkEvent = (key: string) => ({
    key,
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
    preventDefault() { prevented = true; },
  }) as unknown as KeyboardEvent;

  handleFamilyKeyDown(mkEvent('Enter'), {
    nav,
    focusedKey: 'evicted-key',
    fallbackToFirstOnActivate: false,
    pageStep: 5,
    blocked: false,
    isPinnedTooltip: () => false,
    onFocusKey: () => {},
    onActivate: entry => activated.push(entry[0]),
  });
  assert.equal(prevented, true);
  assert.deepEqual(activated, []);

  handleFamilyKeyDown(mkEvent('End'), {
    nav,
    totalCount: 500,
    focusedKey: 'row-a',
    fallbackToFirstOnActivate: false,
    pageStep: 10,
    blocked: false,
    isPinnedTooltip: () => false,
    onFocusKey: () => {},
    onFocusOrdinal: ord => focusedOrdinals.push(ord),
    onActivate: () => {},
  });
  assert.deepEqual(focusedOrdinals, [499]);
});

test('parseDefaultRowHit distinguishes row and toggle hits', () => {
  assert.deepEqual(parseDefaultRowHit('row:GER'), { key: 'GER', toggle: false });
  assert.deepEqual(parseDefaultRowHit('toggle:grp-1'), { key: 'grp-1', toggle: true });
  assert.equal(parseDefaultRowHit('header:col'), null);
  assert.equal(parseDefaultRowHit(null), null);
});

test('reportFamilyShownHour prefers explicit installedHour over view.through', () => {
  const reported: (number | null)[] = [];
  const sink = (hour: number | null) => { reported.push(hour); };
  assert.equal(reportFamilyShownHour(sink, true, { mode: 'picture', target: 100, through: 90, painted: 88, dim: false, tooltip: null }, 75), 75);
  assert.equal(reportFamilyShownHour(sink, true, { mode: 'picture', target: 100, through: 90, painted: 88, dim: false, tooltip: null }), 88);
  assert.equal(reportFamilyShownHour(sink, false, { mode: 'reading', target: 100, dim: false, tooltip: null }), null);
  assert.deepEqual(reported, [75, 90, null]);
});
