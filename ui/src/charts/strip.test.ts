import test from 'node:test';
import assert from 'node:assert/strict';
import { chartBottom, hitZone, keyedHour, plotFrame } from './frame.ts';
import { stripColumns } from './strip.ts';

const FRAME = plotFrame(400, 260, { left: 40, right: 10, top: 10, bottom: 100 }, [1000, 2000], [0, 100], false, 60);

test('a strip sits between the plot and the dates, and has its own hit zone', () => {
  assert.equal(FRAME.plotHeight, 150);
  assert.deepEqual(FRAME.strip, { top: 170, height: 60 });
  assert.equal(chartBottom(FRAME), 230);
  assert.equal(hitZone({ x: 200, y: 100 }, FRAME), 'plot');
  // The gap above the strip belongs to it, so the pointer never falls between them.
  assert.equal(hitZone({ x: 200, y: 165 }, FRAME), 'strip');
  assert.equal(hitZone({ x: 200, y: 229 }, FRAME), 'strip');
  assert.equal(hitZone({ x: 200, y: 240 }, FRAME), 'x-axis');
  assert.equal(hitZone({ x: 20, y: 200 }, FRAME), null);
  const plain = plotFrame(400, 260, { left: 40, right: 10, top: 10, bottom: 100 }, [1000, 2000], [0, 100], false);
  assert.equal(plain.strip, null);
  assert.equal(chartBottom(plain), 160);
  assert.equal(hitZone({ x: 200, y: 200 }, plain), 'x-axis');
});

test('wide bars keep their width with a pixel between them; narrow ones share columns keeping the largest', () => {
  const toX = (hour: number) => hour / 10;
  const wide = stripColumns([{ firstHour: 0, lastHour: 100, up: 1, down: 2 }, { firstHour: 100, lastHour: 120, up: 3, down: 0 }], toX, 0, 1000);
  assert.deepEqual(wide, [{ x: 0, width: 9, up: 1, down: 2 }, { x: 10, width: 2, up: 3, down: 0 }]);
  const narrow = stripColumns([
    { firstHour: 0, lastHour: 5, up: 1, down: 4 },
    { firstHour: 5, lastHour: 9, up: 3, down: 1 },
    { firstHour: 10, lastHour: 15, up: 2, down: 2 },
  ], toX, 0, 1000);
  assert.deepEqual(narrow, [{ x: 0, width: 1, up: 3, down: 4 }, { x: 1, width: 1, up: 2, down: 2 }]);
  // Bars outside the view are left out and a bar across its edge is cut to it.
  assert.deepEqual(stripColumns([{ firstHour: 0, lastHour: 100, up: 1, down: 1 }, { firstHour: 200, lastHour: 300, up: 1, down: 1 }], toX, 50, 150),
    [{ x: 5, width: 4, up: 1, down: 1 }]);
});

test('the keyboard hover steps through the stops shown, starting at the latest', () => {
  const stops = [1000, 1024, 1048, 1072];
  assert.equal(keyedHour('ArrowLeft', null, [1000, 2000], null, stops), 1072);
  assert.equal(keyedHour('ArrowLeft', 1072, [1000, 2000], null, stops), 1048);
  assert.equal(keyedHour('ArrowRight', 1072, [1000, 2000], null, stops), 1072);
  assert.equal(keyedHour('Home', 1072, [1000, 2000], null, stops), 1000);
  assert.equal(keyedHour('ArrowLeft', 1000, [1000, 2000], null, stops), 1000);
  // Never after the playhead or outside the view.
  assert.equal(keyedHour('End', null, [1000, 2000], 1050, stops), 1048);
  assert.equal(keyedHour('ArrowRight', 1030, [1010, 2000], 1050, stops), 1048);
  // A view starting between stops can stop at its start, where the value read at 1024 is carried.
  assert.equal(keyedHour('ArrowLeft', 1030, [1030, 2000], 1050, stops), 1030);
  assert.equal(keyedHour('ArrowRight', 1030, [1030, 2000], 1050, stops), 1048);
  assert.equal(keyedHour('End', null, [1100, 2000], null, stops), 1100);
  assert.equal(keyedHour('End', null, [1000, 2000], 900, stops), null);
  // Without stops it steps a day inside the view.
  assert.equal(keyedHour('ArrowRight', null, [1000.5, 1100], 1050.5, undefined), 1050);
  assert.equal(keyedHour('ArrowLeft', 1050, [1000.5, 1100], 1050.5, undefined), 1026);
  assert.equal(keyedHour('ArrowLeft', 1010, [1000.5, 1100], 1050.5, undefined), 1000);
});
