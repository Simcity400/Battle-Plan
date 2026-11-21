import test from 'node:test';
import assert from 'node:assert/strict';
import { DisplayRate, PresentationRate, shownAt } from '../draw/frameRate.ts';
import { axisFraction, hitZone, plotFrame, pointerHour, wheelSteps } from './frame.ts';
import { hideTooltip, showTooltip, tooltipStore } from './tooltip.ts';
import { parseAppearance, DEFAULT_APPEARANCE } from '../shell/appearance/settings.ts';

const FRAME = plotFrame(400, 200, { left: 40, right: 10, top: 10, bottom: 30 }, [1000, 2000], [0, 100], false);

test('the plot frame maps hours and values both ways', () => {
  assert.equal(FRAME.plotWidth, 350);
  assert.equal(FRAME.plotHeight, 160);
  assert.equal(FRAME.toX(1000), 40);
  assert.equal(FRAME.toX(2000), 390);
  assert.equal(FRAME.toY(0), 170);
  assert.equal(FRAME.toY(100), 10);
  assert.equal(FRAME.hourAt(FRAME.toX(1234)), 1234);
  assert.ok(Math.abs(FRAME.valueAt(FRAME.toY(42)) - 42) < 1e-9);
  const log = plotFrame(400, 200, { left: 40, right: 10, top: 10, bottom: 30 }, [0, 1], [1, 1000], true);
  assert.equal(log.log, true);
  assert.ok(Math.abs(log.toY(10) - (10 + 160 * 2 / 3)) < 1e-9);
  assert.equal(plotFrame(400, 200, FRAME.padding, [0, 1], [0, 10], true).log, false);
});

test('hit zones tell the two axes from the plot', () => {
  assert.equal(hitZone({ x: 200, y: 185 }, FRAME), 'x-axis');
  assert.equal(hitZone({ x: 20, y: 100 }, FRAME), 'y-axis');
  assert.equal(hitZone({ x: 200, y: 100 }, FRAME), 'plot');
  assert.equal(hitZone({ x: 20, y: 185 }, FRAME), null);
  assert.equal(axisFraction({ x: 215, y: 0 }, FRAME, 'x'), 0.5);
  assert.equal(axisFraction({ x: 0, y: 90 }, FRAME, 'y'), 0.5);
});

test('the tooltip store publishes changes only and each owner hides only its own', () => {
  let changes = 0;
  const stop = tooltipStore.subscribe(() => { changes++; });
  const content = { title: '12 Mar 1940', rows: [{ label: 'Balance', value: '42.0' }] };
  showTooltip('a', content, 10, 10);
  showTooltip('a', { ...content, rows: [...content.rows] }, 10, 10);
  assert.equal(changes, 1);
  hideTooltip('b');
  assert.equal(tooltipStore.get()?.owner, 'a');
  hideTooltip('a');
  assert.equal(tooltipStore.get(), null);
  assert.equal(changes, 2);
  stop();
});

test('chart settings default to smooth motion and raw colours, and reject unknown values', () => {
  assert.deepEqual(DEFAULT_APPEARANCE.charts, { motion: 'smooth', countryColours: 'raw', frameRate: '60' });
  assert.deepEqual(parseAppearance({ charts: { motion: 'cinematic', countryColours: 'fixed', frameRate: '120' } }).charts, { motion: 'cinematic', countryColours: 'fixed', frameRate: '120' });
  assert.deepEqual(parseAppearance({ charts: { motion: 'warp', countryColours: 'neon' } }).charts, DEFAULT_APPEARANCE.charts);
  assert.deepEqual(parseAppearance({}).charts, DEFAULT_APPEARANCE.charts);
});

test('the wheel zooms in proportion and ignores sideways scrolling', () => {
  assert.equal(wheelSteps(0, 0), 0);
  assert.equal(wheelSteps(100, 0), 1);
  assert.equal(wheelSteps(-4, 0), -0.04);
  assert.equal(wheelSteps(3, 1), 0.99);
  assert.equal(wheelSteps(10_000, 0), 3);
});

test('the pointer never shows hours after the playhead', () => {
  assert.equal(pointerHour(500, [0, 1000], null), 500);
  assert.equal(pointerHour(900, [0, 1000], 600), 600);
  assert.equal(pointerHour(1500, [1200, 2000], 600), null);
  assert.equal(pointerHour(-5, [0, 1000], 600), 0);
});

test('the worker presents 60 frames on both a 95 Hz and a 240 Hz display and skips missed turns', () => {
  for (const hz of [95, 240]) {
    const cadence = new PresentationRate();
    const times = Array.from({ length: hz }, (_, i) => 1000 * i / hz);
    assert.equal(times.filter(at => cadence.due(at, 60)).length, 60);
    assert.equal(cadence.due(2500, 60), true);
    assert.equal(cadence.due(2501, 60), false);
    assert.equal(times.filter(at => cadence.due(at, 0)).length, hz);
  }
});

test('a frame is drawn for the first display it can reach, the next vsync after its work begins', () => {
  const rate = new DisplayRate();
  assert.equal(rate.interval, 1000 / 60, 'a display not yet measured is taken as 60 Hz');
  for (let i = 0; i <= 32; i++) rate.sample(i * 10);
  assert.equal(rate.interval, 10);
  assert.equal(shownAt(100, 100, 10), 110, 'begun at its vsync');
  assert.equal(shownAt(100, 109.9, 10), 110, 'begun late within the interval');
  assert.equal(shownAt(100, 110, 10), 120, 'begun after the next vsync passed');
  assert.equal(shownAt(100, 95, 10), 110, 'a clock read before the frame time');
});

test('a 60 FPS cap presents each eligible native step when the 95 Hz display starts between steps', () => {
  const cadence = new PresentationRate(), presented = [];
  const stepMs = 1000 / 60, origin = 4;
  for (let i = 0; i <= 95; i++) {
    const at = i * 1000 / 95;
    const step = Math.floor((at - origin) / stepMs);
    if (cadence.due(at, 60, origin + step * stepMs)) presented.push(step);
  }
  assert.deepEqual(presented, Array.from({ length: 61 }, (_, i) => i - 1));
});
