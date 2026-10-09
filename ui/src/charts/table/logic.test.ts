import assert from 'node:assert/strict';
import { test } from 'node:test';
import { commitRows, EMPTY_HISTORY, shownRows } from '../rowMotion.ts';
import { barFraction, barMaximum, groupedRows, nextSort, rankChanges, rowOffsets, scrollToShow, shownMotion, sortRows, visibleRange } from './logic.ts';
import { sortKey, tableSortOf, tableSortOptions, TABLE_OPTIONS } from './options.ts';
import { resolveOptions, setOption } from '../../dashboard/options.ts';

test('missing values sort last both ways; large integers and ties stay exact', () => {
  const values = [null, 9007199254740993n, 0n, undefined, 9007199254740992n];
  assert.deepEqual(sortRows(values, v => v, 'ascending', String), [0n, 9007199254740992n, 9007199254740993n, null, undefined]);
  assert.deepEqual(sortRows(values, v => v, 'descending', String), [9007199254740993n, 9007199254740992n, 0n, null, undefined]);
  assert.deepEqual(sortRows([NaN, 0], v => v, 'descending', String), [0, NaN]);
  assert.deepEqual(sortRows([10n ** 400n + 1n, 10n ** 400n, 1n], v => v, 'ascending', String), [1n, 10n ** 400n, 10n ** 400n + 1n]);
  assert.deepEqual(sortRows(['2', '10'], v => v, 'ascending', String), ['2', '10']);
  assert.deepEqual(nextSort({ column: 'score', direction: 'descending' }, 'score'), { column: 'score', direction: 'ascending' });
});

test('string values use locale-aware numeric ordering; equal values still tie by deterministic key', () => {
  assert.deepEqual(sortRows(['Zambia', 'Ägypten'], v => v, 'ascending', String), ['Ägypten', 'Zambia']);
  assert.deepEqual(sortRows(['Zambia', 'Ägypten'], v => v, 'descending', String), ['Zambia', 'Ägypten']);
  assert.deepEqual(sortRows(['State 10', 'State 2', 'State 1'], v => v, 'ascending', String), ['State 1', 'State 2', 'State 10']);
});

test('changing either sort column or direction resets the rank history before value changes', () => {
  const initial = { column: 'score', direction: 'descending' } as const;
  const before = rankChanges(new Map(), ['a', 'b', 'c'], 0);
  for (const after of [{ column: 'score', direction: 'ascending' }, { column: 'name', direction: 'descending' }] as const) {
    const reset = rankChanges(before, ['c', 'b', 'a'], 100, 2500, sortKey(initial) !== sortKey(after));
    assert.deepEqual([...reset.values()].map(item => item.delta), [0, 0, 0]);
    assert.equal(rankChanges(reset, ['b', 'c', 'a'], 200).get('b')?.delta, 1);
  }
});

test('equal values, including missing values, use deterministic string keys across shuffled updates', () => {
  const rows = [
    { key: 'z', value: 7n }, { key: 'a', value: 7n }, { key: '10', value: 7 }, { key: '2', value: 7n },
    { key: 'missing-z', value: null }, { key: 'missing-a', value: NaN },
  ];
  for (const direction of ['ascending', 'descending'] as const) {
    for (const input of [rows, [...rows].reverse(), [...rows.slice(2), ...rows.slice(0, 2)]]) {
      assert.deepEqual(sortRows(input, row => row.value, direction, row => row.key).map(row => row.key),
        ['10', '2', 'a', 'z', 'missing-a', 'missing-z']);
    }
  }
  assert.deepEqual(sortRows([{ key: 'z', value: 'same' }, { key: 'a', value: 'same' }], row => row.value, 'descending', row => row.key).map(row => row.key), ['a', 'z']);
});

test('bars fit the filtered rows or a declared fixed scale; unknown never fills', () => {
  assert.equal(barMaximum([null, 7n, 12n, -1n], v => v), 12n);
  assert.equal(barFraction(3n, 12n), .25);
  assert.equal(barFraction(150n, 100n), 1);
  assert.equal(barFraction(null, 100n), 0);
  assert.equal(barFraction(NaN, 100n), 0);
  assert.equal(barFraction(-1n, 100n), 0);
  assert.equal(barFraction(1n, 0n), 0);
});

test('rank markers last 2.5 seconds and a returning row has no invented delta', () => {
  let ranks = rankChanges(new Map(), ['a', 'b', 'c'], 0);
  ranks = rankChanges(ranks, ['b', 'c', 'a'], 100);
  assert.equal(ranks.get('b')?.delta, 1);
  assert.equal(ranks.get('a')?.delta, -2);
  assert.equal(rankChanges(ranks, ['b', 'c', 'a'], 2599).get('a')?.delta, -2);
  assert.equal(rankChanges(ranks, ['b', 'c', 'a'], 2600).get('a')?.delta, 0);
  ranks = rankChanges(ranks, ['a'], 200);
  assert.equal(rankChanges(ranks, ['a', 'b'], 300).get('b')?.delta, 0);
});

test('windowing includes an overscan, clamps its ends, and keeps absolute slots', () => {
  const offsets = rowOffsets(5000, 34);
  assert.deepEqual(visibleRange(offsets, 34000, 340), [994, 1016]);
  assert.deepEqual(visibleRange(rowOffsets(4, 34), -10, 100), [0, 4]);
  assert.deepEqual(visibleRange(offsets, 999999, 340), [4993, 5000]);
  assert.deepEqual(visibleRange(rowOffsets(0, 34), 0, 100), [0, 0]);
  const rows = [
    { key: 'slide-out', slot: 100, phase: 'staying' as const, since: 0, item: 1 },
    { key: 'shown', slot: 1, phase: 'staying' as const, since: 0, item: 2 },
    { key: 'offscreen', slot: 200, phase: 'staying' as const, since: 0, item: 3 },
    { key: 'gone', slot: 2, phase: 'leaving' as const, since: 0, item: 4 },
  ];
  assert.deepEqual(shownMotion(rows, 0, 10, true, row => row.key === 'slide-out'), [rows[0], rows[1], rows[3]]);
  assert.deepEqual(shownMotion(rows, 0, 10, false), [rows[1]]);
  assert.deepEqual(shownMotion(rows, 0, 10, false, row => row.key === 'offscreen'), [rows[1], rows[2]]);
});

test('keyboard scrolling shows the whole row below the sticky header with minimal movement', () => {
  const offsets = rowOffsets(5000, 48, 26);
  assert.equal(scrollToShow(offsets, 10, 400, 240, 26), 400);
  assert.equal(scrollToShow(offsets, 8, 400, 240, 26), 384);
  assert.equal(scrollToShow(offsets, 13, 400, 240, 26), 458);
  assert.equal(scrollToShow(offsets, 0, 400, 240, 26), 0);
});

test('one atomic widget option stores column and direction, with shared defaults', () => {
  const initial = { column: 'state', direction: 'ascending' } as const;
  const fields = [...TABLE_OPTIONS, ...tableSortOptions([{ key: 'state', label: 'State' }, { key: 'value', label: 'Value' }, { key: 'portrait', label: 'Portrait', sortable: false }], initial)];
  const sortField = fields.at(-1);
  assert.equal(sortField?.kind === 'choice' && sortField.choices.length, 4);
  const next = { column: 'value', direction: 'descending' } as const;
  const saved = setOption(fields, undefined, 'tableSort', sortKey(next));
  assert.deepEqual(tableSortOf(resolveOptions(fields, saved!).tableSort, initial), next);
  assert.equal(resolveOptions(fields).cellBars, true);
  assert.equal(resolveOptions(fields).rankMarkers, true);
  assert.equal(resolveOptions(fields).density, 'comfortable');
  assert.deepEqual(tableSortOf('broken', initial), initial);
});

test('a 5,000-row windowed redraw mounts only the rows in view and those sliding through it', () => {
  const rows = Array.from({ length: 5000 }, (_, id) => ({ key: String(id), value: String((id * 7919) % 5003) }));
  let history = commitRows(EMPTY_HISTORY, rows.map(row => ({ key: row.key, item: row })), 0, 450);
  const retained = new Set(rows.slice(992, 1028).map(row => row.key));
  let drawn = 0;
  // Sorted one way, then redrawn sorted the other: every row slides.
  for (let i = 0; i < 2; i++) {
    const sorted = sortRows(rows, row => BigInt(row.value), i % 2 ? 'ascending' : 'descending', row => row.key);
    const items = sorted.map(row => ({ key: row.key, item: row }));
    const moving = shownRows(history, items, i, 450);
    history = commitRows(history, items, i, 450);
    const [start, end] = visibleRange(rowOffsets(rows.length, 34, 26), 34000, 680);
    drawn = shownMotion(moving, start, end, true, row => retained.has(row.key)).length;
  }
  assert.ok(drawn <= 72);
});

test('fixed header choices keep their direction across repeated clicks and reversed restored sorts', () => {
  for (const direction of ['ascending', 'descending'] as const) {
    const sort = { column: 'date', direction };
    assert.deepEqual(nextSort(sort, 'date', 'ascending', 'descending'), { column: 'date', direction: 'descending' });
    assert.deepEqual(nextSort(sort, 'name', 'descending', 'ascending'), { column: 'name', direction: 'ascending' });
  }
  assert.deepEqual(nextSort({ column: 'name', direction: 'ascending' }, 'name'), { column: 'name', direction: 'descending' });
});

test('faction grouping keeps native identities separate and exact missing-last column order inside them', () => {
  const rows = [
    { id: 'none', tag: 'none', group: { key: 'none', label: 'No faction', order: 3 }, value: 99n },
    { id: 'unknown', tag: 'unknown', group: { key: 'unknown', label: 'Faction not recorded', order: 2 }, value: 100n },
    { id: 'native-a', tag: 'z', group: { key: 'faction:2', label: 'Same name', orderKey: 2n }, value: 9007199254740992n },
    { id: 'native-b', tag: 'a', group: { key: 'faction:2', label: 'Same name', orderKey: 2n }, value: 9007199254740993n },
    { id: 'native-c', tag: 'c', group: { key: 'faction:10', label: 'Same name', orderKey: 10n }, value: 9999999999999999n },
    { id: 'missing', tag: 'missing', group: { key: 'faction:2', label: 'Same name', orderKey: 2n }, value: null },
    { id: 'negative', tag: 'negative', group: { key: 'faction:2', label: 'Same name', orderKey: 2n }, value: -1n },
    { id: 'unnamed', tag: 'unnamed', group: { key: 'faction:30', label: 'Faction name unavailable', order: 1, orderKey: 30n }, value: 1n },
  ];
  for (const input of [rows, [...rows].reverse()]) {
    const order = (direction: 'ascending' | 'descending') => groupedRows(input, row => row.group,
      row => row.value, direction, row => row.id, row => row.tag).map(row => row.id);
    assert.deepEqual(order('descending'), ['native-b', 'native-a', 'negative', 'missing', 'native-c', 'unnamed', 'unknown', 'none']);
    assert.deepEqual(order('ascending'), ['negative', 'native-a', 'native-b', 'missing', 'native-c', 'unnamed', 'unknown', 'none']);
  }
  const ties = rows.slice(2, 4).map(row => ({ ...row, value: 1n }));
  assert.deepEqual(groupedRows(ties, row => row.group, row => row.value, 'descending', row => row.id, row => row.tag)
    .map(row => row.id), ['native-b', 'native-a']);
});

test('dated faction changes move existing country keys through the shared row motion', () => {
  const rows = [
    { key: 'native-country-a', group: { key: 'faction:1', label: 'Z' }, value: 2n },
    { key: 'native-country-b', group: { key: 'faction:2', label: 'A' }, value: 1n },
  ];
  const items = (input: typeof rows) => groupedRows(input, row => row.group, row => row.value,
    'descending', row => row.key).map(row => ({ key: row.key, item: row }));
  const initial = items(rows);
  assert.deepEqual(initial.map(row => row.key), ['native-country-b', 'native-country-a']);
  const history = commitRows(EMPTY_HISTORY, initial, 0, 450);
  const changed = items([{ ...rows[0], group: rows[1].group }, rows[1]]);
  assert.deepEqual(changed.map(row => row.key), ['native-country-a', 'native-country-b']);
  assert.ok(shownRows(history, changed, 100, 450).every(row => row.phase === 'staying'));
});
