import assert from 'node:assert/strict';
import test from 'node:test';
import { preparedLossHour, validatePreparedLosses } from './preparedLosses.ts';

const reply = () => ({ branchId: '1', tag: 'mod country', landSource: 'destruction', landLowerBound: null, amountScale: 100000,
  coverage: [], destructionCoverage: [{ firstHour: 100, lastHour: 200, reason: null }], destructionCountedFrom: 100,
  retainedMonths: null, designs: [], warnings: [],
  causes: [{ source: { kind: 'sunkConvoys' }, domain: 'naval', name: { key: null, text: 'Convoys', template: null },
    group: null, loss: true, unknown: null, unknownSpans: [], leaves: [{ design: null, lowerBoundFrom: null,
      points: [{ hour: 100, totalRaw: '9007199254740993' }, { hour: 180, totalRaw: '-9007199254740993' }] }] }],
  preparedReads: [100, 150, 200], preparedHeadHour: 150, preparedDetailHour: 150,
  preparedFrom: 100, preparedTo: 200, preparedWidth: 10 });

test('prepared loss geometry never establishes an unfetched exact hover or pick', () => {
  const data = validatePreparedLosses(reply());
  assert.equal(preparedLossHour(data, 150), true);
  assert.equal(preparedLossHour(data, 180), false);
  assert.equal(data.causes[0].leaves[0].points[1].totalRaw, '-9007199254740993');
  const reached = reply(); reached.preparedReads = [100, 150, 180, 200];
  assert.equal(preparedLossHour(validatePreparedLosses(reached), 180), true);
});

test('prepared loss reads reject invented native dates and inconsistent view metadata', () => {
  for (const patch of [{ preparedReads: [100, 99] }, { preparedReads: [100, 100] }, { preparedReads: [2 ** 31] },
    { preparedHeadHour: NaN }, { preparedDetailHour: 150.5 }, { preparedHeadHour: 180 }, { preparedDetailHour: 180 },
    { preparedFrom: 201 }, { preparedWidth: 0 }]) {
    assert.throws(() => validatePreparedLosses({ ...reply(), ...patch }), /malformed/);
  }
});
