import assert from 'node:assert/strict';
import test from 'node:test';
import { engineHour } from '../../dashboard/calendar.ts';
import { keyTarget, logDate, logItems, logOffsets, monthLabel, MONTH_HEIGHT, newestFirst, ROW_HEIGHT, rowId, scrollToShow, shownMotion, tipRow, visibleRange } from './logData.ts';
import type { LogRow } from './logData';
import type { MovingRow } from '../rowMotion';
import { LOG_OPTIONS, logSettings } from './logOptions.ts';
import { resolveOptions } from '../../dashboard/options.ts';

const hour = (year: number, month: number, day: number, at = 1) => engineHour({ year, month, day, hour: at }) as number;
const row = (key: string, at: number, by = false): LogRow<string> => ({ key, hour: at, by, item: key });

test('rows are newest first, and rows of one hour keep the order given', () => {
  const rows = [row('a', 10), row('b', 30), row('c', 20), row('d', 30)];
  assert.deepEqual(newestFirst(rows).map(item => item.key), ['b', 'd', 'c', 'a']);
});

test('month dividers precede each month’s first row, keyed by the month so they hold still', () => {
  const rows = newestFirst([row('a', hour(1941, 6, 22)), row('b', hour(1941, 6, 2)), row('c', hour(1941, 5, 30)), row('d', hour(1940, 12, 31))]);
  const items = logItems(rows, true);
  assert.deepEqual(items.map(item => item.kind === 'month' ? `[${item.label}]` : item.key), ['[Jun 1941]', 'a', 'b', '[May 1941]', 'c', '[Dec 1940]', 'd']);
  assert.equal(logItems(rows, false).length, 4);
  // The same month keeps its key whatever rows it holds.
  const again = logItems(newestFirst([row('x', hour(1941, 6, 30)), ...rows]), true);
  assert.equal(again[0].key, items[0].key);
  assert.equal(monthLabel(hour(1936, 1, 1)), 'Jan 1936');
  assert.equal(monthLabel(0), 'Undated');
});

test('offsets stack rows and dividers at their heights for either density', () => {
  const items = logItems(newestFirst([row('a', hour(1941, 6, 2)), row('b', hour(1941, 5, 2))]), true);
  assert.deepEqual(logOffsets(items, 'comfortable'), [0, MONTH_HEIGHT, MONTH_HEIGHT + ROW_HEIGHT.comfortable, 2 * MONTH_HEIGHT + ROW_HEIGHT.comfortable,
    2 * MONTH_HEIGHT + 2 * ROW_HEIGHT.comfortable]);
  assert.equal(logOffsets(items, 'compact').at(-1), 2 * MONTH_HEIGHT + 2 * ROW_HEIGHT.compact);
  assert.deepEqual(logOffsets([], 'compact'), [0]);
});

test('only the items in view are drawn, with a margin each way', () => {
  const offsets = Array.from({ length: 101 }, (_, index) => index * 10);
  assert.deepEqual(visibleRange(offsets, 0, 50, 2), [0, 7]);
  assert.deepEqual(visibleRange(offsets, 205, 50, 2), [18, 28]);
  // Scrolled past the end, or a list with nothing in it.
  assert.deepEqual(visibleRange(offsets, 5000, 50, 2), [97, 100]);
  assert.deepEqual(visibleRange([0], 0, 50), [0, 0]);
});

test('reaching a row scrolls as little as needed, clear of a held month divider', () => {
  const offsets = Array.from({ length: 21 }, (_, index) => index * 10);
  // Already whole in view: no scroll.
  assert.equal(scrollToShow(offsets, 3, 0, 50), 0);
  // Below the view: its bottom meets the view's bottom.
  assert.equal(scrollToShow(offsets, 8, 0, 50), 40);
  // Above the view: its top clears the divider held over the rows.
  assert.equal(scrollToShow(offsets, 5, 100, 50, 24), 26);
  assert.equal(scrollToShow(offsets, 0, 100, 50, 24), 0);
});

test('keys move the keyboard row within the list', () => {
  assert.equal(keyTarget('ArrowDown', 0, 5, 3), 1);
  assert.equal(keyTarget('ArrowDown', 4, 5, 3), 4);
  assert.equal(keyTarget('ArrowUp', 0, 5, 3), 0);
  assert.equal(keyTarget('PageDown', 1, 5, 3), 4);
  assert.equal(keyTarget('PageUp', 4, 5, 3), 1);
  assert.equal(keyTarget('Home', 3, 5, 3), 0);
  assert.equal(keyTarget('End', 0, 5, 3), 4);
  assert.equal(keyTarget('a', 0, 5, 3), null);
  assert.equal(keyTarget('ArrowDown', 0, 0, 3), null);
});

test('a row’s date shows its day and month over its year', () => {
  assert.deepEqual(logDate(hour(1943, 12, 18, 4)), { day: '18 Dec', year: '1943' });
  assert.deepEqual(logDate(12), { day: 'Hour 12', year: '' });
});

test('log settings default to the signed-off look and fall back on bad values', () => {
  assert.deepEqual(logSettings(resolveOptions(LOG_OPTIONS, {})), { density: 'comfortable', months: false, animate: true, chips: true });
  assert.deepEqual(logSettings(resolveOptions(LOG_OPTIONS, { density: 'compact', months: true, rowAnimation: false, chips: false })),
    { density: 'compact', months: true, animate: false, chips: false });
  assert.equal(logSettings(resolveOptions(LOG_OPTIONS, { density: 'tiny' })).density, 'comfortable');
});

test('without animation a row that is gone goes at once, never left over its replacement', () => {
  const moving = (key: string, slot: number, phase: MovingRow<string>['phase']): MovingRow<string> => ({ key, item: key, slot, phase, since: 0 });
  const rows = [moving('new', 0, 'entering'), moving('b', 1, 'staying'), moving('old', 0, 'leaving'), moving('far', 40, 'staying')];
  const keys = (list: MovingRow<string>[]) => list.map(item => item.key);
  assert.deepEqual(keys(shownMotion(rows, 0, 10, true)), ['new', 'b', 'old']);
  assert.deepEqual(keys(shownMotion(rows, 0, 10, false)), ['new', 'b']);
  // The keyboard's row stays drawn out of view, but a gone row is never kept.
  assert.deepEqual(keys(shownMotion(rows, 0, 10, false, item => item.key === 'far' || item.key === 'old')), ['new', 'b', 'far']);
});

test('a tooltip follows what its row is about across the row’s keys, and goes with it', () => {
  const stage = (key: string, id?: string): LogRow<string> => ({ key, id, hour: 1, item: key });
  assert.equal(rowId(stage('p:0', 'p')), 'p');
  assert.equal(rowId(stage('plain')), 'plain');
  const before = new Map([stage('p:0', 'p')].map(row => [rowId(row), row]));
  const after = new Map([stage('p:1', 'p')].map(row => [rowId(row), row]));
  // The project built a level: its row re-entered as p:1, and the tooltip now shows that row.
  assert.equal(tipRow({ id: 'p' }, before)?.key, 'p:0');
  assert.equal(tipRow({ id: 'p' }, after)?.key, 'p:1');
  // Filtered out or gone: the tooltip is hidden, not left describing an old state.
  assert.equal(tipRow({ id: 'p' }, new Map()), null);
  assert.equal(tipRow(null, after), null);
});
