import test from 'node:test';
import assert from 'node:assert/strict';
import { chartBottom, hitZone, keyedHour, plotFrame } from './frame.ts';
import { keyedStop, keyStopList, keyStopMemo } from './hover.ts';
import { barStrip, stripBarAt, stripColumns, stripKeyTargets, stripMiddle, stripRects } from './strip.ts';

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
  assert.deepEqual(wide, [{ x: 0, width: 9, up: 1, down: 2, upBar: 0, downBar: 0 }, { x: 10, width: 2, up: 3, down: 0, upBar: 1, downBar: 1 }]);
  const narrow = stripColumns([
    { firstHour: 0, lastHour: 5, up: 1, down: 4 },
    { firstHour: 5, lastHour: 9, up: 3, down: 1 },
    { firstHour: 10, lastHour: 15, up: 2, down: 2 },
  ], toX, 0, 1000);
  // A shared column draws each half's largest, and names the bar it comes from.
  assert.deepEqual(narrow, [{ x: 0, width: 1, up: 3, down: 4, upBar: 1, downBar: 0 }, { x: 1, width: 1, up: 2, down: 2, upBar: 2, downBar: 2 }]);
  // Bars outside the view are left out and a bar across its edge is cut to it.
  assert.deepEqual(stripColumns([{ firstHour: 0, lastHour: 100, up: 1, down: 1 }, { firstHour: 200, lastHour: 300, up: 1, down: 1 }], toX, 50, 150),
    [{ x: 5, width: 4, up: 1, down: 1, upBar: 0, downBar: 0 }]);
  // A bar that only touches the view (ending where it starts) has no column.
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

test('a strip hover is about the bar drawn under the pointer, in the half it points at', () => {
  // Hour-long bars share one pixel column (0.35 px an hour): 1000 gained is drawn from
  // the first, 500 burned from the third; the second, 1 and 1, is drawn nowhere.
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
  // A bar ending after the last hour drawn is not drawn, so the column holds the others.
  assert.equal(stripBarAt(bars, FRAME, 1102, { x, y: below }), bars[1]);
  // Over the plot, between columns or in the gap after a wide bar, there is no bar.
  assert.equal(stripBarAt(bars, FRAME, null, { x, y: 100 }), null);
  assert.equal(stripBarAt(bars, FRAME, null, { x: FRAME.toX(1500), y: above }), null);
  const strip = barStrip(bars, { height: () => 60, upLabel: 'gain', downLabel: 'burn' });
  assert.equal(strip.describe!({ x, y: above }, FRAME, null), 1101);
  assert.equal(strip.describe!({ x, y: below }, FRAME, null), 1103);
  // A day cut off by a zoom that ends at 1016 is still about its read at 1024.
  const zoomed = plotFrame(400, 260, { left: 40, right: 10, top: 10, bottom: 100 }, [990, 1016], [0, 100], false, 60);
  const day = barStrip([{ firstHour: 976, lastHour: 1000, up: 1, down: 1 }, { firstHour: 1000, lastHour: 1024, up: 2, down: 2 }], { height: () => 60, upLabel: 'gain', downLabel: 'burn' });
  assert.equal(day.describe!({ x: zoomed.toX(1012), y: above }, zoomed, null), 1024);
  assert.equal(day.describe!({ x: zoomed.toX(995), y: below }, zoomed, null), 1000);
  assert.equal(day.describe!({ x: zoomed.toX(1012), y: above }, zoomed, 1020), null);
  // A view starting at 1000: the day ending there is not drawn, so the first pixel is the next day's.
  const from1000 = plotFrame(400, 260, { left: 40, right: 10, top: 10, bottom: 100 }, [1000, 1100], [0, 100], false, 60);
  assert.equal(day.describe!({ x: from1000.toX(1000) + 0.2, y: above }, from1000, null), 1024);
  // The pixel of space after a wide bar draws nothing, so it describes nothing.
  const gap = from1000.toX(1024) - 0.5;
  assert.equal(day.describe!({ x: gap, y: above }, from1000, null), null);
  assert.equal(day.describe!({ x: gap - 1, y: above }, from1000, null), 1024);
});

test('where a shared column overlaps the bar drawn before it, the hover is about the one drawn on top', () => {
  // A wide bar ends at 75.35 px; the next, hour-long bars share the column drawn from pixel 75.
  const bars = [{ firstHour: 1090, lastHour: 1101, up: 1, down: 1 }, { firstHour: 1101, lastHour: 1102, up: 7, down: 7 }];
  const shown = stripColumns(bars, FRAME.toX, FRAME.x[0], FRAME.x[1]);
  assert.deepEqual(shown.map(column => [column.x, column.upBar]), [[FRAME.toX(1090), 0], [75, 1]]);
  assert.equal(stripBarAt(bars, FRAME, null, { x: 75.1, y: stripMiddle(FRAME.strip!) - 5 }), bars[1]);
});

test('a hover is about the drawn rectangle under it, so the exposed part of a taller bar stays its own', () => {
  // A tall day and a tiny day cut by the view share a pixel: the tiny one's column is drawn
  // over the tall one's, but only as high as its own amount.
  // 0.1 px an hour: the tall day is 2.4 px (no space after it), the tiny one cut to 0.7 px.
  const view = plotFrame(400, 260, { left: 40, right: 10, top: 10, bottom: 100 }, [995, 4495], [0, 100], false, 60);
  const bars = [{ firstHour: 4464, lastHour: 4488, up: 100, down: 100 }, { firstHour: 4488, lastHour: 4512, up: 1, down: 1 }];
  const columns = stripColumns(bars, view.toX, view.x[0], view.x[1]);
  assert.equal(columns.length, 2);
  const shared = columns[1].x + 0.2;
  // The point lies in both drawn columns.
  assert.ok(shared >= columns[0].x && shared < columns[0].x + columns[0].width);
  const middle = stripMiddle(view.strip!);
  // High above the midline only the tall day is drawn there.
  assert.equal(stripBarAt(bars, view, null, { x: shared, y: middle - 20 }), bars[0]);
  assert.equal(stripBarAt(bars, view, null, { x: shared, y: middle + 20 }), bars[0]);
  // On the tiny day's own rectangle, drawn on top, it is the tiny day.
  const tiny = stripRects(columns, view.strip!).find(rect => rect.bar === 1 && rect.half === 'up')!;
  assert.equal(stripBarAt(bars, view, null, { x: shared, y: tiny.y + tiny.height / 2 }), bars[1]);
  // Above a short bar but inside its column, the hover is still about that bar.
  const alone = [{ firstHour: 4000, lastHour: 4100, up: 100, down: 100 }, { firstHour: 4100, lastHour: 4200, up: 1, down: 1 }];
  assert.equal(stripBarAt(alone, view, null, { x: view.toX(4150), y: middle - 20 }), alone[1]);
  // The rectangles are those drawStrip fills: each column's gain, then its burn.
  assert.deepEqual(stripRects(columns, view.strip!).map(rect => [rect.bar, rect.half]), [[0, 'up'], [0, 'down'], [1, 'up'], [1, 'down']]);
});

test('the keys reach every strip bar drawn in view, a cut one at the view end for its read', () => {
  // Daily bars read at 1312, 1336 and 1360; a zoom to [1000, 1340] draws the last
  // one's first four hours, and a pointer over them is about its read at 1360.
  const bars = [
    { firstHour: 1288, lastHour: 1312, up: 1, down: 1 },
    { firstHour: 1312, lastHour: 1336, up: 1, down: 1 },
    { firstHour: 1336, lastHour: 1360, up: 1, down: 1 },
  ];
  const view = [1000, 1340] as const;
  const targets = stripKeyTargets(bars, view, null);
  assert.deepEqual(targets, [{ at: 1312, describes: 1312 }, { at: 1336, describes: 1336 }, { at: 1340, describes: 1360 }]);
  const stops = keyStopList([1312, 1336, 1360], targets, view[0], view[1])!;
  // A bar read in view is the read's own stop, not a second one.
  assert.deepEqual(stops.map(stop => [stop.at, stop.describes]), [[1312, 1312], [1336, 1336], [1340, 1360]]);
  // End and ArrowRight reach the cut bar at the view's end, about its read.
  assert.deepEqual(keyedStop('End', null, stops), stops[2]);
  assert.deepEqual(keyedStop('ArrowRight', stops[1], stops), stops[2]);
  assert.deepEqual(keyedStop('ArrowLeft', stops[2], stops), stops[1]);
  // A bar read after the last hour drawn is not drawn, so it is no stop.
  assert.deepEqual(stripKeyTargets(bars, view, 1350), [{ at: 1312, describes: 1312 }, { at: 1336, describes: 1336 }]);
  // A bar that only touches the view has none; a view starting inside a bar keeps it.
  assert.deepEqual(stripKeyTargets(bars, [1360, 1400], null), []);
  assert.deepEqual(stripKeyTargets(bars, [1330, 1400], null), [{ at: 1336, describes: 1336 }, { at: 1360, describes: 1360 }]);
  assert.deepEqual(barStrip(bars, { height: () => 60, upLabel: 'gain', downLabel: 'burn' }).keyTargets!(view, null), targets);
  // Without key hours or targets the keys step a day.
  assert.equal(keyStopList(undefined, [], 1000, 1340), null);
});

test('two keyboard stops at one hour are both reachable, the chart\'s own first', () => {
  // Daily reads at 1313, 1337 and 1361 with bars before each, and a refresh read at
  // 1341 with no bar of its own. A zoom to [1000, 1341] cuts the 1361 bar at 1341,
  // the refresh's hour.
  const bars = [
    { firstHour: 1289, lastHour: 1313, up: 1, down: 1 },
    { firstHour: 1313, lastHour: 1337, up: 1, down: 1 },
    { firstHour: 1337, lastHour: 1361, up: 1, down: 1 },
  ];
  const view = [1000, 1341] as const;
  const stops = keyStopList([1313, 1337, 1341, 1361], stripKeyTargets(bars, view, null), view[0], view[1])!;
  assert.deepEqual(stops.map(stop => [stop.at, stop.describes]), [[1313, 1313], [1337, 1337], [1341, 1341], [1341, 1361]]);
  // End reaches the visible bar's read; ArrowLeft then shows the refresh at the same hour.
  assert.deepEqual(keyedStop('End', null, stops), stops[3]);
  assert.deepEqual(keyedStop('ArrowLeft', stops[3], stops), stops[2]);
  // From 1337, one ArrowRight shows the refresh and one more the bar's read.
  assert.deepEqual(keyedStop('ArrowRight', stops[1], stops), stops[2]);
  assert.deepEqual(keyedStop('ArrowRight', stops[2], stops), stops[3]);
  assert.deepEqual(keyedStop('ArrowRight', stops[3], stops), stops[3]);
  // A hover the keys did not set (the pointer's, at an hour) steps on from its hour.
  assert.deepEqual(keyedStop('ArrowRight', { at: 1337 }, stops), stops[2]);
  assert.deepEqual(keyedStop('ArrowLeft', { at: 1341 }, stops), stops[1]);
  assert.equal(keyedStop('ArrowLeft', { at: 1341 }, []), null);
  // A value carried into a view that starts between reads still has its stop.
  assert.deepEqual(keyStopList([1313, 1337], [], 1320, 1341)!.map(stop => stop.at), [1320, 1337]);
});

/** Whether building takes about linear time (up to a log): the best of nine runs at
 * `n` and at 4n, after warming up, so a slow machine does not fail it but a
 * quadratic cost (16 times as long) does. */
function assertLinear(make: (count: number) => () => unknown) {
  const best = (run: () => unknown) => {
    for (let round = 0; round < 3; round++) run();
    let fastest = Infinity;
    for (let round = 0; round < 9; round++) {
      const started = performance.now();
      run();
      fastest = Math.min(fastest, performance.now() - started);
    }
    return fastest;
  };
  const small = best(make(5_000));
  const large = best(make(20_000));
  assert.ok(large < small * 8, `5,000 stops ${small.toFixed(2)} ms, 20,000 stops ${large.toFixed(2)} ms`);
}

test('keyboard stops for decades of daily reads are built in about linear time, once per change', () => {
  // A read and a bar a day, all in view: every bar's read is also a chart stop.
  const campaign = (days: number) => {
    const reads = Array.from({ length: days }, (_, day) => 1000 + day * 24);
    const bars = reads.map(read => ({ firstHour: read - 24, lastHour: read, up: 1, down: 1 }));
    return { reads, bars, view: [reads[0] - 24, reads.at(-1)!] as const };
  };
  assertLinear(days => {
    const { reads, bars, view } = campaign(days);
    return () => keyStopList(reads, stripKeyTargets(bars, view, null), view[0], view[1]);
  });
  const { reads, bars, view } = campaign(100);
  const stops = keyStopList(reads, stripKeyTargets(bars, view, null), view[0], view[1])!;
  assert.equal(stops.length, 100, 'a bar read in view is the read\'s own stop');
  // Memoised: the targets are asked for again only when an input changes.
  const memo = keyStopMemo();
  const strip = barStrip(bars, { height: () => 60, upLabel: 'gain', downLabel: 'burn' });
  let asked = 0;
  const targets = () => { asked++; return strip.keyTargets!(view, null); };
  const first = memo(reads, strip, targets, view, null);
  assert.deepEqual(first, stops);
  assert.equal(memo(reads, strip, targets, [view[0], view[1]], null), first);
  assert.equal(asked, 1);
  memo(reads, strip, targets, [view[0], view[1] - 24], null);
  memo(reads, strip, targets, [view[0], view[1] - 24], 2000);
  memo(reads.slice(), strip, targets, [view[0], view[1] - 24], 2000);
  assert.equal(asked, 4);
});
