import assert from 'node:assert/strict';
import test from 'node:test';
import { fill, share, typeFill, typeKey, typeName, typeTooltip, validateDivisionEquipment } from './divisionEquipmentData.ts';
import type { DivisionEquipmentReply, EquipmentType } from '../../generated/dashboard';

const type = (extra: Partial<EquipmentType> = {}): EquipmentType => ({ archetype: 'rifle', name: 'Rifles', nameMissing: null, heldRaw: '150000000',
  neededRaw: '200000000', filledRaw: '140000000', missingRaw: '60000000', divisionsShort: 2, unknownDivisions: 0, stockpileRaw: '900000000', stockpileHour: 40, variants: [], ...extra });
const reply = (types: EquipmentType[]): DivisionEquipmentReply => validateDivisionEquipment({ tag: 'GER', countFor: 'owner',
  missing: null, poolHour: 48, divisions: 3, divisionsShort: 2, unknownDivisions: 0, types, templates: [], divisionRows: null, pool: null, warnings: [] });

test('fill counts holdings only up to the need and an unknown need is no fill', () => {
  assert.equal(fill('150000000', '200000000'), 0.75);
  assert.equal(fill('300000000', '200000000'), 1, 'a surplus is never more than full');
  assert.equal(fill('-5', '200000000'), 0);
  assert.equal(fill('1', null), null);
  assert.equal(fill('1', '0'), null);
  assert.equal(share(null, '10'), null);
  assert.equal(share('5', '10'), 0.5);
});

test('missing is the summed shortfall as sent, never held minus needed, and stock stays in the tooltip', () => {
  const tip = typeTooltip(type(), reply([type()]));
  assert.equal(tip.rows.find(row => row.label === 'Missing')?.value, '600', 'a surplus elsewhere does not shrink it to 500');
  assert.match(tip.rows.find(row => row.label === 'In stockpile')?.value ?? '', /^9,000 \(as of /);
  assert.equal(typeTooltip(type({ neededRaw: null, missingRaw: null }), reply([])).rows.find(row => row.label === 'Missing')?.value, 'Not recorded');
});

test('unclassified holdings keep their own named bucket', () => {
  const unclassified = type({ archetype: null, name: null });
  assert.notEqual(typeKey(unclassified), typeKey(type({ archetype: 'unclassified' })), 'a mod archetype named unclassified is its own type');
  assert.equal(typeName(unclassified), 'Unclassified equipment');
  assert.equal(validateDivisionEquipment(reply([unclassified])).types.length, 1);
});

test('a type fills by each division’s capped fulfilment, never by aggregate holdings', () => {
  // Needs of 100 and 100 against holdings of 150 and 50: 200 held, 150 filled.
  const row = type({ heldRaw: '20000000', neededRaw: '20000000', filledRaw: '15000000', missingRaw: '5000000' });
  assert.equal(typeFill(row), 0.75, 'not 100%');
  assert.equal(typeFill(type({ filledRaw: null, neededRaw: null })), null);
  assert.equal(typeTooltip(row, reply([row])).rows.find(tip => tip.label === 'Filled')?.value, '150');
});

test('counts the recording does not hold read as not recorded, never as zero', () => {
  const unknown = type({ heldRaw: null, divisionsShort: null, unknownDivisions: null });
  const tip = typeTooltip(unknown, reply([unknown]));
  assert.equal(tip.rows.find(row => row.label === 'In divisions')?.value, 'Not recorded');
  assert.equal(tip.rows.find(row => row.label === 'Divisions short')?.value, 'Not recorded');
  assert.equal(tip.rows.find(row => row.label === 'Divisions not recorded')?.value, 'Not recorded');
  assert.throws(() => validateDivisionEquipment({ tag: 'GER', types: [{ archetype: null, heldRaw: 5, variants: [] }], templates: [], warnings: [], missing: null }));
});
