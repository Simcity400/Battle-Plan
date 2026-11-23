import assert from 'node:assert/strict';
import { test } from 'node:test';
import { deriveTableAudit, resolveTableRows, type TableColumnSpec, type TableModel } from '../../families/table/model.ts';
import { OFFERED, tableDefaults } from '../../families/table/settings.ts';
import { knownValue } from '../../families/value.ts';
import { panelAuditRows } from './panelAudit.ts';

const cols: readonly TableColumnSpec[] = [
  { key: 'v', label: 'Value', as: 'plain', offered: OFFERED.number, priority: 0, numeric: true, format: 'integer' },
];
const settings = tableDefaults(cols, { density: 'comfortable', overflow: 'tooltip', sort: { column: 'v', direction: 'descending' } });

test('review 4 finding 15: a table reader compares the values shown, not each country with itself', () => {
  const modelOf = (value: number): TableModel => ({
    columns: cols,
    rows: [{ key: 'GER', name: 'GER', cells: { v: { value: knownValue(value), format: 'integer' } } }],
    hour: 10,
  });
  const shown = deriveTableAudit(resolveTableRows(modelOf(10), settings), cols, 10);
  const exact = deriveTableAudit(resolveTableRows(modelOf(20), settings), cols, 10);
  assert.deepEqual(shown.find(row => row.key === 'GER:v')?.value, JSON.stringify(['known', '10']));
  assert.notEqual(shown.find(row => row.key === 'GER:v')?.value, exact.find(row => row.key === 'GER:v')?.value, 'shown 10 against an exact 20 is a difference the auditor sees');
  assert.deepEqual(deriveTableAudit([], cols, 10, 'No states are occupied'), [
    { row: '__columns__', key: '__columns__:visible', value: JSON.stringify(['v']) },
    { row: 'empty', key: 'empty:notice', value: JSON.stringify(['empty', 'No states are occupied']) },
  ]);
});

test('review 4 finding 15: a panel reader compares live values as well as structure, so two products with empty structure still differ', () => {
  const shown = panelAuditRows('no-such-product', {}, new Map([['GER\u0000total', 10]]));
  const exact = panelAuditRows('no-such-product', {}, new Map([['GER\u0000total', 20]]));
  assert.deepEqual([shown.length, shown[0]!.value === exact[0]!.value], [1, false]);
});
