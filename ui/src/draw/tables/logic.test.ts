import test from 'node:test';
import assert from 'node:assert/strict';
import { HEADER_CARET_LANE, amountCell, displayExtra, endAligned, formatTableNumber, hitTable, layoutTable, percentCell, preserveRevealTimes, sortTableRows, revealedRows, ROW_HEIGHT } from './logic.ts';

const columns = [
  { key: 'state', label: 'State', priority: 0, width: 140, shown: true, display: 'plain' as const },
  { key: 'resistance', label: 'Resistance', priority: 1, width: 130, shown: true, display: 'meterAfter' as const, numeric: true },
  { key: 'owner', label: 'Owner', priority: 6, width: 140, shown: true, display: 'plain' as const },
];

test('hide overflow moves the lowest-priority columns to the tooltip', () => {
  const fitted = layoutTable({ columns, available: 280 }, 'tooltip');
  assert.deepEqual(fitted.columns.map(column => column.key), ['state', 'resistance']);
  assert.deepEqual(fitted.hidden, ['owner']);
});

test('scroll overflow keeps all tracks and pins the first column', () => {
  const fitted = layoutTable({ columns, available: 250 }, 'scroll');
  assert.equal(fitted.scrollable, true);
  assert.equal(fitted.pinnedWidth, columns[0]!.width);
});

test('sorting keeps missing values last and equal values stable', () => {
  const sorted = sortTableRows([
    { key: '2', values: { resistance: null } }, { key: '1', values: { resistance: 10 } }, { key: '3', values: { resistance: 10 } },
  ], 'resistance', 'descending');
  assert.deepEqual(sorted.map(row => row.key), ['1', '3', '2']);
});

test('sorting preserves the input order for equal values', () => {
  const sorted = sortTableRows([{ key: 'b', values: { resistance: 10 } }, { key: 'a', values: { resistance: 10 } }], 'resistance', 'descending');
  assert.deepEqual(sorted.map(row => row.key), ['b', 'a']);
});

test('structure updates retain existing row reveal hours', () => {
  const rows = preserveRevealTimes([{ key: 'old', revealAt: 30 }, { key: 'new', revealAt: 50 }], [{ key: 'old', revealAt: 10 }]);
  assert.deepEqual(rows.map(row => row.revealAt), [10, 50]);
});

test('reveal and pinned hit testing use the presented row set', () => {
  const rows = revealedRows([{ key: 'old', revealAt: 10, values: {} }, { key: 'new', revealAt: 20, values: {} }], 15);
  assert.deepEqual(rows.map(row => row.key), ['old']);
  const fit = layoutTable({ columns, available: 250 }, 'scroll');
  assert.deepEqual(hitTable(fit, rows, { x: 20, y: 30 }, ROW_HEIGHT.comfortable, 28, 180), { kind: 'row', rowKey: 'old', column: 'state' });
  assert.equal(hitTable(fit, rows, { x: columns[0]!.width + HEADER_CARET_LANE, y: 30 }, ROW_HEIGHT.comfortable, 28, 180).rowKey, 'old');
  assert.equal(hitTable(fit, [{ key: 'old', values: {} }, { key: 'next', values: {} }], { x: 20, y: 30 }, ROW_HEIGHT.comfortable, 28, 0, false, 28, 28).rowKey, 'next');
});

test('cell numbers follow the game: percentages round half away and clamp, amounts truncate to two decimals', () => {
  assert.deepEqual(['4449999', '4450000', '12000000', '-1'].map(raw => formatTableNumber(Number(raw), 'fixed5Percent')), ['44%', '45%', '100%', '0%']);
  assert.deepEqual([173_611, 0, -5, 99_999].map(raw => formatTableNumber(raw, 'fixed5Amount')), ['1.73', '-', '!', '0.99'], 'truncated, not rounded; none "-", unmet "!"');
});

test('displays reserve their room and numbers align right unless their bar leads', () => {
  assert.deepEqual(['plain', 'faint', 'meterAfter', 'flagName', 'flag', 'name', 'dot'].map(display => displayExtra({ display: display as never })), [0, 4, 52, 26, 20, 0, 12]);
  assert.deepEqual(['plain', 'meterAfter', 'track', 'meterBefore'].map(display => endAligned({ numeric: true, display: display as never })), [true, true, false, false]);
  assert.equal(endAligned({ numeric: false, display: 'plain' }), false);
});

test('the Number arithmetic for cell text matches the exact rules everywhere a game records', () => {
  const values = [0, 1, -1, 49_999, 50_000, -50_000, -50_001, 99_999, 100_000, 4_449_999, 4_450_000, 9_950_000, 10_000_000, 12_345_678, 173_611, 2 ** 39, -(2 ** 39)];
  for (let seed = 1; seed < 400; seed++) values.push(Math.round(Math.sin(seed) * 10 ** (seed % 12)));
  for (const value of values) {
    assert.equal(formatTableNumber(value, 'fixed5Percent'), percentCell(BigInt(value)), `percent ${value}`);
    assert.equal(formatTableNumber(value, 'fixed5Amount'), amountCell(BigInt(value)), `amount ${value}`);
  }
});
