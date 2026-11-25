import assert from 'node:assert/strict';
import test from 'node:test';
import { engineHour } from '../../dashboard/calendar.ts';
import { anchoredScrollTop, logOffsets, MONTH_HEIGHT, ROW_HEIGHT } from './logData.ts';
import type { LogPage, LogRow } from './logData';
import { logPageItems, logPageRange, logPageTarget, logPixelTarget, logRowOrdinals, logRowTarget, wantedLogPages } from './logPages.ts';
import { logItemState, logShownOrdinals, logSpans } from './logAhead.ts';

const row = (key: string, hour: number): LogRow<string> => ({ key, hour, item: key });
const page = (key: string, start: number, count: number, rows?: LogRow<string>[]): LogPage<string> => ({ key, start, end: start + 9, count, rows });

test('window order and each loaded window are newest first while unloaded counts occupy one block', () => {
  const items = logPageItems([page('old', 0, 1000), page('new', 20, 3, [row('early', 21), row('late', 28)]), page('middle', 10, 400)], false);
  assert.deepEqual(items.map(item => item.kind === 'row' ? item.row.key : item.kind === 'page' ? item.page.key : item.label), ['late', 'early', 'middle', 'old']);
  assert.deepEqual(logOffsets(items, 'comfortable'), [0, 38, 76, 15276, 53276]);
  assert.equal(logRowOrdinals(items).at(-1), 1402);
  assert.equal(logRowOrdinals(items).length, 5);
});

test('viewport requests use a row-height margin, not unloaded page-size overscan', () => {
  const pages = Array.from({ length: 200 }, (_, index) => page(`window ${index}`, index * 10, 100000));
  const items = logPageItems(pages, false);
  const offsets = logOffsets(items, 'comfortable');
  assert.deepEqual(logPageRange(offsets, 0, 0, 'comfortable'), [0, 1]);
  assert.deepEqual(wantedLogPages(items, ...logPageRange(offsets, 0, 0, 'comfortable')), ['window 199']);
  const inside = offsets[100] + 500;
  const range = logPageRange(offsets, inside, 400, 'comfortable');
  assert.deepEqual(range, [100, 101]);
  assert.deepEqual(wantedLogPages(items, ...range), ['window 99']);
  const boundary = logPageRange(offsets, offsets[100] - 50, 400, 'comfortable');
  assert.deepEqual(wantedLogPages(items, ...boundary), ['window 100', 'window 99']);
  assert.deepEqual(wantedLogPages(items, ...range, ['window 199']), ['window 199', 'window 99']);
});

test('loading an empty filtered page lazily discovers the next page without repeating checked windows', () => {
  let items = logPageItems([page('first', 20, 20), page('second', 10, 20), page('third', 0, 20)], false);
  assert.deepEqual(wantedLogPages(items, ...logPageRange(logOffsets(items, 'comfortable'), 0, 300, 'comfortable')), ['first']);
  items = logPageItems([page('first', 20, 20, []), page('second', 10, 20), page('third', 0, 20)], false);
  assert.deepEqual(wantedLogPages(items, ...logPageRange(logOffsets(items, 'comfortable'), 0, 300, 'comfortable')), ['second']);
  items = logPageItems([page('first', 20, 0), page('second', 10, 0), page('third', 0, 20)], false);
  assert.deepEqual(wantedLogPages(items, ...logPageRange(logOffsets(items, 'comfortable'), 0, 300, 'comfortable')), ['third']);
});

test('releasing a filtered page retains its scrollbar height from the caller count', () => {
  const rows = [row('a', 29), row('b', 28), row('c', 27)];
  const held = logPageItems([page('new', 20, 20, rows), page('old', 0, 50)], false);
  const released = logPageItems([page('new', 20, rows.length), page('old', 0, 50)], false);
  assert.equal(logOffsets(held, 'comfortable').at(-1), logOffsets(released, 'comfortable').at(-1));
  assert.equal(logRowOrdinals(held).at(-1), logRowOrdinals(released).at(-1));
});

test('keyboard targets reach pending windows and retain their offset when loaded or filtered', () => {
  const pending = logPageItems([page('new', 20, 100), page('old', 0, 1000)], false);
  const ordinals = logRowOrdinals(pending);
  assert.deepEqual(logRowTarget(pending, ordinals, 1099), { index: 1, offset: 999, ordinal: 1099, pageKey: 'old' });
  assert.equal(logRowTarget(pending, ordinals, -1), null);
  assert.equal(logRowTarget(pending, ordinals, 1100), null);
  const held = logPageItems([page('new', 20, 100), page('old', 0, 1000, [row('latest', 9), row('earliest', 2)])], false);
  const target = logPageTarget(held, logRowOrdinals(held), 'old', 999);
  assert.equal(target?.row?.key, 'earliest');
  assert.equal(target?.offset, 1);
  assert.equal(target?.ordinal, 101);
  assert.equal(logPageTarget(logPageItems([page('old', 0, 0)], false), [0], 'old', 999), null);
  const spans = logSpans(held), shown = (index: number) => logItemState(spans, index, 5) === 'shown';
  assert.deepEqual(logPageTarget(held, logShownOrdinals(held, spans, 5), 'old', 0, shown), { index: 2, offset: 0, ordinal: 100, pageKey: 'old', row: held[2].kind === 'row' ? held[2].row : undefined });
  assert.equal(logPageTarget(held, logShownOrdinals(held, spans, 1), 'old', 0, index => logItemState(spans, index, 1) === 'shown'), null, 'no row of the window shows yet');
});

test('months are unknown over pending pages and loaded month sections keep unique keys', () => {
  const hour = (month: number, day: number) => engineHour({ year: 1941, month, day, hour: 1 }) as number;
  const items = logPageItems([
    { key: 'new', start: hour(6, 10), end: hour(6, 20), count: 2, rows: [row('newest', hour(6, 15))] },
    { key: 'pending', start: hour(6, 5), end: hour(6, 9), count: 10 },
    { key: 'old', start: hour(5, 1), end: hour(6, 4), count: 2, rows: [row('same-month', hour(6, 3)), row('oldest', hour(5, 1))] },
  ], true);
  assert.deepEqual(items.filter(item => item.kind === 'month').map(item => item.label), ['Jun 1941', 'Jun 1941', 'May 1941']);
  assert.equal(new Set(items.map(item => item.key)).size, items.length);
  assert.equal(logOffsets(items, 'compact').at(-1), 13 * ROW_HEIGHT.compact + 3 * MONTH_HEIGHT);
  const ordinals = logRowOrdinals(items);
  assert.equal(logRowTarget(items, ordinals, 0)?.row?.key, 'newest');
  assert.equal(logRowTarget(items, ordinals, 11)?.row?.key, 'same-month');
  assert.equal(logRowTarget(items, ordinals, 12)?.row?.key, 'oldest');
});

test('a failed page remains a counted virtual window and is wanted for caller retry', () => {
  const failed = { ...page('failed opaque | page', 0, 5000), error: new Error('Unsupported content form: scripted icon') };
  const items = logPageItems([failed], false);
  assert.equal(items.length, 1);
  assert.equal(logOffsets(items, 'comfortable').at(-1), 5000 * ROW_HEIGHT.comfortable);
  assert.equal(items[0].kind === 'page' && items[0].page.error, failed.error);
  assert.deepEqual(wantedLogPages(items, ...logPageRange(logOffsets(items, 'comfortable'), 10000, 400, 'comfortable')), [failed.key]);
});

test('long campaigns at 5000-country scale allocate by windows and only the drawn page payload', () => {
  const pages = Array.from({ length: 5000 }, (_, index) => page(`p${index}`, index * 10, 5000));
  const items = logPageItems(pages, false);
  const ordinals = logRowOrdinals(items);
  assert.equal(items.length, 5000);
  assert.equal(ordinals.at(-1), 25000000);
  assert.equal(logOffsets(items, 'comfortable').at(-1), 25000000 * ROW_HEIGHT.comfortable);
  assert.deepEqual(logRowTarget(items, ordinals, 24999999), { index: 4999, offset: 4999, ordinal: 24999999, pageKey: 'p0' });
  assert.deepEqual(wantedLogPages(items, ...logPageRange(logOffsets(items, 'comfortable'), 200000 * ROW_HEIGHT.comfortable + 500, 400, 'comfortable')), ['p4959']);
});


test('page loading and release preserve the visible row offset, including month headings', () => {
  const rows = [row('a', engineHour({ year: 1941, month: 6, day: 2, hour: 1 })!),
    row('b', engineHour({ year: 1941, month: 5, day: 2, hour: 1 })!), row('c', 1)];
  for (const density of ['compact', 'comfortable'] as const) for (const months of [false, true]) {
    const pending = logPageItems([page('new', 20, 2), page('held', 0, rows.length)], months);
    const loaded = logPageItems([page('new', 20, 1, [row('new', 29)]), page('held', 0, rows.length, rows)], months);
    const before = logOffsets(pending, density), after = logOffsets(loaded, density);
    const top = before[1] + ROW_HEIGHT[density] + 3;
    const target = loaded.findIndex(item => item.key === 'b');
    const anchored = anchoredScrollTop(pending, before, loaded, after, top);
    assert.equal(anchored, after[target] + 3);
    assert.equal(anchoredScrollTop(loaded, after, pending, before, anchored), top);
    assert.equal(anchoredScrollTop(pending, before, loaded, after, 0), 0);
  }
});

test('a filtered page anchors to its last remaining row and a removed page retains its position', () => {
  const pending = logPageItems([page('p', 0, 10)], false);
  const loaded = logPageItems([page('p', 0, 2, [row('a', 9), row('b', 8)])], false);
  const before = logOffsets(pending, 'comfortable'), after = logOffsets(loaded, 'comfortable');
  assert.equal(anchoredScrollTop(pending, before, loaded, after, ROW_HEIGHT.comfortable * 8 + 3), after[1] + 3);
  assert.equal(anchoredScrollTop(loaded, after, [], [0], after[1] + 3), after[1] + 3);
});


test('pixel paging retains measured card destinations around month headings', () => {
  const june = engineHour({ year: 1941, month: 6, day: 2, hour: 1 })!;
  const may = engineHour({ year: 1941, month: 5, day: 2, hour: 1 })!;
  const items = logPageItems([page('cards', 0, 4, [row('a', june + 1), row('b', june), row('c', may + 1), row('d', may)])], true);
  const heights = new Map([['a', 100], ['b', 300], ['c', 50], ['d', 80]]);
  const offsets = logOffsets(items, 'comfortable', at => heights.get(at.key)!);
  const ordinals = logRowOrdinals(items);
  assert.deepEqual(offsets, [0, 24, 124, 424, 448, 498, 578]);
  assert.equal(logPixelTarget(items, offsets, ordinals, 0, 400, 1)?.row?.key, 'b');
  assert.equal(logPixelTarget(items, offsets, ordinals, 3, 74, -1)?.row?.key, 'c');
  assert.equal(logPixelTarget(items, offsets, ordinals, 1, 10, 1)?.row?.key, 'c');
  assert.equal(logPixelTarget(items, offsets, ordinals, 2, 1000, -1)?.row?.key, 'a');
});

test('pixel paging reaches compact pending offsets and the same filtered page destination', () => {
  const pending = logPageItems([page('cards', 20, 1, [row('tall', 29)]), page('pending', 0, 20)], false);
  const offsets = logOffsets(pending, 'comfortable', () => 300);
  const ordinals = logRowOrdinals(pending);
  const target = logPixelTarget(pending, offsets, ordinals, 0, 400, 1)!;
  assert.deepEqual(target, { index: 1, offset: 2, ordinal: 3, pageKey: 'pending' });
  assert.equal(logPixelTarget(pending, offsets, ordinals, 5, 80, -1)?.offset, 2);
  const loaded = logPageItems([page('cards', 20, 1, [row('tall', 29)]), page('pending', 0, 20, [row('newer', 9), row('older', 8)])], false);
  assert.equal(logPageTarget(loaded, logRowOrdinals(loaded), target.pageKey!, target.offset)?.row?.key, 'older');
});

test('page load and release clamp anchors inside measured cards and pending row estimates', () => {
  const pending = logPageItems([page('cards', 0, 3)], false);
  const loaded = logPageItems([page('cards', 0, 3, [row('a', 9), row('b', 8), row('c', 7)])], false);
  const before = logOffsets(pending, 'comfortable');
  const measured = logOffsets(loaded, 'comfortable', at => at.key === 'b' ? 10 : 100);
  assert.equal(anchoredScrollTop(pending, before, loaded, measured, 38 + 30), 100 + 9);
  assert.equal(anchoredScrollTop(loaded, measured, pending, before, 110 + 80), 76 + 37);
});


test('loading a same-month newer page does not transfer an older page divider anchor', () => {
  const january = engineHour({ year: 1941, month: 1, day: 2, hour: 1 })!;
  const oldPage = page('C', january, 1, [row('C', january)]);
  const pending = logPageItems([page('A', january + 20, 100), oldPage], true);
  const loaded = logPageItems([page('A', january + 20, 100, [row('A', january + 20)]), oldPage], true);
  const before = logOffsets(pending, 'comfortable'), after = logOffsets(loaded, 'comfortable');
  assert.deepEqual(before, [0, 3800, 3824, 3862]);
  assert.deepEqual(after, [0, 24, 62, 100]);
  assert.equal(anchoredScrollTop(pending, before, loaded, after, 3800), 62);
});
