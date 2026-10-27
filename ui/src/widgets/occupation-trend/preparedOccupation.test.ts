import test from 'node:test';
import assert from 'node:assert/strict';
import { mergeOccupationReads, validateOccupationSummaries } from './preparedOccupation.ts';
import { occupationLines, occupationModel } from './occupationTrendData.ts';
import type { OccupationTrendReply } from '../../generated/dashboard';
import type { MeasureSummaries } from '../country-measure/preparedMeasureData';

const exact: OccupationTrendReply = { branchId: '1', decimals: 5, warnings: [], series: [{ tag: 'opaque occupier', segments: [{ firstHour: 100, lastHour: 200 }], membership: null,
  runs: [{ firstHour: 100, lastHour: 100, readHour: 99, startsSegment: true, resistanceRaw: '123456', complianceRaw: '987654', states: 2, heldStates: 3, heldComplete: true, collaboration: [] }] }] };
const summary: MeasureSummaries = { branchId: '1', measure: 'resistance', decimals: 5, warnings: [], series: [{ tag: 'opaque occupier', marks: null, summary: { from: 100, to: 200, width: 1, buckets: [{
  firstHour: 100, lastHour: 200, firstValue: 1.23456, lastValue: 9, lastValueHour: 200, minHour: 100, minValue: 1.23456, maxHour: 180, maxValue: 90, firstBreak: true, hasBreak: false,
}] } }] };

test('occupation dense geometry never invents exact means, state counts or collaboration reads', () => {
  validateOccupationSummaries(summary);
  const result = mergeOccupationReads(exact, exact, summary)!;
  assert.deepEqual(result.series[0].runs, exact.series[0].runs);
  assert.equal(result.series[0].runs[0].states, 2);
  assert.equal(result.series[0].runs[0].readHour, 99);
  assert.equal(mergeOccupationReads(undefined, exact, summary), undefined);
});

test('occupation historical lines survive a missing reached value and keep the independent Both geometry', () => {
  const result = mergeOccupationReads({ ...exact, series: [] }, undefined, summary)!;
  result.series[0].segments = exact.series[0].segments;
  const compliance = { ...summary.series[0].summary, runs: [{ firstHour: 100, lastHour: 200, value: 9.87654 }] };
  const summaries = new Map([[JSON.stringify(['opaque occupier', 'resistance']), summary.series[0].summary], [JSON.stringify(['opaque occupier', 'compliance']), compliance]]);
  const lines = occupationLines(occupationModel(result), ['opaque occupier'], 'both', { firstHour: 100, lastHour: 200 }, null, 2, summaries);
  assert.equal(lines.length, 2);
  assert.equal(lines[0].runs.length, 0);
  assert.equal(lines[1].runs[0].value, 9.87654);
  assert.deepEqual(result.series[0].runs, []);
  assert.deepEqual(lines[0].endLabel!(200, 'Occupier'), { text: 'Occupier Resistance', suffix: 'Not recorded' });
});

test('occupation labels use exact endpoint facts rather than extrema or an unreached state', () => {
  const endRun = { ...exact.series[0].runs[0], firstHour: 200, lastHour: 200, startsSegment: false, resistanceRaw: '900000', states: 7, readHour: 199 };
  const reply = { ...summary, series: [{ ...summary.series[0], endRun }] };
  const result = mergeOccupationReads(exact, undefined, reply, reply)!;
  assert.equal(result.series[0].runs.length, 2);
  assert.equal(result.series[0].runs[1].states, 7);
  assert.equal(result.series[0].runs[1].resistanceRaw, '900000');
  const duplicateReply = { ...summary, series: [{ ...summary.series[0], endRun: { ...exact.series[0].runs[0], startsSegment: false } }] };
  const duplicate = mergeOccupationReads(exact, undefined, duplicateReply)!;
  assert.deepEqual(duplicate.series[0].runs, exact.series[0].runs);
});

test('exact reached identity evidence replaces sampled aliases while metadata still being prepared stays visible', () => {
  const reached = { ...exact, preparing: ['identities', 'segments'], series: [{ ...exact.series[0],
    identities: [{ firstHour: 100, lastHour: 100, slot: 7, countryId: 70 }] }] };
  const sampled = { ...summary, pending: [{ firstHour: 150, lastHour: 200 }], series: [{ ...summary.series[0],
    identities: [{ firstHour: 100, lastHour: 200, slot: 7, countryId: 71 }] }] };
  const result = mergeOccupationReads(reached, undefined, sampled)!;
  assert.equal(result.series[0].identities?.[0].countryId, 70);
  assert.deepEqual(result.preparing, ['identities', 'segments']);
  assert.deepEqual(result.pending, [{ firstHour: 150, lastHour: 200 }]);
});
