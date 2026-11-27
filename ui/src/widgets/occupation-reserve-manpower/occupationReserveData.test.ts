import { test } from 'node:test';
import assert from 'node:assert/strict';
import type { OccupationReserveReply, ReserveHistoryReply } from '../../generated/dashboard';
import { reserveHistoryProof, segmentParts, segmentTotal, sourceTagAt, UNADMITTED_RESERVE_REASON, validateOccupationReserve, validateReserveHistory } from './occupationReserveData.ts';

const country = (id: number, tag: string | null = `T${id}`) => ({ id, tag });
const reply: OccupationReserveReply = {
  holders: [
    { country: country(3), slot: null, admitted: false, total: 700, entries: [{ ordinal: 0, source: country(9, null), amount: 700 }] },
    { country: country(2), slot: null, admitted: true, total: 500, entries: [{ ordinal: 0, source: country(8), amount: -50 }, { ordinal: 1, source: country(9), amount: 300 }, { ordinal: 2, source: country(9), amount: 200 }] },
    { country: country(1), slot: null, admitted: true, total: 0, entries: [] },
  ],
  readHour: 100,
  unnamed: 1,
  noneMissing: null,
  missing: null,
  warnings: [],
};

test('validateOccupationReserve accepts a ranked reply with totals and amounts and rejects malformed shapes', () => {
  assert.deepEqual(validateOccupationReserve(reply), reply);
  assert.throws(() => validateOccupationReserve({ ...reply, unnamed: -1 }));
  assert.throws(() => validateOccupationReserve({ ...reply, holders: [{ country: country(1), slot: null, admitted: true, entries: [] }] }));
});

const history: ReserveHistoryReply = {
  hour: 49,
  nextChange: 60,
  segments: [
    { firstHour: 0, lastHour: 9, admitted: true, entries: [{ source: 8, amount: 100 }], unknown: null },
    { firstHour: 10, lastHour: 19, admitted: true, entries: [{ source: 8, amount: 100 }, { source: 9, amount: 400 }, { source: 9, amount: -20 }], unknown: null },
    { firstHour: 20, lastHour: 29, admitted: false, entries: null, unknown: 'The reserve was not recorded at these hours' },
    { firstHour: 40, lastHour: 49, admitted: true, entries: [], unknown: null },
  ],
  sources: [{ id: 8, tags: [{ tag: 'AAA', firstHour: 0, lastHour: 49 }] }, { id: 9, tags: [{ tag: null, firstHour: 10, lastHour: 14 }, { tag: 'BBB', firstHour: 15, lastHour: 19 }] }],
  warnings: [],
};

test('reserve history helpers validate bounded segments, prove intervals to nextChange, and resolve dated source tags', () => {
  assert.equal(validateReserveHistory(history), history);
  const sources = [9, 8];
  const parts0 = segmentParts(history.segments[0]!, sources);
  const parts1 = segmentParts(history.segments[1]!, sources);
  const parts2 = segmentParts(history.segments[2]!, sources);
  assert.deepEqual(parts0, [{ kind: 'zero', value: 0 }, { kind: 'known', value: 100 }]);
  assert.deepEqual(segmentTotal(history.segments[0]!, parts0), { kind: 'known', value: 100 });
  assert.deepEqual(parts1, [{ kind: 'known', value: 400 }, { kind: 'known', value: 100 }]);
  assert.deepEqual(segmentTotal(history.segments[1]!, parts1), { kind: 'known', value: 500 });
  assert.deepEqual(parts2, [
    { kind: 'notRecorded', reason: 'The reserve was not recorded at these hours' },
    { kind: 'notRecorded', reason: 'The reserve was not recorded at these hours' },
  ]);
  assert.deepEqual(segmentTotal(history.segments[2]!, parts2), { kind: 'notRecorded', reason: 'The reserve was not recorded at these hours' });
  const unadmitted = { firstHour: 30, lastHour: 39, admitted: false, entries: [{ source: 9, amount: 250 }], unknown: null };
  const unadmittedParts = segmentParts(unadmitted, sources);
  assert.deepEqual(unadmittedParts, [
    { kind: 'known', value: 250 },
    { kind: 'notRecorded', reason: UNADMITTED_RESERVE_REASON },
  ]);
  assert.deepEqual(segmentTotal(unadmitted, unadmittedParts), { kind: 'notRecorded', reason: UNADMITTED_RESERVE_REASON });
  const negWitnessed = { firstHour: 30, lastHour: 39, admitted: false, entries: [{ source: 9, amount: 250 }, { source: 8, amount: -20 }], unknown: null };
  const negParts = segmentParts(negWitnessed, sources);
  assert.deepEqual(negParts, [{ kind: 'known', value: 250 }, { kind: 'zero', value: 0 }]);
  assert.deepEqual(segmentTotal(negWitnessed, negParts), { kind: 'known', value: 250 });
  assert.equal(sourceTagAt(history, 8, 55), 'AAA', 'proven interval extends tag covering reply hour through nextChange - 1');
  assert.equal(sourceTagAt(history, 9, 55), null, 'tag run ending before reply hour does not extend');
  assert.equal(sourceTagAt(history, 8, 60), null, 'proven interval stops before nextChange');
  assert.equal(sourceTagAt(history, 9, 12), null);
  assert.equal(sourceTagAt(history, 9, 16), 'BBB');
  assert.deepEqual(reserveHistoryProof(history, [0, 49], 55), [49, 59]);
  assert.equal(reserveHistoryProof(history, [0, 49], 60), null);
  assert.throws(() => validateReserveHistory({ ...history, segments: [history.segments[1]!, history.segments[0]!] }));
  assert.throws(() => validateReserveHistory({ ...history, hour: 15 }));
  assert.throws(() => validateReserveHistory({ ...history, nextChange: 49 }));
});
