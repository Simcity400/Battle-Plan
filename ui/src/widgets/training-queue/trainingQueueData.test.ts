import assert from 'node:assert/strict';
import test from 'node:test';
import { divisionTooltip, equipmentFill, lowestFill, orderTooltip, queueRows, queueStructure, trainingRatio, validateTrainingQueue } from './trainingQueueData.ts';
import type { TrainingDivision, TrainingOrder } from '../../generated/dashboard';

const division = (line: string, extra: Partial<TrainingDivision> = {}): TrainingDivision => ({ line, ordinal: 0, name: `Division ${line}`, nameMissing: null,
  trainingRaw: '50000', maxTrainingRaw: '100000', manpower: 100, manpowerNeeded: 900, maxManpower: 1000, contributors: [], equipment: [],
  need: [{ archetype: 'rifle', name: 'Rifles', needRaw: '100000000', heldRaw: '150000000' }, { archetype: 'gun', name: 'Guns', needRaw: '10000000', heldRaw: '0' }],
  needMissing: null, comparisonMissing: null, halted: false, ...extra });
const order = (id: string, divisions: TrainingDivision[]): TrainingOrder => ({ order: id, ordinal: 0, template: '5', templateName: 'Infantry', templateMissing: null, icon: null, iconMissing: 'Template artwork operands not recorded',
  amount: 3, priority: 1, priorityName: 'Medium', priorityMissing: null, location: 0, locationName: 'Not set', locationMissing: null,
  closed: false, exile: null, divisions });

test('orders keep the game’s order and their own divisions only', () => {
  const reply = validateTrainingQueue({ tag: 'GER', readHour: 8, missing: null, warnings: [],
    orders: [order('b', [division('2'), division('1')]), order('a', [])] });
  const rows = queueRows(reply);
  assert.deepEqual(rows.map(row => row.key), ['order:b', 'order:a']);
  assert.ok(rows[0].kind === 'order' && rows[0].children.map(row => row.key).join() === 'line:2,line:1');
  assert.equal(queueStructure({ ...reply, warnings: ['a name refused'] }), queueStructure(reply));
  assert.notEqual(queueStructure({ ...reply, readHour: 32 }), queueStructure(reply));
  assert.throws(() => validateTrainingQueue({ tag: 'GER', readHour: 8, missing: 3, warnings: [], orders: [] }), /Invalid/);
});

test('equipment fill counts no type past its need and an unknown need is no fill', () => {
  assert.ok(Math.abs(equipmentFill(division('1'))! - 100 / 110) < 1e-5, 'a surplus of rifles never covers missing guns');
  assert.equal(equipmentFill(division('1', { need: [], needMissing: 'not read', comparisonMissing: 'not read' })), null);
  assert.equal(lowestFill(order('a', [division('1'), division('2', { need: [{ archetype: 'rifle', name: null, needRaw: '100000', heldRaw: '0' }] })])), 0);
  assert.equal(trainingRatio(division('1', { maxTrainingRaw: '0' })), null);
  assert.equal(trainingRatio(division('1')), 0.5);
});


test('a held design of unresolved type leaves the fill unknown, never 0%, and keeps the known needs', () => {
  const unresolved = division('1', { need: [{ archetype: 'rifle', name: 'Rifles', needRaw: '10000000', heldRaw: '0' }],
    comparisonMissing: 'A held design’s equipment type is unresolved' });
  assert.equal(equipmentFill(unresolved), null);
  const rows = divisionTooltip(unresolved, 0).rows;
  assert.equal(rows.find(row => row.label === 'Rifles')?.value, '0 / 100');
  assert.equal(rows.find(row => row.label === 'Fill not recorded')?.value, 'A held design’s equipment type is unresolved');
});

test('an order with a division of unknown fill has no lowest fill, not the known divisions’ lowest', () => {
  const mixed = order('a', [division('1'), division('2', { comparisonMissing: 'A held design’s equipment type is unresolved' })]);
  assert.equal(lowestFill(mixed), null);
  assert.equal(orderTooltip(mixed, 8).rows.find(row => row.label === 'Lowest equipment fill')?.value, 'Not recorded');
  assert.ok(Math.abs(lowestFill(order('b', [division('1')]))! - 100 / 110) < 1e-5);
});
