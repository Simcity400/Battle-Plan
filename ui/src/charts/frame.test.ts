import test from 'node:test';
import assert from 'node:assert/strict';
import { DisplayRate } from '../draw/frameRate.ts';
import { axisFraction, hitZone, plotFrame, pointerHour, wheelSteps } from './frame.ts';
import { hideTooltip, showTooltip, tooltipStore } from './tooltip.ts';
import { parseAppearance, DEFAULT_APPEARANCE } from '../shell/appearance/settings.ts';

const FRAME = plotFrame(400, 200, { left: 40, right: 10, top: 10, bottom: 30 }, [1000, 2000], [0, 100], false);


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

test('the display rate is measured from short frame intervals', () => {
  const rate = new DisplayRate();
  assert.equal(rate.interval, 0, 'a display not yet measured gives no interval');
  for (let i = 0; i <= 32; i++) rate.sample(i * 10);
  assert.equal(rate.interval, 10);
  rate.sample(325);
  assert.equal(rate.interval, 5, 'a faster display shortens the interval at its first frame');
});

