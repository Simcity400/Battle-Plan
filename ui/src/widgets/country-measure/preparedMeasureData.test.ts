import assert from 'node:assert/strict';
import test from 'node:test';
import { reachedMeasureReply, validateMeasureDrops, validateMeasureStates, validateMeasureSummaries } from './preparedMeasureData.ts';

test('reached balances preserve exact wide amounts, gaps and native segments', () => {
  const run = { firstHour: 7, lastHour: 9, segmentFirstHour: 2, valueRaw: '-9223372036854775808', startsSegment: false, slot: 4, countryId: 8 };
  const states = validateMeasureStates({ branchId: '3', measure: 'politicalPower', decimals: 5, warnings: [], states: [
    { tag: 'opaque', hour: 7, run, drop: null }, { tag: 'opaque', hour: 9, run, drop: null },
    { tag: 'opaque', hour: 10, run: null, drop: null },
  ] });
  const reply = reachedMeasureReply('3', 'politicalPower', ['opaque', 'missing'], states);
  assert.deepEqual(reply.series[0].runs, [run]);
  assert.deepEqual(reply.series[1].runs, []);
  assert.throws(() => validateMeasureStates({ ...states, states: [{ ...states.states[0], run: { ...run, segmentFirstHour: 8 } }] }));
});

test('event pages keep huge ordinals and duplicate dates; malformed geometry is refused', () => {
  const drop = { hour: 7, beforeRaw: '9223372036854775807', afterRaw: '-9223372036854775808' };
  const page = { firstOrdinal: '9007199254740993', total: '9007199254740995', rows: [0, 1].map(index => ({ tag: 'opaque', hour: 7, ordinal: (9007199254740993n + BigInt(index)).toString(), row: drop })), warnings: [] };
  assert.deepEqual(validateMeasureDrops(page), page);
  assert.throws(() => validateMeasureDrops({ ...page, firstOrdinal: 9007199254740993 }));
  assert.throws(() => validateMeasureDrops({ ...page, rows: [{ ...page.rows[0], hour: 8 }] }));
  assert.throws(() => validateMeasureSummaries({ branchId: '3', measure: 'politicalPower', decimals: 5, warnings: [],
    series: [{ tag: 'opaque', summary: { from: 0, to: 100, width: 1, buckets: Array(5).fill(null) }, marks: null }] }));
});


test('stability and maximum command power retain supplying reads even when the formula is unknown', () => {
  for (const measure of ['stability', 'warSupport', 'maxCommandPower']) {
    const reply = { branchId: '3', measure, decimals: 5, warnings: [], states: [{ tag: 'opaque', hour: 9, run: null, drop: null, recorded: true, readHour: 7 }] };
    assert.deepEqual(validateMeasureStates(reply), reply);
    assert.throws(() => validateMeasureStates({ ...reply, states: [{ ...reply.states[0], readHour: 7.5 }] }));
    assert.throws(() => validateMeasureStates({ ...reply, states: [{ ...reply.states[0], recorded: 'yes' }] }));
  }
});

test('hourly summary supplying runs retain unclipped bounds and refuse overlapping or malformed paths', () => {
  const summary = { from: 1.25, to: 3.5, width: 100, buckets: [null, null, null],
    runs: [{ firstHour: 0, lastHour: 8, value: 20, breakBefore: true }] };
  const reply = (runs: unknown) => ({ branchId: '3', measure: 'politicalPower', decimals: 5, warnings: [],
    series: [{ tag: 'opaque', summary: { ...summary, runs }, marks: null }] });
  assert.deepEqual(validateMeasureSummaries(reply(summary.runs)), reply(summary.runs));
  for (const runs of [[summary.runs[0], summary.runs[0]], [{ ...summary.runs[0], value: Infinity }],
    [{ ...summary.runs[0], breakBefore: undefined }]]) assert.throws(() => validateMeasureSummaries(reply(runs)));
});


test('sampled state never establishes an exact reached balance and summary provenance is validated', () => {
  const state = { tag: 'new tail country', hour: 10, approximate: true, sampleHour: 7,
    run: { firstHour: 7, lastHour: 7, segmentFirstHour: 7, valueRaw: '100000', slot: 9, countryId: 4, startsSegment: true }, drop: null };
  const values = validateMeasureStates({ branchId: '1', measure: 'politicalPower', decimals: 5, warnings: [], states: [state] });
  assert.deepEqual(reachedMeasureReply('1', 'politicalPower', [state.tag], values).series[0].runs, []);
  const summary = { from: 0, to: 20, width: 10, buckets: [], approximate: [{ firstHour: 7, lastHour: 20 }] };
  const reply = { branchId: '1', measure: 'politicalPower', decimals: 5, warnings: [], series: [{ tag: state.tag, summary, marks: null }] };
  assert.deepEqual(validateMeasureSummaries(reply), reply);
  assert.throws(() => validateMeasureSummaries({ ...reply, coverage: { firstHour: 10, lastHour: 9, complete: false } }));

  assert.throws(() => validateMeasureSummaries({ ...reply, series: [{ ...reply.series[0], summary: { ...summary,
    approximate: [{ firstHour: 7, lastHour: 6 }] } }] }));
  assert.throws(() => validateMeasureStates({ ...values, states: [{ ...state, sampleHour: 11 }] }));
});
