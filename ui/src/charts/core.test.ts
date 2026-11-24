import test from 'node:test';
import assert from 'node:assert/strict';
import { fitLabelWidth, layoutEndLabels } from './labels.ts';
import { FrameScheduler } from './scheduler.ts';
import type { FrameTask } from './scheduler.ts';
import { monotonePath, runAt, samplePoints, segments, stack, valueAt } from './series.ts';
import type { Run } from './series.ts';
import { backingSize, toLocal } from './surface.ts';
import { placeTooltip } from './tooltipPlacement.ts';
import { fixedNumber, formatFixed, linearTicks, logTicks, niceStep } from './valueAxis.ts';
import { EMPTY_VIEW, playbackDomain, reduceView, resolveX, resolveY, shownUntil } from './view.ts';
import type { ViewContext } from './view.ts';

const RUNS: Run[] = [
  { firstHour: 10, lastHour: 19, value: 5 },
  { firstHour: 20, lastHour: 29, value: 8 },
  { firstHour: 40, lastHour: 49, value: 2 },
];

test('runs answer by hour and gaps stay missing, never zero', () => {
  assert.equal(valueAt(RUNS, 15), 5);
  assert.equal(valueAt(RUNS, 29.9), 8);
  assert.equal(valueAt(RUNS, 35), null);
  assert.equal(runAt(RUNS, 9), null);
  assert.deepEqual(segments(RUNS, 0, 100).map(segment => segment.length), [2, 1]);
  assert.deepEqual(segments(RUNS, 25, 45).map(segment => [segment[0].firstHour, segment.at(-1)!.lastHour]), [[25, 29], [40, 45]]);
});

test('samples keep both ends and the monotone curve never overshoots its samples', () => {
  const points = samplePoints(segments(RUNS, 0, 100)[0], hour => hour * 10, value => 100 - value * 10, 12);
  assert.equal(points[0].x, 100);
  assert.equal(points.at(-1)!.x, 290);
  const zigzag = [{ x: 0, y: 0 }, { x: 10, y: 50 }, { x: 20, y: 40 }, { x: 30, y: 90 }, { x: 40, y: 90 }, { x: 50, y: 10 }];
  const path = monotonePath(zigzag);
  assert.equal(path[0][0], 'M');
  path.slice(1).forEach((command, index) => {
    assert.equal(command[0], 'C');
    const [low, high] = [Math.min(zigzag[index].y, zigzag[index + 1].y), Math.max(zigzag[index].y, zigzag[index + 1].y)];
    for (const y of [command[2], command[4]] as number[]) assert.ok(y >= low - 1e-9 && y <= high + 1e-9, `control ${y} outside ${low}..${high}`);
  });
  assert.deepEqual(stack([[1, 2], [3, null], [1, 1]]), [[1, 2], [4, null], [5, null]]);
});


const CONTEXT: ViewContext = { x: [0, 1000], extent: [0, 1000], y: [0, 100], minSpan: 14 * 24 };

test('axis drags keep the axis start fixed and move the grabbed value to the pointer', () => {
  const stretched = reduceView(EMPTY_VIEW, { type: 'axis-x', grab: 500, fraction: 0.25 }, { ...CONTEXT, extent: [0, 10_000] });
  assert.deepEqual(stretched.x, [0, 2000]);
  const shrunk = reduceView(EMPTY_VIEW, { type: 'axis-x', grab: 500, fraction: 1 }, CONTEXT);
  assert.deepEqual(shrunk.x, [0, 500]);
  const floor = reduceView(EMPTY_VIEW, { type: 'axis-x', grab: 10, fraction: 1 }, CONTEXT);
  assert.equal(floor.x![1] - floor.x![0], CONTEXT.minSpan);
  const tall = reduceView(EMPTY_VIEW, { type: 'axis-y', grab: 50, fraction: 0.25 }, CONTEXT);
  assert.deepEqual(tall.y, [0, 200]);
});

test('wheel, pan and box zoom stay inside the data, and resets clear only their axis', () => {
  const zoomed = reduceView(EMPTY_VIEW, { type: 'wheel', at: 500, steps: -4 }, CONTEXT);
  assert.ok(zoomed.x![0] > 0 && zoomed.x![1] < 1000 && zoomed.x![0] < 500 && zoomed.x![1] > 500);
  const panned = reduceView(zoomed, { type: 'pan', hours: 10_000 }, CONTEXT);
  assert.equal(panned.x![1], 1000);
  const boxed = reduceView(EMPTY_VIEW, { type: 'box', x: [700, 200], y: [80, 20] }, CONTEXT);
  assert.deepEqual(boxed, { x: [200, 700], y: [20, 80] });
  assert.deepEqual(reduceView(boxed, { type: 'reset-y' }, CONTEXT), { x: [200, 700], y: null });
  assert.deepEqual(reduceView(boxed, { type: 'reset' }, CONTEXT), EMPTY_VIEW);
});

test('a recent window paused or after a seek ends at the playhead, never past it; playing it holds its first length', () => {
  const fortnight = 14 * 24, year = 365 * 24, extent = [0, 10 * year] as const;
  assert.deepEqual(playbackDomain('window', extent, 0, year, fortnight, true), [0, 1]);
  assert.deepEqual(playbackDomain('window', extent, 50, year, fortnight, true), [0, 50]);
  assert.deepEqual(playbackDomain('window', extent, 2 * year, year, fortnight, true), [year, 2 * year]);
  assert.deepEqual(playbackDomain('window', extent, 50, 0, fortnight, true), [0, 50]);
  assert.deepEqual(playbackDomain('window', extent, 50, year, fortnight), [0, year]);
  assert.deepEqual(playbackDomain('window', extent, 2 * year, year, fortnight), [year, 2 * year]);
  assert.deepEqual(playbackDomain('draw-in', extent, 50, year, fortnight, true), [0, fortnight]);
});

test('playback domains: draw-in grows, cursor holds, window slides; zoom wins over playback', () => {
  assert.deepEqual(playbackDomain('draw-in', [0, 1000], 600, 0, 100), [0, 600]);
  assert.deepEqual(playbackDomain('draw-in', [0, 1000], 10, 0, 100), [0, 100]);
  assert.deepEqual(playbackDomain('cursor', [0, 1000], 600, 0, 100), [0, 1000]);
  assert.deepEqual(playbackDomain('window', [0, 1000], 600, 200, 100), [400, 600]);
  assert.deepEqual(playbackDomain('window', [0, 1000], 50, 200, 100), [0, 200]);
  assert.deepEqual(resolveX({ x: [5, 9], y: null }, { xFrom: 0, xTo: 100 }, 1000, [0, 1]), [5, 9]);
  assert.deepEqual(resolveX(EMPTY_VIEW, { xFrom: 10, xTo: null }, 1000, [1000, 1500]), [1010, 1500]);
  assert.deepEqual(resolveY(EMPTY_VIEW, { yMin: null, yMax: null, log: true }, [0, 100]).log, false);
  assert.deepEqual(resolveY(EMPTY_VIEW, { yMin: 5, yMax: 50, log: true }, [0, 100]), { range: [5, 50], log: true });
  const logFit = resolveY(EMPTY_VIEW, { yMin: null, yMax: null, log: true }, [10, 2000]);
  assert.equal(logFit.log, true);
  assert.ok(logFit.range[0] > 0 && logFit.range[0] < 10);
});

test('value ticks are nice and fixed-point values format exactly', () => {
  assert.equal(niceStep(100), 20);
  assert.equal(niceStep(7, 5), 2);
  assert.deepEqual(linearTicks(0, 100), [0, 20, 40, 60, 80, 100]);
  assert.deepEqual(linearTicks(-0.3, 0.3, 3), [-0.2, 0, 0.2]);
  assert.deepEqual(logTicks(1, 100), [1, 2, 5, 10, 20, 50, 100]);
  assert.deepEqual(logTicks(0, 100), []);
  assert.equal(formatFixed('1000001', 5, 2), '10.00');
  assert.equal(formatFixed('1234567890', 5, 1), '12,345.7');
  assert.equal(formatFixed('-150000', 5, 0), '−2');
  assert.equal(formatFixed('-49', 5, 2), '0.00');
  assert.equal(formatFixed(12n, 0, 2), '12.00');
  assert.equal(fixedNumber('250000', 5), 2.5);
});

test('end labels keep their order without overlapping and stay in bounds', () => {
  const placed = layoutEndLabels([{ id: 'a', y: 50, height: 14 }, { id: 'b', y: 52, height: 14 }, { id: 'c', y: 198, height: 14 }], 0, 200);
  assert.ok(placed.get('b')! - placed.get('a')! >= 14);
  assert.ok(placed.get('c')! <= 193);
  assert.equal(fitLabelWidth(['ab', 'abcd'], text => text.length * 10, 20, 200, 6), 46);
  assert.equal(fitLabelWidth(['x'.repeat(100)], text => text.length * 10, 20, 200), 200);
  const measured: string[] = [];
  const width = fitLabelWidth(['ab', 'x'.repeat(30), 'abcd', 'y'.repeat(40)], text => { measured.push(text); return text.length * 10; }, 20, 200, 6);
  assert.equal(width, 200);
  assert.deepEqual(measured, ['ab', 'x'.repeat(30)]);
});

test('canvases follow on-screen size, so the Size zoom stays sharp', () => {
  const at200 = backingSize({ layoutWidth: 300, layoutHeight: 100, rectWidth: 600, rectHeight: 200, devicePixelRatio: 1.5 });
  assert.deepEqual(at200, { width: 900, height: 300, scaleX: 3, scaleY: 3 });
  const exact = backingSize({ layoutWidth: 300, layoutHeight: 100, rectWidth: 600, rectHeight: 200, devicePixelRatio: 1.5, devicePixelWidth: 901, devicePixelHeight: 299 });
  assert.equal(exact.width, 901);
  assert.deepEqual(toLocal(150, 60, { left: 100, top: 50, width: 600, height: 200 }, 300, 100), { x: 25, y: 5 });
});

test('tooltips flip away from the window edge and are clamped inside it', () => {
  const viewport = { width: 1000, height: 800 };
  assert.deepEqual(placeTooltip({ x: 100, y: 100 }, { width: 200, height: 100 }, viewport), { x: 114, y: 114 });
  assert.deepEqual(placeTooltip({ x: 950, y: 750 }, { width: 200, height: 100 }, viewport), { x: 736, y: 636 });
  assert.deepEqual(placeTooltip({ x: 10, y: 10 }, { width: 2000, height: 100 }, viewport).x, 8);
});

test('the scheduler draws only woken, visible tasks and stops when nothing moves', () => {
  let queued: ((now: number) => void) | null = null;
  const scheduler = new FrameScheduler({ request: callback => { queued = callback; return 1; } });
  const run = (now: number) => { const callback = queued!; queued = null; callback(now); };
  let frames = 0;
  let moving = 2;
  const task: FrameTask = { frame: () => { frames++; return --moving > 0; } };
  const remove = scheduler.add(task);
  run(16); run(32);
  assert.equal(frames, 2);
  assert.equal(queued, null);
  scheduler.setVisible(task, false);
  scheduler.wake(task);
  assert.equal(queued, null);
  scheduler.setVisible(task, true);
  moving = 1;
  run(48);
  assert.equal(frames, 3);
  remove();
  scheduler.wake(task);
  assert.equal(queued, null);
});

test('whole-campaign playback stops at the playhead unless set to show past it', () => {
  assert.equal(shownUntil(500, 'cursor', false), 500);
  assert.equal(shownUntil(500, 'cursor', true), null);
  assert.equal(shownUntil(500, 'draw-in', true), 500);
  assert.equal(shownUntil(500, 'window', true), 500);
  assert.equal(shownUntil(null, 'cursor', false), null);
});
