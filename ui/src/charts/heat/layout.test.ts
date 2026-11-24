import { keyTarget } from '../rowNavigation.ts';
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { cellRect, cellRuns, heatNavigation, heatWindow, heatRowAt, periodAt } from './layout.ts';
import { keyedStop, keyStopList } from '../hover.ts';
import { hitZone, plotFrame } from '../frame.ts';
const parts = [
  { id: 'a', label: 'A', color: '#cc4422', share: 0.45 },
  { id: 'b', label: 'B', color: '#2277cc', share: 0.35 },
  { id: 'c', label: 'C', color: '#22aa77', share: 0.2 },
];
test('layout clips exact periods, leaves missing days blank and hides future reads', () => {
  const periods = [{ firstHour: 0, lastHour: 23, readHour: 23 }, { firstHour: 48, lastHour: 71, readHour: 70 }];
  assert.equal(periodAt(periods, 30), null);
  assert.equal(periodAt(periods, 49), 1);
  const frame = { plotTop: 20, plotHeight: 100, plotLeft: 80, plotWidth: 72, toX: (h: number) => 80 + h };
  assert.deepEqual(cellRect(periods[0], 1, 2, frame, 10, 72, null), { left: 90, top: 71, width: 14, height: 48 });
  assert.equal(cellRect(periods[1], 0, 2, frame, 0, 72, 50), null);
  assert.equal(cellRect({ ...periods[1], readHour: 50 }, 0, 2, frame, 0, 72, 55)?.width, 8);
});
test('keyboard steps rows with boundaries and no phantom empty row', () => {
  assert.equal(keyTarget('ArrowUp', 0, 3, 1), 0);
  assert.equal(keyTarget('ArrowDown', 0, 3, 1), 1);
  assert.equal(keyTarget('ArrowDown', 2, 3, 1), 2);
  assert.equal(keyTarget('ArrowDown', 0, 0, 1), null);
});
test('shared keyboard stops step dated daily cells and respect the playhead', () => {
  const periods = [
    { firstHour: 0, lastHour: 23, readHour: 23 },
    { firstHour: 24, lastHour: 47, readHour: null },
    { firstHour: 72, lastHour: 95, readHour: 73 },
    { firstHour: 96, lastHour: 119, readHour: 119 },
  ];
  const navigation = heatNavigation(periods);
  assert.deepEqual(navigation.keyHours, [23, 73, 119]);
  assert.equal(navigation.snap(5), 23);
  assert.equal(navigation.snap(80), 73);
  assert.equal(navigation.snap(30), 30);
  assert.equal(navigation.snap(55), 55);
  const stops = keyStopList(navigation.keyHours, [], 0, 80)!;
  assert.equal(keyedStop('Home', null, stops)?.at, 23);
  assert.equal(keyedStop('ArrowRight', { at: 23 }, stops)?.at, 73);
  assert.equal(keyedStop('ArrowLeft', { at: 73 }, stops)?.at, 23);
  assert.equal(keyedStop('End', null, stops)?.at, 73);
});
test('categorical date axis hit zone is at the top and labels remain the row zone', () => {
  const frame = plotFrame(600, 300, { left: 100, right: 10, top: 24, bottom: 10 }, [0, 100], [0, 3], false, 0, true);
  assert.equal(hitZone({ x: 200, y: 10 }, frame), 'x-axis');
  assert.equal(hitZone({ x: 90, y: 30 }, frame), 'y-axis');
  assert.equal(hitZone({ x: 200, y: 295 }, frame), null);
});
test('equal cells share drawing runs without filling gaps or using future reads', () => {
  const row = { id: 'r', label: 'R', cells: Array.from({ length: 4 }, () => ({ parts })) };
  const periods = [0, 1, 3, 4].map(day => ({ firstHour: day * 24, lastHour: day * 24 + 23, readHour: day * 24 + 23 }));
  assert.deepEqual(cellRuns(row, periods, 100).map(run => run.period), [
    { firstHour: 0, lastHour: 47, readHour: 47 }, { firstHour: 72, lastHour: 95, readHour: 95 },
  ]);
  assert.equal(cellRuns(row, periods, null).length, 2);
  row.cells[1] = { parts: [...parts] };
  assert.equal(cellRuns(row, periods, null).length, 3);
  const approximate = { id: 'r', label: 'R', cells: [{ parts }, { parts, approximate: true }] };
  assert.equal(cellRuns(approximate, periods.slice(0, 2), null).length, 2, 'an approximate cell never joins an exact one');
});


test('filter exclusion cannot merge into blank inactivity', () => {
  const parts: [] = [];
  const rows = { id: 'mod', label: 'Mod region', cells: [{ parts }, { parts, filtered: true }] };
  const periods = [0, 1].map(day => ({ firstHour: day * 24, lastHour: day * 24 + 23, readHour: day * 24 + 23 }));
  assert.equal(cellRuns(rows, periods, null).length, 2);
});

test('heat scroll geometry reaches all 5,000 rows with bounded pixels at any Size', () => {
  for (const rowHeight of [13, 26]) {
    const top = 22, bottom = 8, height = 260;
    for (const scroll of [0, 2000 * rowHeight + 5, 5000 * rowHeight]) {
      const window = heatWindow(5000, scroll, height, rowHeight, top, bottom);
      assert.ok(window.plotHeight <= height + 5 * rowHeight);
      assert.ok(window.end - window.first <= Math.ceil(height / rowHeight) + 4);
      const firstVisible = heatRowAt(top + 1, window, top, 5000);
      assert.equal(firstVisible, Math.floor(Math.min(scroll, window.total - height) / rowHeight));
    }
    const end = heatWindow(5000, Infinity, height, rowHeight, top, bottom);
    assert.equal(end.end, 5000);
    assert.equal(heatRowAt(height - bottom - 1, end, top, 5000), 4999);
  }
});
