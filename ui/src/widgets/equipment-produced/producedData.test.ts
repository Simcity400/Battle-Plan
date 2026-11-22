import { gatedHour } from '../../test-support/gated.ts';
import assert from 'node:assert/strict';
import test from 'node:test';
import { lineRuns, mergeProducedWindows, producedAt, producedModel, producedRankHour, producedRequest, producedWindows, raceEntries, rankProduced, seriesModel, seriesRuns,
  validateProducedReply, validateProducedWindow, valueAt } from './producedData.ts';
import type { ProducedData } from './producedData.ts';
import { WINDOW_HOURS } from '../../dashboard/heldWindows.ts';
import type { EquipmentProducedReply, ProducedSeries } from '../../generated/dashboard';

const series = (runs: [number, number, number | null][], unknown: ProducedSeries['unknown'] = [], breaks: number[] = []): ProducedSeries => ({
  firstHours: runs.map(run => run[0]), lastHours: runs.map(run => run[1]), values: runs.map(run => run[2]), unknown, breaks });

const reply = (lines: EquipmentProducedReply['lines']): ProducedData => ({
  branchId: '1', count: 'units', measure: 'day', decimals: 0, types: ['infantry_equipment'], rankHour: 100, lines, reads: [10, 34], observations: [0], warnings: [] });

const identity = [{ firstHour: 0, lastHour: 200, slot: 1, countryId: 1 }];

test('a value at an hour is its run, with why where it is not known and the stretch it belongs to', () => {
  const model = seriesModel(series([[0, 9, 0], [10, 33, null], [34, 57, 7], [60, 90, 3]], [{ run: 1, reason: 'windowIncomplete' }], [3]));
  assert.deepEqual(valueAt(model, 5), { value: 0, reason: null, index: 0, stretch: 0 });
  assert.deepEqual(valueAt(model, 20), { value: null, reason: 'windowIncomplete', index: 1, stretch: 0 });
  assert.equal(valueAt(model, 58), null);
  assert.deepEqual(valueAt(model, 70.5), { value: 3, reason: null, index: 3, stretch: 1 });
  assert.deepEqual(seriesRuns(model, 0), [
    { firstHour: 0, lastHour: 9, value: 0, breakBefore: true },
    { firstHour: 34, lastHour: 57, value: 7, breakBefore: true },
    { firstHour: 60, lastHour: 90, value: 3, breakBefore: true },
  ]);
});

test('the race ranks known values greatest first and leaves out unknown ones, and zero ones for a top N', () => {
  const model = producedModel(validateProducedReply(reply([
    { country: 'AAA', tag: 'AAA', identities: identity, series: series([[0, 100, 5]]) },
    { country: 'BBB', tag: 'BBB', identities: identity, series: series([[0, 100, 9]]) },
    { country: 'CCC', tag: 'CCC', identities: identity, series: series([[0, 100, null]], [{ run: 0, reason: 'unitsInexact' }]) },
    { country: 'DDD', tag: 'DDD', identities: identity, series: series([[0, 100, 0]]) },
  ])));
  assert.deepEqual(rankProduced(model, model.tags, 50, 10).map(entry => entry.tag), ['BBB', 'AAA', 'DDD']);
  assert.deepEqual(rankProduced(model, model.tags, 50, 10, true).map(entry => entry.tag), ['BBB', 'AAA']);
  assert.deepEqual(rankProduced(model, model.tags, 50, 1).map(entry => entry.tag), ['BBB']);
  assert.equal(producedAt(model, 'CCC', 50)?.reason, 'unitsInexact');
  const ic = raceEntries(rankProduced(model, ['AAA'], 50, 1), 5, 50);
  assert.equal(ic[0].value, 5 / 100000);
  assert.equal(ic[0].hour, 50);
  assert.deepEqual(lineRuns(model, 'AAA', [{ firstHour: 20, lastHour: 30 }], { firstHour: 0, lastHour: 100 }).map(run => [run.firstHour, run.lastHour]), [[20, 30]]);
});

test('a malformed reply is refused, and a request ranks a top N at a fixed hour', () => {
  assert.throws(() => validateProducedReply(reply([{ country: 'AAA', tag: 'AAA', identities: identity, series: series([[0, 10, null]]) }])), /malformed/);
  assert.throws(() => validateProducedReply(reply([{ country: 'AAA', tag: 'AAA', identities: identity, series: series([[5, 10, 1], [8, 12, 2]]) }])), /malformed/);
  assert.deepEqual(producedRequest('1', { tags: [], top: 10 }, gatedHour(99), 'units', 'day', ['infantry_equipment'], false),
    { branchId: '1', count: 'units', measure: 'day', types: ['infantry_equipment'], refits: false, countries: [], top: 10, rankHour: 99 });
  assert.deepEqual(producedRequest('1', { tags: ['AAA'] }, gatedHour(99), 'ic', 'total', [], true),
    { branchId: '1', count: 'ic', measure: 'total', types: [], refits: true, countries: ['AAA'] });
});

test('a followed top N is ranked at the hour shown when paused or sought, and kept while playing, never at the branch end', () => {
  assert.equal(producedRankHour(true, gatedHour(1942), false, gatedHour(1947)), 1942, 'a seek ranks at its hour, not the end of the recording');
  assert.equal(producedRankHour(true, gatedHour(1950), true, gatedHour(1942)), 1942, 'playing asks nothing: the ranking stands');
  assert.equal(producedRankHour(true, gatedHour(1942), true, null), 1942);
  assert.equal(producedRankHour(false, gatedHour(1900), false, null), 1900);
  assert.equal(producedRankHour(true, null, true, gatedHour(1942)), null, 'outside the recording nothing is ranked, not even the held ranking');
});

const windowReply = (window: number, firstHour: number, lastHour: number, lines: EquipmentProducedReply['lines'], reads: number[], observations: number[]): EquipmentProducedReply =>
  ({ ...reply(lines), window, firstHour, lastHour, reads, observations });

test('the windows held join in hour order into the reply of their hours together, a run across their edge one run', () => {
  const W = WINDOW_HOURS;
  const first = windowReply(0, 100, W - 1, [
    { country: 'AAA', tag: 'AAA', identities: identity, series: series([[100, 199, null], [200, W - 1, 4]], [{ run: 0, reason: 'windowIncomplete' }]) },
    { country: 'BBB', tag: 'BBB', identities: identity, series: series([[100, W - 1, null]], [{ run: 0, reason: 'typeNotRecorded' }]) },
  ], [100, 124], [0]);
  const second = windowReply(1, W, 2 * W - 1, [
    { country: 'AAA', tag: 'AAA', identities: identity, series: series([[W, W + 9, 4], [W + 10, 2 * W - 1, 4]], [], [1]) },
    { country: 'BBB', tag: 'BBB', identities: identity, series: series([[W, W + 5, null], [W + 6, 2 * W - 1, 2]], [{ run: 0, reason: 'typeNotRecorded' }]) },
  ], [W + 4], [0, W + 10]);
  const merged = mergeProducedWindows(new Map([[1, validateProducedWindow(second)], [0, validateProducedWindow(first)]]));
  assert.ok(merged);
  assert.deepEqual(merged.lines.map(line => line.tag), ['AAA', 'BBB']);
  assert.deepEqual(merged.lines[0].series, series([[100, 199, null], [200, W + 9, 4], [W + 10, 2 * W - 1, 4]], [{ run: 0, reason: 'windowIncomplete' }], [2]));
  assert.deepEqual(merged.lines[1].series, series([[100, W + 5, null], [W + 6, 2 * W - 1, 2]], [{ run: 0, reason: 'typeNotRecorded' }]));
  assert.deepEqual(merged.reads, [100, 124, W + 4]);
  assert.deepEqual(merged.observations, [0, W + 10]);
  assert.equal(validateProducedReply(merged), merged);
  const third = windowReply(3, 3 * W, 3 * W + 9, [{ country: 'AAA', tag: 'AAA', identities: identity, series: series([[3 * W, 3 * W + 9, 4]]) }], [], [0]);
  const gapped = mergeProducedWindows(new Map([[0, first], [3, third]]));
  assert.deepEqual(gapped?.lines[0].series.firstHours, [100, 200, 3 * W]);
  assert.equal(mergeProducedWindows(new Map()), null);
});

test('a window names the windows of an hour span, and one reaching outside its own hours is refused', () => {
  const W = WINDOW_HOURS;
  assert.deepEqual(producedWindows(W - 1, W), [0, 1]);
  assert.deepEqual(producedWindows(5 * W + 3, 5 * W + 3), [5]);
  assert.deepEqual(producedWindows(10, 5), []);
  const outside = windowReply(1, W, 2 * W - 1, [{ country: 'AAA', tag: 'AAA', identities: identity, series: series([[W - 1, W + 3, 1]]) }], [], []);
  assert.throws(() => validateProducedWindow(outside), /malformed/);
  assert.throws(() => validateProducedWindow({ ...outside, lines: [], firstHour: 0 }), /malformed/);
  assert.equal(validateProducedWindow({ ...outside, lines: [], firstHour: 2 * W + 5, lastHour: 2 * W - 1 }).lines.length, 0, 'a window the timeline holds none of');
});
