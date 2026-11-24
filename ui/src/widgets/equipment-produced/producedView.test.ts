import assert from 'node:assert/strict';
import test from 'node:test';
import { producedCaptionAt, typeRows } from './producedView.ts';
import type { ProducedAtReply, ProducedModelAt } from '../../generated/dashboard';

const model = (variant: string, typeKey: string, units: number | null, ic: number | null, typeRecorded = true): ProducedModelAt => ({
  variant, rocket: false, typeKey, typeName: typeKey === 'infantry_equipment' ? 'Infantry Equipment' : null, domain: 'land', typeRecorded, name: `Model ${variant}`,
  units: { value: units, reason: units == null ? 'unitsInexact' : null }, ic: { value: ic, reason: ic == null ? 'lostOutput' : null } });

const at = (models: ProducedModelAt[], recorded = true): ProducedAtReply => ({ branchId: '1', tag: 'AAA', asked: 'AAA', hour: 50, measure: 'day', recorded, models,
  icTotal: { value: 600000, reason: null }, observation: 0, read: 48, warnings: [] });

test('a type sums its models, names a single model alone, and its IC share is of all equipment', () => {
  const rows = typeRows(at([model('1', 'infantry_equipment', 10, 300000), model('2', 'infantry_equipment', 5, 200000), model('3', 'artillery', 2, 100000)]))!;
  assert.deepEqual(rows.map(row => [row.type, row.label, row.units, row.ic, row.share]), [
    ['infantry_equipment', 'Infantry Equipment', 15, 500000, '83.3%'],
    ['artillery', 'Equipment type name unavailable', 2, 100000, '16.7%'],
  ]);
  assert.equal(rows[0].models, 'Model 1 10 · Model 2 5');
  assert.equal(rows[1].models, 'Model 3');
});

test('a type is not known while a design whose type was not recorded holds something, and not recorded is never zero', () => {
  const rows = typeRows(at([model('1', 'infantry_equipment', 10, 300000), model('9', ':variant/9', 1, 0, false)]))!;
  const infantry = rows.find(row => row.type === 'infantry_equipment')!;
  assert.equal(infantry.units, null);
  assert.equal(infantry.ic, 300000);
  assert.equal(rows.find(row => row.type === ':variant/9')!.label, 'Type not recorded');
  assert.equal(typeRows(at([], false)), null);
  assert.deepEqual(typeRows(at([])), []);
  assert.equal(typeRows(at([model('1', 'infantry_equipment', null, 300000)]))![0].units, null);
});


test('the race caption is dated at the hour it shows, and a total counts from the read it began at then', () => {
  const day = (hour: number) => `day ${hour}`;
  assert.equal(producedCaptionAt('units', 'day', null, 6600, day), 'Units delivered on day 6600');
  assert.equal(producedCaptionAt('units', 'total', [0, 5000], 6600, day), 'Units delivered from day 5000 to day 6600');
  assert.equal(producedCaptionAt('units', 'total', [0, 5000], 4000, day), 'Units delivered from day 0 to day 4000');
  assert.equal(producedCaptionAt('ic', 'month', [], 24, day), 'IC put into equipment in the month to day 24');
});
