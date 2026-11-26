import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { knownValue, notApplicable, notRecorded, partialValue } from '../../families/value.ts';
import type { TrainingDivision, TrainingQueueReply } from '../../generated/dashboard';
import { trainingQueueProjection, validateTrainingQueue } from './trainingQueueProjection.ts';

const division = (line: string, extra: Partial<TrainingDivision> = {}): TrainingDivision => ({
  line, ordinal: 0, name: `${line}. Infanterie-Division`, nameMissing: null, trainingRaw: '92000', maxTrainingRaw: '100000',
  manpower: 11760, manpowerNeeded: 11760, manpowerMissing: null, maxManpower: 11760, contributors: [], equipment: [], need: [], needMissing: null,
  comparisonMissing: null, equipmentFillRaw: '100000', halted: false, ...extra,
});

const reply = (extra: Partial<TrainingQueueReply> = {}): TrainingQueueReply => ({
  tag: 'GER', readHour: 40, missing: null, inTraining: 34, warnings: [],
  orders: [{
    order: '1', ordinal: 0, template: '5', templateName: 'Infantry 1940 A', templateMissing: null, icon: null, iconMissing: 'Template icon not recorded',
    amount: 6, priority: 1, priorityName: 'Medium equipment priority', priorityMissing: null, location: 9, locationName: 'Brandenburg', locationMissing: null,
    closed: false, exile: null, manpower: 23520, manpowerNeeded: 23520,
    divisions: [division('197'), division('198', { equipmentFillRaw: null, comparisonMissing: 'A held design’s equipment type is unresolved' })],
  }],
  ...extra,
});

const project = (raw: unknown) => trainingQueueProjection.project(createBoundedReadView({ hour: 48, panels: new Map([['main', raw]]) }), {});

test('a template row carries the product’s manpower total and the title its divisions in training; its divisions their own figures', () => {
  const model = project(reply());
  const [order] = model.rows;
  assert.deepEqual(order!.cells.division!.value, knownValue('Infantry 1940 A'));
  assert.deepEqual([order!.cells.manpower!.value, order!.cells.manpower!.secondaryValue], [knownValue(23520), knownValue('23,520')]);
  assert.equal(order!.cells.division!.subtext, '2 divisions · Brandenburg · Medium equipment priority');
  assert.deepEqual(order!.cells.training!.value, notApplicable('Shown on each division'));
  assert.equal(model.titleSummary, '34 in training');
  assert.equal(order!.initiallyOpen, true, 'the first order opens, as the approved crop shows');
  const [first, second] = order!.children!;
  assert.deepEqual([first!.cells.training!.value, first!.cells.equipment!.value], [knownValue(92), knownValue(100)]);
  assert.deepEqual([order!.cells.division!.iconSlot, first!.cells.division!.iconSlot], [true, undefined], 'only a template row has an art slot');
  assert.deepEqual(second!.cells.equipment!.value, notRecorded('A held design’s equipment type is unresolved'), 'an unmeasurable fill is missing with its reason, never 0');
});

test('a queue not read near the hour is unavailable with the reason, and an old reply without the product totals is refused', () => {
  const missing = project(reply({ orders: [], inTraining: null, missing: 'The training queue was not read within a day of this hour' }));
  assert.deepEqual([missing.rows.length, missing.emptyKind, missing.titleSummary], [0, 'unavailable', undefined]);
  const { inTraining: _total, ...old } = reply();
  assert.throws(() => validateTrainingQueue(old));
});

test('a division’s name and training survive unread manpower, and held amounts are not printed while the held equipment cannot be compared', () => {
  const unread = 'Training manpower not read whole: Deployment manpower not available at this hour';
  const model = project(reply({ orders: [{ ...reply().orders[0]!, manpower: null, manpowerNeeded: null, divisions: [
    division('197', { manpower: null, manpowerNeeded: null, manpowerMissing: unread,
      need: [{ archetype: 'infantry_equipment', name: 'Infantry equipment', needRaw: '100000000', heldRaw: '40000000' }],
      equipmentFillRaw: null, comparisonMissing: 'Held equipment not read whole' }),
  ] }] }));
  const [order] = model.rows;
  const [first] = order!.children!;
  assert.deepEqual([first!.cells.division!.value, first!.cells.training!.value], [knownValue('197. Infanterie-Division'), knownValue(92)], 'witnessed name and progress stay');
  assert.deepEqual([first!.cells.manpower!.value, first!.cells.manpower!.secondaryValue], [notRecorded(unread), notRecorded(unread)]);
  assert.equal(order!.cells.manpower!.value.kind, 'notRecorded', 'an order total over an unknown division is not a sum');
  const tip = typeof first!.tooltipExtra === 'function' ? first!.tooltipExtra() : first!.tooltipExtra;
  assert.deepEqual(tip?.[0]?.value, partialValue('1,000 needed', 'Held equipment not read whole'), 'no exact held figure that leaves out what was not read, and the recorded need still shown');
});

test('the order tooltip distinguishes parallel divisions in training from serial waves remaining, including continuous orders', () => {
  const finite = project(reply()).rows[0]!;
  const finiteTip = typeof finite.tooltipExtra === 'function' ? finite.tooltipExtra() : finite.tooltipExtra;
  assert.deepEqual(finiteTip?.slice(0, 2).map(item => [item.label, item.value]), [
    ['Parallel divisions in training', knownValue('2')],
    ['Serial waves remaining', knownValue('6')],
  ]);
  const continuous = project(reply({ orders: [{ ...reply().orders[0]!, amount: -1 }] })).rows[0]!;
  const continuousTip = typeof continuous.tooltipExtra === 'function' ? continuous.tooltipExtra() : continuous.tooltipExtra;
  assert.deepEqual(continuousTip?.slice(0, 2).map(item => [item.label, item.value]), [
    ['Parallel divisions in training', knownValue('2')],
    ['Serial waves remaining', knownValue('Continuous')],
  ]);
});
