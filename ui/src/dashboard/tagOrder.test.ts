import assert from 'node:assert/strict';
import test from 'node:test';
import { topCountries } from './groups.ts';
import { compareTags } from './tagOrder.ts';

test('tags order by their UTF-8 bytes, as the data service (Rust) orders them', () => {
  const sorted = (tags: string[]) => [...tags].sort(compareTags);
  assert.deepEqual(sorted(['aaa', 'AAA', 'BBB', 'Aa']), ['AAA', 'Aa', 'BBB', 'aaa']);
  assert.ok('aaa'.localeCompare('AAA') < 0, 'localeCompare disagrees');
  assert.deepEqual(sorted(['\u{10000}', '']), ['', '\u{10000}']);
  assert.ok('\u{10000}' < '', 'code units disagree');
  assert.equal(compareTags('GER', 'GER'), 0);
  assert.equal(compareTags('GE', 'GER'), -1);
  assert.equal(compareTags('GER', 'GE'), 1);
});

test('ranked ties break in the service’s tag order everywhere', () => {
  assert.deepEqual(topCountries(new Map([['aaa', 5], ['AAA', 5]]), 2), ['AAA', 'aaa']);
});
