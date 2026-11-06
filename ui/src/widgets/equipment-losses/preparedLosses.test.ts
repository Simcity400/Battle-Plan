import assert from 'node:assert/strict';
import test from 'node:test';
import { preparedLossHour, preparedLossWindow, validatePreparedLosses } from './preparedLosses.ts';

const complete = { schema: 24, firstHour: 100, lastHour: 200, complete: true };
const reply = () => ({ branchId: '1', tag: 'mod country', landSource: 'destruction', landLowerBound: null, amountScale: 100000,
  coverage: complete as object, pending: [] as object[], landCoverage: [], destructionCoverage: [{ firstHour: 100, lastHour: 200, reason: null }], destructionCountedFrom: 100,
  retainedMonths: null, designs: [], warnings: [],
  causes: [{ source: { kind: 'sunkConvoys' }, domain: 'naval', name: { key: null, text: 'Convoys', template: null },
    group: null, loss: true, unknown: null, unknownSpans: [], leaves: [{ design: null, lowerBoundFrom: null,
      points: [{ hour: 100, totalRaw: '9007199254740993' }, { hour: 180, totalRaw: '-9007199254740993' }] }] }],
  preparedReads: [100, 150, 200], preparedHeadHour: 150, preparedDetailHour: 150,
  preparedFrom: 100, preparedTo: 200, preparedWidth: 10, restarts: [] as number[] });

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

test('the reply root carries the prepared prefix; its land coverage spans travel as landCoverage', () => {
  const land = [{ firstHour: 100, lastHour: 200, coverage: 'logged' }];
  const data = validatePreparedLosses({ ...reply(), landCoverage: land });
  assert.deepEqual(data.coverage, land);
  assert.deepEqual(data.prefix, { coverage: complete, pending: [] });
  const { landCoverage: _land, ...unnamed } = reply();
  assert.throws(() => validatePreparedLosses(unnamed), /malformed/, 'a reply without its land coverage is refused');
  assert.throws(() => validatePreparedLosses({ ...reply(), pending: [{ firstHour: 2, lastHour: 1 }] }), /malformed/);
});

test('a prefix reply answers through its last hour; a range starting after it is still being prepared', () => {
  const prefix = validatePreparedLosses({ ...reply(), coverage: { ...complete, lastHour: 160, complete: false },
    pending: [{ firstHour: 161, lastHour: 200 }], preparedReads: [100, 160], preparedHeadHour: 160, preparedDetailHour: 160 });
  const campaign = { first: 100, hour: 200, end: 200, campaign: true, from: null };
  assert.deepEqual(preparedLossWindow(prefix, campaign), { ...campaign, hour: 160, end: 160 });
  // An hour inside the prefix is answered as asked, with the window ending at the prefix.
  const early = validatePreparedLosses({ ...reply(), coverage: { ...complete, lastHour: 160, complete: false },
    pending: [{ firstHour: 161, lastHour: 200 }], preparedReads: [100, 150, 160], preparedHeadHour: 150, preparedDetailHour: 150 });
  assert.deepEqual(preparedLossWindow(early, { ...campaign, hour: 150, campaign: false }), { ...campaign, hour: 150, end: 160, campaign: false });
  assert.equal(preparedLossWindow(prefix, { first: 170, hour: 200, end: 200, campaign: true, from: 170 }), null);
  // A complete reply answers the requested window.
  assert.deepEqual(preparedLossWindow(validatePreparedLosses(reply()), { ...campaign, hour: 150 }), { ...campaign, hour: 150 });
});
