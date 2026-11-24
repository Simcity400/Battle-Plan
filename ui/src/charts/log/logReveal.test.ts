import assert from 'node:assert/strict';
import test from 'node:test';
import { logItems } from './logData.ts';
import type { LogRow } from './logData';
import { logPageItems } from './logPages.ts';
import { EMPTY_LOG_REVEALS, logRevealHistory, logReveals as revealAt, logVisibleEntries } from './logReveal.ts';
import { deepFreeze } from '../../test-support/frozen.ts';

const logReveals = (...[previous, items, playhead]: Parameters<typeof revealAt>) => {
  const reveals = revealAt(deepFreeze(previous), deepFreeze(items), playhead);
  return deepFreeze({ ...reveals, history: logRevealHistory(previous, reveals, playhead) });
};

const row = (key: string, hour: number, revealedHour = hour): LogRow<string> => ({ key, hour, revealedHour, item: key });
const pageItems = (key: string, rows?: LogRow<string>[]) => logPageItems([{ key, start: 0, end: 200, count: 10, rows }], false);

test('first render, Live off and backward seeks show historical rows without arrival motion', () => {
  const items = logItems([row('old', 1), row('later', 11)], false);
  const first = logReveals(EMPTY_LOG_REVEALS, items, 10);
  assert.deepEqual([...first.entering], []);
  assert.deepEqual([...logReveals(first.history, items, null).entering], []);
  const forward = logReveals(first.history, items, 12);
  assert.deepEqual([...forward.entering], ['later']);
  assert.deepEqual([...logReveals(forward.history, items, 9).entering], []);
});

test('late observation animates at its supplying read, even with an older exact displayed date', () => {
  const previous = logReveals(EMPTY_LOG_REVEALS, [], 10).history;
  const items = logItems([row('late-observed', 3, 12), row('historical', 5), row('future', 20)], false);
  assert.deepEqual([...logReveals(previous, items, 15).entering], ['late-observed']);
});

test('async page loads at one advancing hour preserve the whole revelation interval', () => {
  const previous = logReveals(EMPTY_LOG_REVEALS, pageItems('new'), 10).history;
  const advancing = logReveals(previous, pageItems('new'), 15);
  assert.equal(advancing.history.from, 10);
  const late = logReveals(advancing.history, pageItems('new', [row('newly-reached', 12), row('old', 3)]), 15);
  assert.deepEqual([...late.entering], ['newly-reached']);
  const unloaded = logReveals(late.history, pageItems('new'), 15);
  const reloaded = logReveals(unloaded.history, pageItems('new', [row('newly-reached', 12)]), 15);
  assert.deepEqual([...reloaded.entering], []);
});

test('a reply arriving after the playhead moved animates the rows the retained one lacked', () => {
  const shown = logReveals(EMPTY_LOG_REVEALS, pageItems('0', [row('old', 3)]), 10);
  const retained = logReveals(shown.history, pageItems('0', [row('old', 3)]), 15);
  assert.deepEqual([...retained.entering], []);
  const fresh = logReveals(retained.history, pageItems('0', [row('new', 12), row('old', 3)]), 15);
  assert.deepEqual([...fresh.entering], ['new']);
  assert.deepEqual([...logReveals(fresh.history, pageItems('0', [row('new', 12), row('old', 3)]), 15).entering], [], 'presented once');
});

test('continuously loaded pages animate only newly reached rows and future advances stay eligible', () => {
  const first = logReveals(EMPTY_LOG_REVEALS, pageItems('same', [row('old', 3)]), 10);
  const forward = logReveals(first.history, pageItems('same', [row('new', 12), row('old', 3)]), 15);
  assert.deepEqual([...forward.entering], ['new']);
  const later = logReveals(forward.history, pageItems('same', [row('latest', 18), row('new', 12), row('old', 3)]), 20);
  assert.deepEqual([...later.entering], ['latest']);
});

test('logs without an explicit playhead keep ordinary row updates but suppress initial page loads', () => {
  assert.deepEqual([...logReveals(EMPTY_LOG_REVEALS, logItems([row('ordinary', 1)], false)).entering], ['ordinary']);
  const first = logReveals(EMPTY_LOG_REVEALS, pageItems('page', [row('history', 1)]));
  assert.deepEqual([...first.entering], []);
  assert.deepEqual([...logReveals(first.history, pageItems('page', [row('changed', 2)])).entering], ['changed']);
});

test('scrolling a newly revealed row away and back never repeats its arrival animation', () => {
  const items = logItems(Array.from({ length: 5000 }, (_, index) => row(`r${index}`, 12)), false);
  const eligible = logReveals(logReveals(EMPTY_LOG_REVEALS, [], 10).history, items, 15).entering;
  const first = logVisibleEntries(items, 0, 10, eligible, new Set());
  assert.equal(first.entering.size, 10);
  const away = logVisibleEntries(items, 100, 110, eligible, first.seen);
  assert.equal(away.entering.size, 10);
  const back = logVisibleEntries(items, 0, 10, eligible, away.seen);
  assert.equal(back.entering.size, 0);
  assert.equal(back.seen.size, 20);
  assert.equal(logVisibleEntries([], 0, 0, new Set(), back.seen).seen.size, 0);
});

test('an already mounted arrival completes through forward ticks and metadata updates, without a remount replay', () => {
  const items = logItems([row('arrived', 12), row('old', 3)], false);
  const first = logVisibleEntries(items, 0, 2, new Set(['arrived']), new Set());
  const nextTick = logVisibleEntries(items, 0, 2, new Set(), first.seen, first.entering, true);
  assert.deepEqual([...nextTick.entering], ['arrived']);
  const metadata = logVisibleEntries(items.map(item => ({ ...item })), 0, 2, new Set(), nextTick.seen, nextTick.entering, true);
  assert.deepEqual([...metadata.entering], ['arrived']);
  const off = logVisibleEntries(items, 0, 2, new Set(), metadata.seen, metadata.entering, false);
  assert.equal(off.entering.size, 0);
  const away = logVisibleEntries(items, 1, 2, new Set(), metadata.seen, metadata.entering, true);
  const back = logVisibleEntries(items, 0, 2, new Set(['arrived']), away.seen, away.entering, true);
  assert.equal(back.entering.size, 0);
});


test('arrival waits for both the exact displayed date and the supplying read', () => {
  const initial = logReveals(EMPTY_LOG_REVEALS, [], 5).history;
  const items = logItems([row('future-date', 20, 10)], false);
  const read = logReveals(initial, items, 10);
  assert.deepEqual([...read.entering], []);
  assert.deepEqual([...logReveals(read.history, items, 20).entering], ['future-date']);
});

test('unchanged intervals retain reveal identity and crossings visit only indexed boundaries', () => {
  let reads = 0;
  const rows = Array.from({ length: 5000 }, (_, index) => ({ key: `r${index}`, item: '',
    get hour() { reads++; return index * 10; } }));
  const items = deepFreeze(logItems(rows, true));
  const first = revealAt(EMPTY_LOG_REVEALS, items, 10);
  let history = logRevealHistory(EMPTY_LOG_REVEALS, first, 10);
  reads = 0;
  const inside = revealAt(history, items, 19);
  assert.equal(inside, first);
  history = logRevealHistory(history, inside, 19);
  const crossed = revealAt(history, items, 40);
  assert.notEqual(crossed, first);
  assert.deepEqual([...crossed.entering], ['r2', 'r3', 'r4']);
  assert.equal(reads, 0, 'forward playback never reads the item rows again');
  history = logRevealHistory(history, crossed, 40);
  assert.equal(revealAt(history, items, 49), crossed);
  const backward = revealAt(history, items, 9);
  assert.notEqual(backward, crossed);
  assert.equal(backward.entering.size, 0);
});

test('an async load after unchanged intervals uses the last committed tick and preserves page history', () => {
  const items = deepFreeze(pageItems('page', [row('old', 3)]));
  const first = revealAt(EMPTY_LOG_REVEALS, items, 10);
  const history = logRevealHistory(logRevealHistory(EMPTY_LOG_REVEALS, first, 10), first, 14);
  const advanced = revealAt(history, items, 15);
  assert.equal(advanced, first);
  const committed = logRevealHistory(history, advanced, 15);
  const loaded = logReveals(committed, pageItems('page', [row('too-old', 12), row('new', 15), row('old', 3)]), 15);
  assert.deepEqual([...loaded.entering], ['new']);
  assert.equal(loaded.history.from, 14);
  assert.deepEqual([...loaded.history.presented.get('page')!].sort(), ['new', 'old', 'too-old']);
});

test('replacing frozen log rows updates their supplying-read boundary', () => {
  const items = deepFreeze(logItems([row('row', 3, 20)], false));
  const first = logReveals(EMPTY_LOG_REVEALS, items, 10);
  const item = items.find(item => item.kind === 'row');
  assert.ok(item?.kind === 'row');
  assert.throws(() => Object.assign(item.row, { revealedHour: 12 }), TypeError);
  const replaced = deepFreeze(logItems([row('row', 3, 12)], false));
  const next = logReveals(first.history, replaced, 12);
  assert.deepEqual([...next.entering], ['row']);
});
