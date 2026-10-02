import assert from 'node:assert/strict';
import test from 'node:test';
import { FlowTooltipCache } from './tooltipCache.ts';
import type { TooltipContent } from '../tooltip';

test('a paused hover refreshes when its resource filter or range start changes', () => {
  const cache = new FlowTooltipCache<string>();
  const tip = (value: number) => (_hit: string): TooltipContent => ({ title: 'Seller', rows: [{ label: 'Sold', value: String(value) }] });
  const all = tip(10 + 20);
  const first = cache.describe('seller', 'seller', all);
  assert.equal(first.content?.rows[0].value, '30');
  assert.deepEqual(cache.describe('seller', 'seller', all), first);
  const filtered = cache.describe('seller', 'seller', tip(10));
  assert.equal(filtered.content?.rows[0].value, '10');
  assert.notEqual(filtered.key, first.key);
  const range = cache.describe('seller', 'seller', tip(5));
  assert.equal(range.content?.rows[0].value, '5');
  assert.notEqual(range.key, filtered.key);
});

test('a hover is rebuilt once per callback or hit, including after leaving it', () => {
  const cache = new FlowTooltipCache<string>();
  let calls = 0;
  const tooltip = (title: string) => { calls++; return { title, rows: [] }; };
  cache.describe('a', 'A', tooltip);
  for (let frame = 0; frame < 5_000; frame++) cache.describe('a', 'A', tooltip);
  assert.equal(calls, 1);
  assert.equal(cache.describe('b', 'B', tooltip).content?.title, 'B');
  cache.clear();
  cache.describe('b', 'B', tooltip);
  assert.equal(calls, 3);
});
