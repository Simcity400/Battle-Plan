import assert from 'node:assert/strict';
import test from 'node:test';
import { engineHour } from '../../dashboard/calendar.ts';
import { anchoredScrollTop, cardHeightsAt, cardPageTarget, hasLogPartTip, keyTarget, logItems, logOffsets, monthLabel, newCardKeys, newestFirst, rowId, scrollToShow, shownMotion, tipRow, updateCardHeights, visibleRange } from './logData.ts';
import type { CardMeasurements, LogRow } from './logData';
import type { MovingRow } from '../rowMotion';
import { commitRows, EMPTY_HISTORY, shownRows } from '../rowMotion.ts';
import { shownSlot } from '../rowNavigation.ts';

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
  const again = logItems(newestFirst([row('x', hour(1941, 6, 30)), ...rows]), true);
  assert.equal(again[0].key, items[0].key);
  assert.equal(monthLabel(hour(1936, 1, 1)), 'Jan 1936');
  assert.equal(monthLabel(0), 'Undated');
});


test('pinned running rows retain unique month sections before newest-first history', () => {
  const items = logItems([row('running', hour(1941, 5, 30)), row('finished', hour(1941, 6, 2)), row('older', hour(1941, 5, 2))], true);
  assert.equal(new Set(items.map(item => item.key)).size, items.length);
  assert.deepEqual(items.filter(item => item.kind === 'month').map(item => item.label), ['May 1941', 'Jun 1941', 'May 1941']);
});

test('only the items in view are drawn, with a margin each way', () => {
  const offsets = Array.from({ length: 101 }, (_, index) => index * 10);
  assert.deepEqual(visibleRange(offsets, 0, 50, 2), [0, 7]);
  assert.deepEqual(visibleRange(offsets, 205, 50, 2), [18, 28]);
  assert.deepEqual(visibleRange(offsets, 5000, 50, 2), [97, 100]);
  assert.deepEqual(visibleRange([0], 0, 50), [0, 0]);
});

test('reaching a row scrolls as little as needed, clear of a held month divider', () => {
  const offsets = Array.from({ length: 21 }, (_, index) => index * 10);
  assert.equal(scrollToShow(offsets, 3, 0, 50), 0);
  assert.equal(scrollToShow(offsets, 8, 0, 50), 40);
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

test('a step reaches the nearest row shown that way, or none', () => {
  const shown = (slot: number) => slot !== 1 && slot !== 4;
  assert.equal(shownSlot(1, 1, 5, shown), 2, 'down over a row held ahead');
  assert.equal(shownSlot(1, -1, 5, shown), 0, 'up over it');
  assert.equal(shownSlot(4, -1, 5, shown), 3, 'the last row shown, from the end');
  assert.equal(shownSlot(5, 1, 5, shown), null, 'nothing shown below the last row shown');
  assert.equal(shownSlot(-1, -1, 5, () => true), 0, 'clamped into the rows');
  assert.equal(shownSlot(0, 1, 0, () => true), null);
});



test('without animation a row that is gone goes at once, never left over its replacement', () => {
  const moving = (key: string, slot: number, phase: MovingRow<string>['phase']): MovingRow<string> => ({ key, item: key, slot, phase, since: 0 });
  const rows = [moving('new', 0, 'entering'), moving('b', 1, 'staying'), moving('old', 0, 'leaving'), moving('far', 40, 'staying')];
  const keys = (list: MovingRow<string>[]) => list.map(item => item.key);
  assert.deepEqual(keys(shownMotion(rows, 0, 10, true)), ['new', 'b', 'old']);
  assert.deepEqual(keys(shownMotion(rows, 0, 10, false)), ['new', 'b']);
  assert.deepEqual(keys(shownMotion(rows, 0, 10, false, item => item.key === 'far' || item.key === 'old')), ['new', 'b', 'far']);
});

test('a tooltip follows what its row is about across the row’s keys, and goes with it', () => {
  const stage = (key: string, id?: string): LogRow<string> => ({ key, id, hour: 1, item: key });
  assert.equal(rowId(stage('p:0', 'p')), 'p');
  assert.equal(rowId(stage('plain')), 'plain');
  const before = new Map([stage('p:0', 'p')].map(row => [rowId(row), row]));
  const after = new Map([stage('p:1', 'p')].map(row => [rowId(row), row]));
  assert.equal(tipRow({ id: 'p' }, before)?.key, 'p:0');
  assert.equal(tipRow({ id: 'p' }, after)?.key, 'p:1');
  assert.equal(tipRow({ id: 'p' }, new Map()), null);
  assert.equal(tipRow(null, after), null);
});


test('window release preserves a scrolled row while the top follows new entries', async () => {
  const { anchoredScrollTop, logItems, logOffsets } = await import('./logData.ts');
  const rows = Array.from({ length: 100 }, (_, i) => ({ key: String(i), hour: 100 - i, item: i }));
  const before = logItems(rows, false), after = logItems(rows.slice(20), false);
  const oldOffsets = logOffsets(before, 'compact'), newOffsets = logOffsets(after, 'compact');
  Object.defineProperty(oldOffsets, 'findLastIndex', { value: undefined });
  assert.equal(anchoredScrollTop(before, oldOffsets, after, newOffsets, oldOffsets[30] + 3), newOffsets[10] + 3);
  assert.equal(anchoredScrollTop(before, oldOffsets, after, newOffsets, 0), 0);
});


test('a new interface font discards offscreen card measurements at unchanged width', () => {
  const measured = { width: 400, density: 'comfortable' as const, font: 'First', heights: new Map([['old', 100]]) };
  assert.equal(cardHeightsAt(measured, 400, 'comfortable', 'First').get('old'), 100);
  assert.equal(cardHeightsAt(measured, 400, 'comfortable', 'Second').size, 0);
  const next = updateCardHeights(measured, 400, 'comfortable', [{ key: 'drawn', height: 120 }], 'Second');
  assert.deepEqual([...next.heights], [['drawn', 120]]);
});


test('card heights stack with month dividers and window by their measured layout pixels', () => {
  const rows = [row('a', hour(1941, 6, 2)), row('b', hour(1941, 6, 1)), row('c', hour(1941, 5, 2))];
  const heights = new Map([['a', 150], ['b', 310], ['c', 90]]);
  const items = logItems(rows, true);
  const offsets = logOffsets(items, 'comfortable', at => heights.get(at.key)!);
  assert.deepEqual(offsets, [0, 24, 174, 484, 508, 598]);
  assert.deepEqual(visibleRange(offsets, 200, 200, 0), [2, 3]);
  assert.deepEqual(visibleRange(offsets, 460, 100, 0), [2, 5]);
  heights.set('b', 350);
  assert.equal(logOffsets(items, 'compact', at => heights.get(at.key)!).at(-1), 638);
  assert.deepEqual(logOffsets(logItems(rows, false), 'compact', () => Number.NaN), [0, 28, 56, 84]);
});


test('layout width and density changes discard old wrapping heights, including unseen cards', () => {
  const measured = { width: 600, density: 'comfortable' as const, heights: new Map([['drawn', 140], ['unseen', 160]]) };
  assert.equal(cardHeightsAt(measured, 600, 'comfortable'), measured.heights);
  assert.equal(cardHeightsAt(measured, 400, 'comfortable').size, 0);
  assert.equal(cardHeightsAt(measured, 600, 'compact').size, 0);
  assert.equal(updateCardHeights(measured, 600, 'comfortable', [{ key: 'drawn', height: 140 }]), measured);
  const resized = updateCardHeights(measured, 400, 'comfortable', [{ key: 'drawn', height: 210 }]);
  assert.deepEqual([...resized.heights], [['drawn', 210]]);
  const denser = updateCardHeights(resized, 400, 'compact', [{ key: 'drawn', height: 120 }]);
  assert.deepEqual([...denser.heights], [['drawn', 120]]);
  const more = updateCardHeights(denser, 400, 'compact', [{ key: 'new', height: 170 }, { key: 'invalid', height: Number.NaN }]);
  assert.deepEqual([...more.heights], [['drawn', 120], ['new', 170]]);
  assert.deepEqual([...updateCardHeights(more, 350, 'compact', []).heights], []);
});


test('a descendant tooltip takes hover from its log row, including nested chip flags', () => {
  type Part = { parentElement: Part | null; getAttribute(name: string): string | null };
  const part = (parentElement: Part | null, tooltip: string | null = null): Part => ({ parentElement, getAttribute: name => name === 'data-tooltip' ? tooltip : null });
  const row = part(null);
  const plain = part(row);
  const chip = part(row, 'Remaining state names');
  const flag = part(chip);
  assert.equal(hasLogPartTip(chip, row), true);
  assert.equal(hasLogPartTip(flag, row), true);
  assert.equal(hasLogPartTip(plain, row), false);
  assert.equal(hasLogPartTip(part(row, ''), row), false);
  assert.equal(hasLogPartTip(part(chip, ''), row), false);
  assert.equal(hasLogPartTip(row, row), false);
  assert.equal(hasLogPartTip(null, row), false);
});


test('a card taller than the viewport is revealed at its heading without alternating ends', () => {
  const offsets = [0, 100, 600, 700];
  assert.equal(scrollToShow(offsets, 1, 0, 200), 100);
  assert.equal(scrollToShow(offsets, 1, 100, 200), 100);
  assert.equal(scrollToShow(offsets, 1, 400, 200, 24), 76);
  assert.equal(scrollToShow(offsets, 0, 200, 80, 24), 0);
});


test('paging cards moves one viewport through row heights, skipping month dividers', () => {
  const offsets = [0, 24, 124, 424, 448, 498, 578];
  const slots = [1, 2, 4, 5];
  assert.equal(cardPageTarget(offsets, slots, 0, 400, 1), 1);
  assert.equal(cardPageTarget(offsets, slots, 0, 440, 1), 2);
  assert.equal(cardPageTarget(offsets, slots, 1, 100, 1), 2);
  assert.equal(cardPageTarget(offsets, slots, 3, 400, -1), 1);
  assert.equal(cardPageTarget(offsets, slots, 1, 20, -1), 0);
  assert.equal(cardPageTarget(offsets, slots, 3, 400, 1), 3);
  assert.equal(cardPageTarget(offsets, [], 0, 400, 1), 0);
});


test('cards enter only once; existing cards never re-enter after scroll or backward seek', () => {
  const wrap = (keys: string[]) => keys.map(key => ({ key, item: key }));
  const first = wrap(['old']);
  let history = commitRows(EMPTY_HISTORY, first, 0, 450);
  const known = new Set(['old']);
  assert.deepEqual(newCardKeys(shownRows(history, first, 0, 450), known), []);
  const arriving = wrap(['new', 'old']);
  const moving = shownRows(history, arriving, 0, 450);
  assert.deepEqual(newCardKeys(moving, known), ['new']);
  for (const item of arriving) known.add(item.key);
  history = commitRows(history, arriving, 0, 450);
  assert.equal(shownRows(history, arriving, 0, 450)[0].phase, 'entering');
  assert.deepEqual(newCardKeys(shownRows(history, arriving, 0, 450), known), []);
  history = commitRows(history, first, 1000, 450);
  history = commitRows(history, first, 2000, 450);
  assert.deepEqual(newCardKeys(shownRows(history, arriving, 2000, 450), known), []);
  const empty = commitRows(EMPTY_HISTORY, wrap([]), 0, 450);
  assert.deepEqual(newCardKeys(shownRows(empty, first, 0, 450), new Set()), ['old']);
});


test('cards added with motion disabled never replay when full motion is restored', () => {
  const rows: MovingRow<string>[] = [{ key: 'new', item: 'new', slot: 0, phase: 'entering', since: 0 }];
  const known = new Set<string>();
  assert.deepEqual(newCardKeys(rows, known, false), []);
  for (const row of rows) known.add(row.key);
  assert.deepEqual(newCardKeys(rows, known, true), []);
});


test('5,000 variable-height cards retain every offset and window only the rows in view', () => {
  const count = 5_000;
  const items = logItems(Array.from({ length: count }, (_, index) => row(String(index), count - index)), false);
  const offsets = logOffsets(items, 'comfortable', at => 110 + at.hour % 5 * 28);
  assert.equal(items.length, count);
  assert.equal(offsets.length, count + 1);
  assert.equal(offsets.at(-1), 830_000);
  const [first, last] = visibleRange(offsets, offsets[count / 2], 500);
  assert.ok(last - first < 30);
  assert.deepEqual([first, last], [2_494, 2_509]);
});


test('card logs remove future cards immediately on a backward seek or Live cutoff', () => {
  const rows: MovingRow<string>[] = [
    { key: 'reached', item: 'reached', slot: 0, phase: 'staying', since: 0 },
    { key: 'future', item: 'future', slot: 0, phase: 'leaving', since: 0 },
  ];
  assert.deepEqual(shownMotion(rows, 0, 10, true, () => true, 'card').map(row => row.key), ['reached']);
  assert.deepEqual(shownMotion(rows, 0, 10, true).map(row => row.key), ['reached', 'future']);
});


test('measured cards preserve the reached heading when overscan heights change above it', () => {
  const items = logItems([row('above', 3), row('reached', 2), row('below', 1)], false);
  const before = logOffsets(items, 'comfortable', () => 100);
  const heights = new Map([['above', 180], ['reached', 140], ['below', 100]]);
  const after = logOffsets(items, 'comfortable', at => heights.get(at.key)!);
  const top = anchoredScrollTop(items, before, items, after, 112);
  assert.equal(top, 192);
  assert.deepEqual(visibleRange(after, top, 60, 0), [1, 2]);
  assert.equal(anchoredScrollTop(items, before, items, after, 0), 0);
});

test('a changed type scale invalidates offscreen wrapping at the same font family and width', () => {
  const first = JSON.stringify(['Synthetic family', 11, 12, 13]);
  const larger = JSON.stringify(['Synthetic family', 12, 14, 16]);
  const measured = { width: 400, density: 'comfortable' as const, font: first, heights: new Map([['unseen', 100]]) };
  assert.equal(cardHeightsAt(measured, 400, 'comfortable', first).size, 1);
  assert.equal(cardHeightsAt(measured, 400, 'comfortable', larger).size, 0);
  assert.deepEqual([...updateCardHeights(measured, 400, 'comfortable', [{ key: 'drawn', height: 140 }], larger).heights], [['drawn', 140]]);
});


test('a shrinking measured card retains its anchor inside the new height', () => {
  const items = logItems([row('reached', 2), row('next', 1)], false);
  const before = [0, 300, 600], after = [0, 100, 400];
  const top = anchoredScrollTop(items, before, items, after, 250);
  assert.equal(top, 99);
  assert.deepEqual(visibleRange(after, top, 1, 0), [0, 1]);
  assert.equal(anchoredScrollTop(items, before, items, [0, 0.5, 300], 250), 0);
});




test('paged card measurement retains only held keys through 5,000 window evictions', () => {
  let measured: CardMeasurements = { width: 400, density: 'comfortable', heights: new Map() };
  for (let window = 0; window < 5_000; window++) {
    const key = `card ${window}`, held = new Set([key]);
    measured = updateCardHeights(measured, 400, 'comfortable', [{ key, height: 100 }, { key: 'released stale node', height: 200 }], '', held);
    assert.deepEqual([...measured.heights], [[key, 100]]);
  }
  measured = updateCardHeights(measured, 400, 'comfortable', [{ key: 'ordinary', height: 140 }]);
  assert.equal(measured.heights.size, 2);
});
