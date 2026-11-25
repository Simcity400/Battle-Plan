import assert from 'node:assert/strict';
import test from 'node:test';
import { clearTextWidthCache, textWidth, textWidthCacheStats } from './textWidth.ts';

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
