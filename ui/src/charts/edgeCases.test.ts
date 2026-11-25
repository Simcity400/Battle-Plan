import test from 'node:test';
import assert from 'node:assert/strict';
import { parseColor, toHex } from '../shell/appearance/color.ts';
import { nudgeApart } from './palette.ts';
import { layoutEndLabels } from './labels.ts';
import { FrameScheduler } from './scheduler.ts';
import type { FrameTask } from './scheduler.ts';
import { timeTicks } from './timeAxis.ts';
import { linearTicks } from './valueAxis.ts';
import { EMPTY_VIEW, reduceView, resolveY } from './view.ts';

test('dragging a log value axis keeps the grabbed value under the pointer', () => {
  const context = { x: [0, 10] as const, extent: [0, 10] as const, y: [1, 10_000] as const, minSpan: 1, log: true };
  const view = reduceView(EMPTY_VIEW, { type: 'axis-y', grab: 100, fraction: 0.25 }, context);
  assert.equal(view.y![0], 1);
  assert.ok(Math.abs(Math.log10(view.y![1]) - 8) < 1e-9, `top ${view.y![1]}`);
  const linear = reduceView(EMPTY_VIEW, { type: 'axis-y', grab: 50, fraction: 0.25 }, { ...context, y: [0, 100], log: false });
  assert.deepEqual(linear.y, [0, 200]);
});

test('a wake that arrives while a task draws is not lost', () => {
  let queued: ((now: number) => void) | null = null;
  const scheduler = new FrameScheduler({ request: callback => { queued = callback; return 1; } });
  let frames = 0;
  const task: FrameTask = { frame: () => { frames++; if (frames === 1) scheduler.wake(task); return false; } };
  scheduler.add(task);
  const run = () => { const callback = queued!; queued = null; callback(frames * 16); };
  run();
  assert.notEqual(queued, null, 'the wake during the frame schedules another');
  run();
  assert.equal(frames, 2);
  assert.equal(queued, null);
});

test('a zoomed time range with fractional bounds still gets ticks', () => {
  const whole = timeTicks(61_320_000, 61_321_000, 600);
  const zoomed = timeTicks(61_320_000.4, 61_320_999.6, 600);
  assert.ok(whole.labels.length > 0);
  assert.equal(zoomed.level, whole.level);
  assert.ok(zoomed.labels.length > 0 && zoomed.grid.length > 0);
});

test('ticks on large values stay distinct', () => {
  assert.deepEqual(linearTicks(1_000_000_000_000, 1_000_000_000_004, 5),
    [1_000_000_000_000, 1_000_000_000_001, 1_000_000_000_002, 1_000_000_000_003, 1_000_000_000_004]);
  assert.deepEqual(linearTicks(0, 1, 5), [0, 0.2, 0.4, 0.6, 0.8, 1]);
});

test('zooming in at the minimum span keeps the range where it is', () => {
  const context = { x: [332, 668] as const, extent: [0, 1000] as const, y: [0, 1] as const, minSpan: 336 };
  const view = reduceView(EMPTY_VIEW, { type: 'wheel', at: 500, steps: -1 }, context);
  assert.ok(Math.abs(view.x![0] - 332) < 1e-9 && Math.abs(view.x![1] - 668) < 1e-9, `moved to ${view.x}`);
});

test('two identical colours on a mid-grey surface still come out distinct', () => {
  const green = parseColor('#008300')!;
  const [a, b] = nudgeApart([green, green], parseColor('#777777')!);
  assert.notEqual(toHex(a), toHex(b));
});

test('ticks for a tiny range at a huge value end instead of looping', () => {
  assert.deepEqual(linearTicks(1e17, 1e17 + 10), [1e17, 1e17 + 10]);
});

test('all-negative data keeps the zero line, as all-positive data does', () => {
  const { range } = resolveY(EMPTY_VIEW, { yMin: null, yMax: null, log: false }, [-100, -50]);
  assert.equal(range[0], -100);
  assert.ok(range[1] > 0);
});

test('labels that cannot all fit start at the top instead of rising above the plot', () => {
  const labels = Array.from({ length: 10 }, (_, index) => ({ id: String(index), y: 50, height: 14 }));
  const placed = layoutEndLabels(labels, 0, 60);
  assert.ok([...placed.values()].every(y => y >= 7));
});
