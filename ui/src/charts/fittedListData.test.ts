import assert from 'node:assert/strict';
import test from 'node:test';
import { fittedListCount, fittedListRemainder } from './fittedListData.ts';

test('available width allows more than six entries and changes with actual labels', () => {
  assert.equal(fittedListCount(12, 180, () => 10, () => 30, 4), 12);
  assert.equal(fittedListCount(12, 150, () => 10, () => 30, 4), 8);
  assert.equal(fittedListCount(12, 100, () => 10, () => 30, 4), 5);
  assert.equal(fittedListCount(12, 150, () => 25, () => 30, 4), 4);
});
test('the measured remainder fits, including digit-width changes and tiny lines', () => {
  const widths = [22, 31, 18, 26, 17];
  const more = (hidden: number) => 18 + String(hidden).length * 7;
  const shown = fittedListCount(widths.length, 95, index => widths[index], more, 4);
  assert.equal(shown, 2);
  assert.ok(widths.slice(0, shown).reduce((sum, width) => sum + width, 0) + shown * 4 + more(widths.length - shown) <= 95);
  assert.equal(fittedListCount(100, 29, () => 20, more, 4), 0);
  assert.equal(fittedListCount(0, 100, () => 20, more, 4), 0);
  assert.equal(fittedListCount(100, 0, () => 20, more, 4), 0);
});
test('every remainder entry is retained, and measuring stops at line overflow', () => {
  const items = Array.from({ length: 5_000 }, (_, index) => `opaque-country:${index}`);
  let measured = 0;
  const shown = fittedListCount(items.length, 150, () => { measured++; return 10; }, () => 40, 4);
  assert.equal(shown, 7);
  assert.ok(measured <= 12);
  const rest = fittedListRemainder(items, shown, item => item);
  assert.equal(rest.split('\n').length, items.length - shown);
  assert.ok(rest.endsWith(items.at(-1)!));
});
