import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createElement, isValidElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { cellContent, cellText, columnAlign, growColumnWidths, hideOrder, tableGrid, withHidden } from './columns.ts';
import type { TableColumn } from './columns';
import { densityRowHeight, rowOffsets, sortRows, visibleRange } from './logic.ts';
import { acesFixture, generalsFixture } from './military.fixture.ts';

test('content tracks share mounted widths without letting long names force a table wider', () => {
  const columns: TableColumn<string>[] = [
    { key: 'name', header: 'Name', label: 'Name', value: row => row, width: 'minmax(0, 2fr)' },
    { key: 'value', header: 'Value', label: 'Value', value: () => 0, numeric: true, width: 'max-content' },
  ];
  const initial = new Map<string, number>();
  const measured = growColumnWidths(initial, [{ key: 'value', width: 81.2 }, { key: 'value', width: 120 }, { key: 'value', width: NaN }]);
  assert.equal(measured.get('value'), 120);
  assert.equal(initial.size, 0);
  assert.equal(tableGrid(columns, true, measured), 'var(--table-rank-width) minmax(0, 2fr) 120px');
  assert.equal(growColumnWidths(measured, [{ key: 'value', width: 70 }, { key: 'value', width: -1 }]), measured);
  const later = growColumnWidths(measured, [{ key: 'value', width: 140.1 }]);
  assert.equal(later.get('value'), 141);
  assert.equal(tableGrid(columns, false, later), 'minmax(0, 2fr) 141px');
});

test('missing values override custom renderers unless opted in, regardless of numeric or alignment', () => {
  let calls = 0;
  const column: TableColumn<number | null | undefined> = { key: 'value', header: '', label: 'Value', value: row => row,
    render: row => { calls++; return createElement('b', null, String(row)); } };
  for (const numeric of [false, true]) {
    for (const value of [null, undefined, NaN, Infinity, -Infinity]) {
      assert.equal(cellContent({ ...column, numeric }, value), '—');
    }
  }
  assert.equal(calls, 0);
  assert.ok(isValidElement(cellContent(column, 0)));
  assert.ok(isValidElement(cellContent({ ...column, renderMissing: true }, null)));
  assert.equal(calls, 2);
  assert.equal(cellContent({ ...column, render: undefined, renderMissing: true }, null), '—');
  assert.equal(columnAlign(column), 'start');
  assert.equal(columnAlign({ numeric: true }), 'end');
  assert.equal(columnAlign({ numeric: true, align: 'start' }), 'start');
  assert.equal(columnAlign({ align: 'end' }), 'end');
});

test('Military story: portrait/name/flag/subtitle, stars, mini-bars and clipped traits use the same columns', () => {
  const row = generalsFixture.rows[0];
  const cells = generalsFixture.columns.map(column => renderToStaticMarkup(createElement('td', null, cellContent(column, row))));
  assert.match(cells[0], /recorded-portrait-z/);
  assert.match(cells[1], /Recorded leader Z.*recorded-flag-z.*Recorded rank/);
  assert.match(cells[2], /★★★★/);
  assert.equal(cells[3].match(/<meter/g)?.length, 4);
  assert.match(cells[4], /flex-wrap:nowrap;overflow:hidden/);
  assert.equal(generalsFixture.columns[0].header, null);
  assert.equal(generalsFixture.columns[0].label, 'Portrait');
  assert.equal(generalsFixture.columns[0].sortable, false);
});

test('Military story: muted aces retain their rank; custom row heights scale for CSS and windowing', () => {
  const skill = generalsFixture.columns[2];
  const ordered = sortRows([...generalsFixture.rows].reverse(), skill.value, 'descending', generalsFixture.rowKey);
  assert.equal(ordered[0].key, 'leader-a');
  assert.equal(generalsFixture.muted(ordered[0]), true);
  assert.equal(generalsFixture.muted(ordered[1]), false);
  assert.equal(ordered.length, generalsFixture.rows.length);
  assert.equal(densityRowHeight(generalsFixture.rowHeight, 'comfortable', 25 / 34), 48);
  const compact = densityRowHeight(acesFixture.rowHeight, 'compact', 25 / 34);
  assert.equal(compact, 50 * (25 / 34));
  const offsets = rowOffsets(5000, compact, 26);
  assert.equal(offsets[101], 26 + 101 * compact);
  assert.deepEqual(visibleRange(offsets, offsets[100], compact * 4.5, 0), [100, 105]);
});

test('hidden columns hide largest priority first and give the tooltip their cell text', () => {
  const columns: TableColumn<{ name: string; amount: number | null }>[] = [
    { key: 'name', header: 'Name', label: 'Name', value: row => row.name },
    { key: 'a', header: 'A', label: 'A', priority: 1, value: row => row.amount },
    { key: 'b', header: 'B', label: 'B', priority: 2, value: row => row.amount, render: row => `${row.amount} units` },
    { key: 'c', header: 'C', label: 'C', priority: 2, value: row => row.amount, render: () => createElement('b'), text: row => `text ${row.amount}` },
    { key: 'd', header: 'D', label: 'D', priority: Number.NaN, value: () => 'x' },
  ];
  assert.deepEqual(hideOrder(columns), ['c', 'b', 'a']);
  const row = { name: 'Recorded', amount: 12345 };
  assert.deepEqual(columns.map(column => cellText(column, row)), ['Recorded', '12,345', '12345 units', 'text 12345', 'x']);
  assert.deepEqual(columns.slice(1, 3).map(column => cellText(column, { name: '', amount: null })), ['—', '—'], 'missing stays a dash');

  const lostColumn: TableColumn<{ name: string; amount: number | null }> = {
    key: 'lost', header: 'Lost', label: 'Lost', numeric: true, priority: 2, value: () => null,
    paint: value => String(value), missingText: (_key, reason) => reason === 'Not in this agreement' ? '—' : 'Not recorded',
  };
  const liveValues = {
    get: () => undefined,
    missing: (key: string) => key.startsWith('unread') ? 'Fuel lost not recorded at this hour' : 'Not in this agreement',
  };
  const unreadTip = withHidden(null, row, [lostColumn], [columns[0]], 'unread', 800, liveValues)!;
  assert.deepEqual(unreadTip.rows, [{ label: 'Lost', value: 'Fuel lost not recorded at this hour', detail: 'Fuel lost not recorded at this hour', tone: 'muted', divider: false }]);
  const absentTip = withHidden(null, row, [lostColumn], [columns[0]], 'absent', 800, liveValues)!;
  assert.deepEqual(absentTip.rows, [{ label: 'Lost', value: 'Not in this agreement', detail: 'Not in this agreement', tone: 'muted', divider: false }]);

  const defaultDashColumn: TableColumn<{ name: string; amount: number | null }> = {
    key: 'deliveries', header: 'Deliveries', label: 'Completed deliveries', numeric: true, priority: 2, value: () => null,
    paint: value => String(value),
  };
  const dashUnreadTip = withHidden(null, row, [defaultDashColumn], [columns[0]], 'unread', 800, liveValues)!;
  assert.deepEqual(dashUnreadTip.rows, [{ label: 'Completed deliveries', value: 'Fuel lost not recorded at this hour', detail: 'Fuel lost not recorded at this hour', tone: 'muted', divider: false }]);
  const spokenTip = withHidden({ title: 'Recorded', rows: [], spoken: 'Recorded' }, row, [defaultDashColumn], [columns[0]], 'unread', 800, liveValues)!;
  assert.equal(spokenTip.spoken, 'Recorded; Completed deliveries Fuel lost not recorded at this hour', 'the reason is spoken, not a dash');
  const noReasonTip = withHidden(null, row, [defaultDashColumn], [columns[0]], 'none', 800, { get: () => undefined, missing: () => '' })!;
  assert.deepEqual(noReasonTip.rows, [{ label: 'Completed deliveries', value: '—', divider: false }]);
});
