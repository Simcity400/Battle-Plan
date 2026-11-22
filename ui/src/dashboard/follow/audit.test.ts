import assert from 'node:assert/strict';
import { test } from 'node:test';
import { auditRows } from '../../draw/views/table.ts';
import { panelAuditRows } from './panelAudit.ts';
import type { TableRowModel, TableValues } from '../../draw/tables/surface.ts';

const values = (value: number): TableValues => ({ get: () => value, missing: () => 'Not recorded' });

test('review 4 finding 15: a table reader compares the values shown, not each country with itself', () => {
  const rows: TableRowModel[] = [{ key: 'GER', cells: { v: { valueKey: 'GER\u0000s', format: 'integer' } } }];
  const shown = auditRows(rows, ['v'], values(10), 10, undefined), exact = auditRows(rows, ['v'], values(20), 10, undefined);
  assert.deepEqual(shown.map(row => [row.key, row.value]), [['GER:v', '10']]);
  assert.notEqual(shown[0]!.value, exact[0]!.value, 'shown 10 against an exact 20 is a difference the auditor sees');
  assert.deepEqual(auditRows([], ['v'], null, 10, 'No states are occupied'), [{ row: 'empty', key: 'empty:notice', value: 'No states are occupied' }]);
});

test('review 4 finding 15: a panel reader compares live values as well as structure, so two products with empty structure still differ', () => {
  const shown = panelAuditRows('no-such-product', {}, new Map([['GER\u0000total', 10]]));
  const exact = panelAuditRows('no-such-product', {}, new Map([['GER\u0000total', 20]]));
  assert.deepEqual([shown.length, shown[0]!.value === exact[0]!.value], [1, false]);
});
