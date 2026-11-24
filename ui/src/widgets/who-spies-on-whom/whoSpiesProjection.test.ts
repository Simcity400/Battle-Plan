import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { deriveMatrixAudit, deriveMatrixTooltip } from '../../families/heat/matrixModel.ts';
import type { OperativeMatrixReply } from '../../generated/dashboard';
import { projectOperativeMatrix, projectOperativePairs } from './whoSpiesProjection.ts';

const HOUR = 60_810_000;
const agent = (id: number) => ({ id: String(id), name: `Agent ${id}`, nameMissing: null });

const reply: OperativeMatrixReply = {
  countries: ['ENG', 'GER', 'ITA', 'SOV'].map(tag => ({ tag, factionId: null, factionName: null, factionNameMissing: null })),
  rows: [
    { country: 0, capacity: { kind: 'known', active: 7, maximum: 7 }, missing: null },
    { country: 1, capacity: { kind: 'known', active: 8, maximum: 8 }, missing: null },
    { country: 2, capacity: { kind: 'missing', reason: 'Agencies were not read' }, missing: 'An operative\'s target country was not recorded at this hour' },
    { country: 3, capacity: { kind: 'noAgency' }, missing: null },
  ],
  columns: [0, 1, 2, 3],
  cells: [
    { row: 0, column: 0, home: true, operatives: [agent(1), agent(2)] },
    { row: 0, column: 1, home: false, operatives: [agent(3), agent(4), agent(5)] },
    { row: 1, column: 0, home: false, operatives: [agent(6)] },
  ],
  most: 3,
  largestMaximum: 8,
  pairs: [{ spy: 0, target: 1, operatives: [agent(3), agent(4), agent(5)] }, { spy: 1, target: 0, operatives: [agent(6)] }],
  readHour: HOUR - 12,
  missing: null,
  warnings: [],
};

const view = (data: unknown) => createBoundedReadView({ hour: HOUR, panels: new Map([['main', data]]) });

test('spy matrix: counter-intelligence on the diagonal, unknown pairs hatched never zero, agency capacity in the margin', () => {
  const model = projectOperativeMatrix(view(reply));
  assert.equal(model.shape, 'wide');
  assert.deepEqual(model.cell(0, 0), { kind: 'diagonal', outlined: true, text: '2' });
  assert.deepEqual(model.cell(1, 1), { kind: 'diagonal', outlined: true });
  const value = model.cell(0, 1);
  assert.equal(value.kind, 'value');
  assert.equal(value.kind === 'value' && value.text, '3');
  assert.equal(value.kind === 'value' && value.intensity, 1);
  assert.equal(model.cell(1, 2).kind, 'empty');
  assert.deepEqual(model.cell(2, 0), { kind: 'missing', reason: 'An operative\'s target country was not recorded at this hour' });
  assert.deepEqual(model.margin!.cell(0), { kind: 'value', fraction: 7 / 8, text: '7 / 7' });
  assert.deepEqual(model.margin!.cell(2), { kind: 'missing', reason: 'Agencies were not read' });
  assert.deepEqual(model.margin!.cell(3), { kind: 'none', label: 'No agency' });
  assert.ok(model.legend?.entries?.some(entry => entry.hatched), 'a not-recorded key is shown when a cell is hatched');
  const tip = deriveMatrixTooltip(model, 0, 1);
  assert.equal(tip?.title, 'ENG has 3 operatives in GER');
  assert.equal(tip?.hour, HOUR - 12, 'dated by the census read hour');
  assert.deepEqual(deriveMatrixTooltip(model, 0, model.columns.length)?.items, [{ label: 'Active / maximum', value: '7 / 7' }]);
  const audit = deriveMatrixAudit(model);
  assert.ok(audit.some(row => row.key === 'ENG->ENG' && row.value === '2'));
  assert.ok(audit.some(row => row.key === 'ITA->ENG' && String(row.value).startsWith('Not recorded')));
});

test('spy matrix: a census not recorded is the not-recorded state, not an empty matrix', () => {
  const model = projectOperativeMatrix(view({ ...reply, rows: [], columns: [], cells: [], pairs: [], missing: 'Operative memberships were not read' }));
  assert.equal(model.emptyKind, 'unavailable');
  assert.equal(model.emptyDetail, 'Operative memberships were not read');
  const none = projectOperativeMatrix(view({ ...reply, rows: [], columns: [], cells: [], pairs: [] }));
  assert.equal(none.empty, 'No operatives in the field at this date.');
  assert.equal(none.emptyKind, undefined);
});

test('top pairs keep the product order and scale', () => {
  const model = projectOperativePairs(view(reply));
  assert.deepEqual(model.rows.map(row => row.key), ['ENG>GER', 'GER>ENG']);
  assert.equal(model.scaleMax, 3);
});
