import test from 'node:test';
import assert from 'node:assert/strict';
import { historyRuns, minefieldsStructure, minesText, orderRegions, ownerMines, regionRows, regionTotalText, segmentWidths, validateMinefields } from './minefieldsData.ts';
import { liveKey, valuesOf } from '../../charts/live.ts';
import { repliedCells } from '../../dashboard/follow/liveCells.ts';

const reply = validateMinefields({
  regions: [{ region: 3, owners: [{ tag: 'ENG' }, { tag: 'GER' }], unknownOwners: 1 }, { region: 9, owners: [{ tag: 'GER' }], unknownOwners: 0 }],
  unknown: [{ region: 5, unknownOwners: 2 }],
  choices: [{ id: '3', name: 'North Sea' }, { id: '5', name: 'Baltic' }, { id: '7', name: 'Channel' }, { id: '9', name: 'Adriatic' }],
  scale: 100000, warnings: [],
});
const cells = (amounts: Record<string, number>) => valuesOf(new Map(Object.entries(amounts)), 'Not recorded at this hour');
const at = cells({ [liveKey('3', 'ENG')]: 900000, [liveKey('3', 'GER')]: 500000, [liveKey('3', 'total')]: 1400000,
  [liveKey('9', 'GER')]: 700000, [liveKey('9', 'total')]: 700000 });

test('every mined region is a row; a region only unknown is counted unlisted unless chosen', () => {
  const all = regionRows(reply);
  assert.deepEqual(all.rows.map(row => [row.region, row.name]), [['3', 'North Sea'], ['9', 'Adriatic']]);
  assert.equal(regionTotalText(all.rows[0], at, 100000), '14');
  assert.equal(all.unlisted, 1);
  const chosen = regionRows(reply, ['5', '7']);
  assert.deepEqual(chosen.rows.map(row => [row.region, row.owners.length, row.unknownOwners]), [['5', 0, 2]], 'a chosen region known empty is not listed');
  assert.equal(regionTotalText(chosen.rows[0], at, 100000), '\u2014', 'no owner known there is a dash, never a zero');
  assert.equal(chosen.unlisted, 0);
});

test('amounts are painted from the live cells: they move without changing what renders, and unknown is never zero', () => {
  const later = structuredClone(reply);
  assert.equal(minefieldsStructure(later), minefieldsStructure(reply));
  later.regions.reverse();
  assert.notEqual(minefieldsStructure(later), minefieldsStructure(reply), 'a change of order renders');
  const { rows } = regionRows(reply);
  assert.deepEqual(segmentWidths(rows, at).get('3'), [64.28, 35.71], 'segments share the greatest total\'s scale');
  assert.deepEqual(segmentWidths(rows, at).get('9'), [50]);
  const unread = cells({});
  assert.equal(regionTotalText(rows[0], unread, 100000), 'Not recorded at this hour');
  assert.deepEqual(segmentWidths(rows, unread).get('3'), [0, 0]);
});

test('mines read exactly on their scale; regions order by most mines or by name', () => {
  assert.equal(minesText(1234567n, 100000), '12.34567');
  assert.equal(minesText(-100000n, 100000), '−1');
  const rows = [...regionRows(reply, ['3', '5']).rows];
  assert.deepEqual(orderRegions(rows, 'name').map(row => row.name), ['Baltic', 'North Sea']);
  assert.deepEqual(orderRegions(rows, 'mines').map(row => row.name), ['North Sea', 'Baltic']);
});

test('an unknown stretch of history breaks the line and draws nothing', () => {
  assert.deepEqual(historyRuns({ country: 'GER', runs: [[0, 9, '0'], [10, 19, null], [20, 29, '200000']] }, 100000),
    [{ firstHour: 0, lastHour: 9, value: 0, breakBefore: true }, { firstHour: 20, lastHour: 29, value: 2, breakBefore: true }]);
});

test('a cold truncated minefields reply keeps witnessed owners live and reports the truncation reason for the region total', () => {
  const tableTrunc = 'Naval regional mines read only in part at this hour (1 rows not read), and which rows were read is not held at the head';
  const truncatedReply = validateMinefields({
    regions: [{ region: 3, owners: [{ tag: 'GER' }], unknownOwners: 1 }],
    unknown: [],
    choices: [{ id: '3', name: 'North Sea' }],
    scale: 1,
    warnings: [],
  });
  const live = {
    rows: ['3'],
    kinds: [{ id: 'GER', op: 'value', scale: 1 }, { id: 'total', op: 'sum', scale: 1 }],
    groups: [
      { table: 1, columns: [0], keys: ['1'], values: [40] },
      { table: 1, columns: [0], keys: ['1'], values: [tableTrunc] },
    ],
    cells: [0, 0, 1, 0, 0, 0, 0, 1, 1, 1, 0, 0],
  };
  const values = repliedCells({ ...truncatedReply, live }).read();
  const { rows } = regionRows(truncatedReply);
  assert.equal(ownerMines(values, '3', 'GER'), 40n);
  assert.equal(regionTotalText(rows[0]!, values, 1), tableTrunc);
});
