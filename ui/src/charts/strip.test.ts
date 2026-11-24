import test from 'node:test';
import assert from 'node:assert/strict';
import { keyedHour, plotFrame } from './frame.ts';
import { keyedStop, keyStopList } from './hover.ts';
import { barStrip, stripBarAt, stripColumns, stripKeyTargets, stripMiddle, stripRects } from './strip.ts';

const FRAME = plotFrame(400, 260, { left: 40, right: 10, top: 10, bottom: 100 }, [1000, 2000], [0, 100], false, 60);


test('wide bars keep their width with a pixel between them; narrow ones share columns keeping the largest', () => {
  const toX = (hour: number) => hour / 10;
  const wide = stripColumns([{ firstHour: 0, lastHour: 100, up: 1, down: 2 }, { firstHour: 100, lastHour: 120, up: 3, down: 0 }], toX, 0, 1000);
  assert.deepEqual(wide, [{ x: 0, width: 9, up: 1, down: 2, upBar: 0, downBar: 0 }, { x: 10, width: 2, up: 3, down: 0, upBar: 1, downBar: 1 }]);
  const narrow = stripColumns([
    { firstHour: 0, lastHour: 5, up: 1, down: 4 },
    { firstHour: 5, lastHour: 9, up: 3, down: 1 },
    { firstHour: 10, lastHour: 15, up: 2, down: 2 },
  ], toX, 0, 1000);
  assert.deepEqual(narrow, [{ x: 0, width: 1, up: 3, down: 4, upBar: 1, downBar: 0 }, { x: 1, width: 1, up: 2, down: 2, upBar: 2, downBar: 2 }]);
  assert.deepEqual(stripColumns([{ firstHour: 0, lastHour: 100, up: 1, down: 1 }, { firstHour: 200, lastHour: 300, up: 1, down: 1 }], toX, 50, 150),
    [{ x: 5, width: 4, up: 1, down: 1, upBar: 0, downBar: 0 }]);
  assert.deepEqual(stripColumns([{ firstHour: 0, lastHour: 50, up: 9, down: 9 }, { firstHour: 50, lastHour: 100, up: 1, down: 1 }], toX, 50, 150),
    [{ x: 5, width: 4, up: 1, down: 1, upBar: 1, downBar: 1 }]);
});

test('the keyboard hover steps through the stops shown, starting at the latest', () => {
  const stops = [1000, 1024, 1048, 1072];
  assert.equal(keyedHour('ArrowLeft', null, [1000, 2000], null, stops), 1072);
  assert.equal(keyedHour('ArrowLeft', 1072, [1000, 2000], null, stops), 1048);
  assert.equal(keyedHour('ArrowRight', 1072, [1000, 2000], null, stops), 1072);
  assert.equal(keyedHour('Home', 1072, [1000, 2000], null, stops), 1000);
  assert.equal(keyedHour('ArrowLeft', 1000, [1000, 2000], null, stops), 1000);
  assert.equal(keyedHour('End', null, [1000, 2000], 1050, stops), 1048);
  assert.equal(keyedHour('ArrowRight', 1030, [1010, 2000], 1050, stops), 1048);
  assert.equal(keyedHour('ArrowLeft', 1030, [1030, 2000], 1050, stops), 1030);
  assert.equal(keyedHour('ArrowRight', 1030, [1030, 2000], 1050, stops), 1048);
  assert.equal(keyedHour('End', null, [1100, 2000], null, stops), 1100);
  assert.equal(keyedHour('End', null, [1000, 2000], 900, stops), null);
  assert.equal(keyedHour('ArrowRight', null, [1000.5, 1100], 1050.5, undefined), 1050);
  assert.equal(keyedHour('ArrowLeft', 1050, [1000.5, 1100], 1050.5, undefined), 1026);
  assert.equal(keyedHour('ArrowLeft', 1010, [1000.5, 1100], 1050.5, undefined), 1000);
});

test('a strip hover is about the bar drawn under the pointer, in the half it points at', () => {
  const bars = [
    { firstHour: 1100, lastHour: 1101, up: 1000, down: 1 },
    { firstHour: 1101, lastHour: 1102, up: 1, down: 1 },
    { firstHour: 1102, lastHour: 1103, up: 1, down: 500 },
  ];
  const x = FRAME.toX(1102.5);
  const above = stripMiddle(FRAME.strip!) - 5;
  const below = stripMiddle(FRAME.strip!) + 5;
  assert.equal(stripBarAt(bars, FRAME, null, { x, y: above }), bars[0]);
  assert.equal(stripBarAt(bars, FRAME, null, { x, y: below }), bars[2]);
  assert.equal(stripBarAt(bars, FRAME, 1102, { x, y: below }), bars[1]);
  assert.equal(stripBarAt(bars, FRAME, null, { x, y: 100 }), null);
  assert.equal(stripBarAt(bars, FRAME, null, { x: FRAME.toX(1500), y: above }), null);
  const strip = barStrip(bars, { height: () => 60, upLabel: 'gain', downLabel: 'burn' });
  assert.equal(strip.describe!({ x, y: above }, FRAME, null), 1101);
  assert.equal(strip.describe!({ x, y: below }, FRAME, null), 1103);
  const zoomed = plotFrame(400, 260, { left: 40, right: 10, top: 10, bottom: 100 }, [990, 1016], [0, 100], false, 60);
  const day = barStrip([{ firstHour: 976, lastHour: 1000, up: 1, down: 1 }, { firstHour: 1000, lastHour: 1024, up: 2, down: 2 }], { height: () => 60, upLabel: 'gain', downLabel: 'burn' });
  assert.equal(day.describe!({ x: zoomed.toX(1012), y: above }, zoomed, null), 1024);
  assert.equal(day.describe!({ x: zoomed.toX(995), y: below }, zoomed, null), 1000);
  assert.equal(day.describe!({ x: zoomed.toX(1012), y: above }, zoomed, 1020), null);
  const from1000 = plotFrame(400, 260, { left: 40, right: 10, top: 10, bottom: 100 }, [1000, 1100], [0, 100], false, 60);
  assert.equal(day.describe!({ x: from1000.toX(1000) + 0.2, y: above }, from1000, null), 1024);
  const gap = from1000.toX(1024) - 0.5;
  assert.equal(day.describe!({ x: gap, y: above }, from1000, null), null);
  assert.equal(day.describe!({ x: gap - 1, y: above }, from1000, null), 1024);
});

test('where a shared column overlaps the bar drawn before it, the hover is about the one drawn on top', () => {
  const bars = [{ firstHour: 1090, lastHour: 1101, up: 1, down: 1 }, { firstHour: 1101, lastHour: 1102, up: 7, down: 7 }];
  const shown = stripColumns(bars, FRAME.toX, FRAME.x[0], FRAME.x[1]);
  assert.deepEqual(shown.map(column => [column.x, column.upBar]), [[FRAME.toX(1090), 0], [75, 1]]);
  assert.equal(stripBarAt(bars, FRAME, null, { x: 75.1, y: stripMiddle(FRAME.strip!) - 5 }), bars[1]);
});

test('a hover is about the drawn rectangle under it, so the exposed part of a taller bar stays its own', () => {
  const view = plotFrame(400, 260, { left: 40, right: 10, top: 10, bottom: 100 }, [995, 4495], [0, 100], false, 60);
  const bars = [{ firstHour: 4464, lastHour: 4488, up: 100, down: 100 }, { firstHour: 4488, lastHour: 4512, up: 1, down: 1 }];
  const columns = stripColumns(bars, view.toX, view.x[0], view.x[1]);
  assert.equal(columns.length, 2);
  const shared = columns[1].x + 0.2;
  assert.ok(shared >= columns[0].x && shared < columns[0].x + columns[0].width);
  const middle = stripMiddle(view.strip!);
  assert.equal(stripBarAt(bars, view, null, { x: shared, y: middle - 20 }), bars[0]);
  assert.equal(stripBarAt(bars, view, null, { x: shared, y: middle + 20 }), bars[0]);
  const tiny = stripRects(columns, view.strip!).find(rect => rect.bar === 1 && rect.half === 'up')!;
  assert.equal(stripBarAt(bars, view, null, { x: shared, y: tiny.y + tiny.height / 2 }), bars[1]);
  const alone = [{ firstHour: 4000, lastHour: 4100, up: 100, down: 100 }, { firstHour: 4100, lastHour: 4200, up: 1, down: 1 }];
  assert.equal(stripBarAt(alone, view, null, { x: view.toX(4150), y: middle - 20 }), alone[1]);
  assert.deepEqual(stripRects(columns, view.strip!).map(rect => [rect.bar, rect.half]), [[0, 'up'], [0, 'down'], [1, 'up'], [1, 'down']]);
});

test('the keys reach every strip bar drawn in view, a cut one at the view end for its read', () => {
  const bars = [
    { firstHour: 1288, lastHour: 1312, up: 1, down: 1 },
    { firstHour: 1312, lastHour: 1336, up: 1, down: 1 },
    { firstHour: 1336, lastHour: 1360, up: 1, down: 1 },
  ];
  const view = [1000, 1340] as const;
  const targets = stripKeyTargets(bars, view, null);
  assert.deepEqual(targets, [{ at: 1312, describes: 1312 }, { at: 1336, describes: 1336 }, { at: 1340, describes: 1360 }]);
  const stops = keyStopList([1312, 1336, 1360], targets, view[0], view[1])!;
  assert.deepEqual(stops.map(stop => [stop.at, stop.describes]), [[1312, 1312], [1336, 1336], [1340, 1360]]);
  assert.deepEqual(keyedStop('End', null, stops), stops[2]);
  assert.deepEqual(keyedStop('ArrowRight', stops[1], stops), stops[2]);
  assert.deepEqual(keyedStop('ArrowLeft', stops[2], stops), stops[1]);
  assert.deepEqual(stripKeyTargets(bars, view, 1350), [{ at: 1312, describes: 1312 }, { at: 1336, describes: 1336 }]);
  assert.deepEqual(stripKeyTargets(bars, [1360, 1400], null), []);
  assert.deepEqual(stripKeyTargets(bars, [1330, 1400], null), [{ at: 1336, describes: 1336 }, { at: 1360, describes: 1360 }]);
  assert.deepEqual(barStrip(bars, { height: () => 60, upLabel: 'gain', downLabel: 'burn' }).keyTargets!(view, null), targets);
  assert.equal(keyStopList(undefined, [], 1000, 1340), null);
});

test('two keyboard stops at one hour are both reachable, the chart\'s own first', () => {
  const bars = [
    { firstHour: 1289, lastHour: 1313, up: 1, down: 1 },
    { firstHour: 1313, lastHour: 1337, up: 1, down: 1 },
    { firstHour: 1337, lastHour: 1361, up: 1, down: 1 },
  ];
  const view = [1000, 1341] as const;
  const stops = keyStopList([1313, 1337, 1341, 1361], stripKeyTargets(bars, view, null), view[0], view[1])!;
  assert.deepEqual(stops.map(stop => [stop.at, stop.describes]), [[1313, 1313], [1337, 1337], [1341, 1341], [1341, 1361]]);
  assert.deepEqual(keyedStop('End', null, stops), stops[3]);
  assert.deepEqual(keyedStop('ArrowLeft', stops[3], stops), stops[2]);
  assert.deepEqual(keyedStop('ArrowRight', stops[1], stops), stops[2]);
  assert.deepEqual(keyedStop('ArrowRight', stops[2], stops), stops[3]);
  assert.deepEqual(keyedStop('ArrowRight', stops[3], stops), stops[3]);
  assert.deepEqual(keyedStop('ArrowRight', { at: 1337 }, stops), stops[2]);
  assert.deepEqual(keyedStop('ArrowLeft', { at: 1341 }, stops), stops[1]);
  assert.equal(keyedStop('ArrowLeft', { at: 1341 }, []), null);
  assert.deepEqual(keyStopList([1313, 1337], [], 1320, 1341)!.map(stop => stop.at), [1320, 1337]);
});


test('a span bar is drawn and hovered over its hours but names no hour: the hover is about the hour pointed at', () => {
  const bars = [{ firstHour: 1099, lastHour: 1195, up: 10, down: 10, span: true as const }, { firstHour: 1195, lastHour: 1219, up: 5, down: 5 }];
  const strip = barStrip(bars, { height: () => 60, upLabel: 'gain', downLabel: 'burn' });
  const above = stripMiddle(FRAME.strip!) - 5;
  assert.equal(stripBarAt(bars, FRAME, null, { x: FRAME.toX(1150), y: above }), bars[0]);
  assert.equal(strip.describe!({ x: FRAME.toX(1150), y: above }, FRAME, null), null);
  assert.equal(strip.describe!({ x: FRAME.toX(1210), y: above }, FRAME, null), 1219);
  assert.deepEqual(stripKeyTargets(bars, [1000, 2000], null).map(target => target.describes), [1219]);
});
