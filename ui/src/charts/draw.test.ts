import assert from 'node:assert/strict';
import test from 'node:test';
import { drawLine } from './draw.ts';
import { plotFrame } from './frame.ts';
import { heldSegments } from './series.ts';
import { tokensFromValues } from './tokens.ts';
import { clearTextWidthCache, textWidth, textWidthCacheStats } from './textWidth.ts';

test('shared held lines preserve exact reads, gaps, identity changes and fractional zoom edges', () => {
  const paths = heldSegments([
    { firstHour: 1, lastHour: 2, value: 2 }, { firstHour: 3, lastHour: 4, value: 8 },
    { firstHour: 6, lastHour: 6, value: 9 }, { firstHour: 7, lastHour: 8, value: 9, breakBefore: true },
  ], 1, 8, value => value, value => value).map(segment => segment.path);
  assert.deepEqual(paths, [
    [['M', 1, 2], ['L', 3, 2], ['L', 3, 8], ['L', 5, 8]],
    [['M', 6, 9], ['L', 7, 9]], [['M', 7, 9], ['L', 8, 9]],
  ]);
  assert.deepEqual(heldSegments([{ firstHour: 1, lastHour: 4, value: 2 }], 2.5, 3.5, value => value, value => value)[0].path,
    [['M', 2.5, 2], ['L', 3.5, 2]]);
  assert.deepEqual(heldSegments([{ firstHour: 1, lastHour: 4, value: 2 }], 5, 6, value => value, value => value), []);
  assert.deepEqual(heldSegments([{ firstHour: 1, lastHour: 4, value: 2 }], 3, 2, value => value, value => value), []);
});


test('line geometry follows the complete time scale while its drawn end stays fixed', () => {
  const ctx = { beginPath() {}, moveTo() {}, lineTo() {}, bezierCurveTo() {}, arc() {}, stroke() {}, fill() {}, setLineDash() {} } as unknown as CanvasRenderingContext2D;
  const tokens = tokensFromValues(() => 'transparent', 'sans-serif');
  const runs = [{ firstHour: 0, lastHour: 100, value: 1 }];
  for (const held of [false, true]) {
    for (const end of [100, 200, 50]) {
      const frame = plotFrame(400, 200, { left: 20, right: 20, top: 20, bottom: 20 }, [0, end], [0, 2], false);
      assert.deepEqual(drawLine(ctx, frame, tokens, runs, 'colour', { held, until: 40 }), { x: frame.toX(40), y: frame.toY(1) });
    }
  }
});

test('textWidth retains hot labels under LRU instead of wiping the font bucket when transient strings fill it', () => {
  clearTextWidthCache();
  let calls = 0;
  const ctx = {
    font: '12px sans-serif',
    measureText(text: string) {
      calls++;
      return { width: text.length * 7 } as TextMetrics;
    },
  };
  const hot = ['Germany', 'Soviet Union', 'United Kingdom', 'United States'];
  for (const label of hot) textWidth(ctx, label);
  for (let i = 0; i < 4500; i++) {
    if (i % 500 === 0) for (const label of hot) textWidth(ctx, label);
    textWidth(ctx, `val-${i}`);
  }
  const before = calls;
  for (const label of hot) textWidth(ctx, label);
  assert.equal(calls, before, 'hot labels stay cached without re-measuring');
  assert.ok(textWidthCacheStats().misses >= 4500);
});
