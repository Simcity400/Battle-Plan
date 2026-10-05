import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createElement, isValidElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { cellContent, columnAlign, growColumnWidths, tableGrid } from './columns.ts';
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
