import assert from 'node:assert/strict';
import { test } from 'node:test';
import { buildColumnLayouts } from '../../families/table/layout.ts';
import { deriveTableAudit, resolveTableRows, type TableColumnSpec, type TableModel } from '../../families/table/model.ts';
import { OFFERED, tableDefaults } from '../../families/table/settings.ts';
import { knownValue } from '../../families/value.ts';
import { panelAuditRows } from './panelAudit.ts';

const cols: readonly TableColumnSpec[] = [
  { key: 'v', label: 'Value', as: 'plain', offered: OFFERED.number, priority: 0, numeric: true, format: 'integer' },
];
const settings = tableDefaults(cols, { density: 'comfortable', sort: { column: 'v', direction: 'descending' } });
const layouts = buildColumnLayouts(cols, settings);

test('review 4 finding 15: a table reader compares the values shown, not each country with itself', () => {
  const modelOf = (value: number): TableModel => ({
    columns: cols,
    rows: [{ key: 'GER', name: 'GER', cells: { v: { value: knownValue(value), format: 'integer' } } }],
    hour: 10,
  });
  const m10 = modelOf(10);
  const m20 = modelOf(20);
  const shown = deriveTableAudit({ model: m10, flatRows: resolveTableRows(m10, settings), visibleColumns: layouts });
  const exact = deriveTableAudit({ model: m20, flatRows: resolveTableRows(m20, settings), visibleColumns: layouts });
  assert.ok(shown.find(row => row.key === 'GER:v')?.value?.includes('["known",["int","10"]]'));
  assert.notEqual(shown.find(row => row.key === 'GER:v')?.value, exact.find(row => row.key === 'GER:v')?.value, 'shown 10 against an exact 20 is a difference the auditor sees');
  const emptyModel: TableModel = { columns: cols, rows: [], hour: 10, empty: 'No states are occupied', emptyKind: 'empty' };
  const emptyAudit = deriveTableAudit({ model: emptyModel, flatRows: [], visibleColumns: layouts });
  assert.equal(emptyAudit.length, 2);
  assert.equal(emptyAudit[0]!.key, '__table__:columns');
  assert.equal(emptyAudit[1]!.key, '__table__:meta');
  assert.ok(emptyAudit[1]!.value?.includes('No states are occupied'));
});

test('review 4 finding 15: a panel reader compares live values as well as structure, so two products with empty structure still differ', () => {
  const shown = panelAuditRows('no-such-product', {}, new Map([['GER\u0000total', 10]]));
  const exact = panelAuditRows('no-such-product', {}, new Map([['GER\u0000total', 20]]));
  assert.deepEqual([shown.length, shown[0]!.value === exact[0]!.value], [1, false]);
});
