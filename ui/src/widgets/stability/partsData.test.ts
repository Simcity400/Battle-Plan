import test from 'node:test';
import assert from 'node:assert/strict';
import { divergingStack } from '../../charts/series.ts';
import type { MeasureParts, StabilityPart, StabilityPartName } from './types';
import type { ChartBucket, ChartSummary } from '../../charts/chartSummary';
import { engineSum, knownAt, partLayers, partsAt, partsStack, partsWithin, REMAINDER_LABEL, SMALLER_LABEL, stackRange, visibleSegment, partsFromLines, partsAtStates, validateStabilityPartNames, namesAt } from './partsData.ts';
import { heldParts } from '../../dashboard/keyedSeriesData.ts';

const run = (firstHour: number, lastHour: number, valueRaw: string) => ({ firstHour, lastHour, valueRaw, startsSegment: false });
const part = (id: string, runs: StabilityPart['runs'], extra: Partial<StabilityPart> = {}): StabilityPart => ({ id, term: null, line: null, runs, ...extra });
const line = { line: 0, kind: 'idea', identity: 'idea_a', identityId: 0, secondaryRaw: '0', valueRaw: '10000', text: null, label: 'Idea A', group: null, groupKind: 'national_spirit' };

const PARTS: MeasureParts = {
  observed: [{ firstHour: 0, lastHour: 9 }, { firstHour: 20, lastHour: 29 }],
  parts: [
    part('term:base', [run(0, 9, '50000'), run(20, 29, '50000')], { term: 'base' }),
    part('line:idea:idea_a:0:0', [run(0, 4, '10000')], { line }),
    part('remainder', [run(0, 9, '-5000')]),
  ],
};


test('parts are known only where recorded, and a part without a run there is zero', () => {
  assert.ok(knownAt(PARTS, 5));
  assert.ok(!knownAt(PARTS, 15));
  assert.equal(partsAt(PARTS, 15), null);
  assert.deepEqual(partsAt(PARTS, 7)!.map(item => [item.part.id, item.raw]), [['term:base', '50000'], ['remainder', '-5000']]);
});

test('the largest parts take the palette in order, the rest share one grey layer', () => {
  const layers = partLayers(PARTS, 'stability', ['#a'], '#grey');
  assert.deepEqual(layers.map(layer => [layer.id, layer.color]), [['term:base', '#a'], ['__smaller', '#grey']]);
  assert.equal(layers[1].name, SMALLER_LABEL);
  assert.deepEqual(layers[1].members, ['line:idea:idea_a:0:0', 'remainder']);
  const two = partLayers(PARTS, 'stability', ['#a', '#b'], '#grey');
  assert.deepEqual(two.map(layer => layer.color), ['#a', '#b', '#grey']);
  assert.equal(two[2].name, REMAINDER_LABEL);
  assert.equal(partLayers(PARTS, 'stability', ['#a', '#b', '#c'], '#grey').length, 3);
  const late = partsWithin(PARTS, [{ firstHour: 20, lastHour: 29 }]);
  assert.deepEqual(partLayers(late, 'stability', ['#a'], '#grey').map(layer => layer.id), ['term:base']);
});

test('signed values stack up and down from zero in layer order', () => {
  assert.deepEqual(divergingStack([[5, -2], [3, -1]]), [
    { low: [0, -2], high: [5, 0] },
    { low: [5, -3], high: [8, -2] },
  ]);
});

test('the stack breaks at unrecorded hours and samples both ends of every change', () => {
  const layers = partLayers(PARTS, 'stability', ['#a', '#b', '#c'], '#grey');
  const segments = partsStack(PARTS, layers, 5, 0, 29);
  assert.equal(segments.length, 2);
  assert.deepEqual(segments[0].hours, [0, 4, 5, 9]);
  const base = segments[0].bands[layers.findIndex(layer => layer.id === 'term:base')];
  const spirit = segments[0].bands[layers.findIndex(layer => layer.id === 'line:idea:idea_a:0:0')];
  assert.deepEqual([base.high[1], spirit.high[1], spirit.high[2]], [50, 60, 50]);
  assert.deepEqual(segments[1].hours, [20, 29]);
  assert.deepEqual(partsStack(PARTS, layers, 5, 12, 18), []);
});

const bucket = (firstHour: number, lastHour: number, value: number, firstBreak = false): ChartBucket => ({ firstHour, lastHour, firstValue: value,
  lastValue: value, lastValueHour: lastHour, minHour: firstHour, minValue: value, maxHour: firstHour, maxValue: value, firstBreak, hasBreak: false });
const summary = (buckets: (ChartBucket | null)[]): ChartSummary => ({ from: 0, to: 100, width: buckets.length, buckets });
const named = (part: string, kind: string, id: string | null, extra: Partial<StabilityPartName> = {}): StabilityPartName =>
  ({ part, measure: 'stability', kind, id, term: null, line: null, missing: null, ...extra });


test('a measure’s parts over a view are their lines’ bins, known where the displayed value’s line holds a bin', () => {
  const names = new Map([['1', named('1', 'total', 'total')], ['2', named('2', 'term', 'term:base', { term: 'base' })], ['3', named('3', 'line', 'line:idea:a:0:0', { line })],
    ['5', named('5', 'line', 'line:idea:a:0:0', { line })]]);
  const lines = new Map([
    ['1', summary([bucket(0, 5, 0.6), bucket(6, 11, 0.6), null, bucket(18, 23, 0.5, true)])],
    ['2', summary([bucket(0, 5, 0.5), bucket(6, 11, 0.5), null, bucket(18, 23, 0.5, true)])],
    ['3', summary([bucket(0, 5, 0.1), null, null, null])],
    ['5', summary([null, bucket(6, 11, 0.1), null, null])],
  ]);
  const parts = partsFromLines(['1', '2', '3', '5', '6'], lines, names);
  assert.deepEqual(parts.observed, [{ firstHour: 0, lastHour: 11 }, { firstHour: 18, lastHour: 23 }]);
  assert.deepEqual(parts.parts.map(item => [item.id, item.runs.map(run => [run.firstHour, run.lastHour, run.valueRaw])]), [
    ['term:base', [[0, 11, '50000'], [18, 23, '50000']]],
    ['line:idea:a:0:0', [[0, 11, '10000']]],
  ], 'one source under two handles is one part; the displayed value is no part');
  assert.ok(parts.parts.every(item => item.id !== 'total'));
  const unnamed = partsFromLines(['1', '9'], new Map([['1', lines.get('1')!], ['9', summary([bucket(0, 5, 0.2)])]]), new Map([['1', names.get('1')!]]));
  assert.equal(unnamed.parts[0].id, 'part:9', 'a part not named yet keeps its key, never another’s name');
  const tiled = (summary: ChartSummary, value: number): ChartSummary => ({ ...summary, tiles: { binHours: 6, valueAt: () => value } });
  const headed = partsFromLines(['1', '2'], new Map([['1', tiled(lines.get('1')!, 0.7)], ['2', tiled(lines.get('2')!, 0.4)]]), names, 8);
  assert.deepEqual(headed.observed, [{ firstHour: 0, lastHour: 8 }]);
  assert.deepEqual(headed.parts[0].runs.map(run => [run.firstHour, run.lastHour, run.valueRaw]), [[0, 5, '50000'], [6, 8, '40000']],
    'the head bin is the value at the head, never the bin’s first value nor an hour after the head');
});

test('each part is named at the last hour it was held in the view, so one only an earlier run held still has its name', () => {
  const listed = heldParts([{ part: '1', id: null, firstHour: 0, lastHour: 50 }, { part: '2', id: null, firstHour: 0, lastHour: 900 },
    { part: '3', id: null, firstHour: 600, lastHour: 2_000 }, { part: '1', id: null, firstHour: 700, lastHour: 800 }, { part: '4', id: null, firstHour: 990, lastHour: 1_200 },
    { part: '4', id: null, firstHour: 100, lastHour: 200 }]);
  assert.deepEqual(namesAt(listed, 1_000), [[800, ['1']], [900, ['2']], [1_000, ['3', '4']]]);
});

test('parts at an hour are the states read there, known only where the displayed value was recorded', () => {
  const names = new Map([['1', named('1', 'total', 'total')], ['2', named('2', 'term', 'term:base', { term: 'base' })], ['3', named('3', 'remainder', 'remainder')]]);
  const state = (part: string, hour: number, valueRaw: string | null) => ({ tag: 'A', part, hour, drop: null, missing: valueRaw === null ? 'Not recorded' : null,
    run: valueRaw === null ? null : { firstHour: hour - 2, lastHour: hour + 2, valueRaw, slot: 1, countryId: 1, startsSegment: false, segmentFirstHour: 0, firstExact: true, lastExact: true } });
  const states = [state('1', 10, '60000'), state('2', 10, '50000'), state('3', 10, '10000'), state('1', 11, null), state('2', 11, '50000')];
  const at = partsAtStates(['1', '2', '3'], states, 10, names);
  assert.deepEqual(at.observed, [{ firstHour: 10, lastHour: 10 }]);
  assert.deepEqual(at.parts.map(item => [item.id, item.runs[0].valueRaw, item.runs[0].firstHour]), [['term:base', '50000', 10], ['remainder', '10000', 10]]);
  assert.deepEqual(partsAtStates(['1', '2'], states, 11, names).observed, [], 'no displayed value, no known parts');
});

test('a part names reply is checked before it is shown', () => {
  const reply = { tag: 'A', hour: 3, warnings: [], parts: [named('1', 'total', 'total')] };
  assert.equal(validateStabilityPartNames(reply), reply);
  assert.throws(() => validateStabilityPartNames({ ...reply, parts: [{ ...reply.parts[0], measure: 'other' }] }), /malformed/);
  assert.throws(() => validateStabilityPartNames({ ...reply, parts: null }), /malformed/);
});

test('parts are known only within the hours the country is shown', () => {
  const within = partsWithin(PARTS, [{ firstHour: 5, lastHour: 22 }]);
  assert.deepEqual(within.observed, [{ firstHour: 5, lastHour: 9 }, { firstHour: 20, lastHour: 22 }]);
  assert.equal(partsAt(within, 3), null);
  assert.equal(partsWithin(PARTS, null), PARTS);
});

test('the value range counts a band crossing the whole view', () => {
  const flat: MeasureParts = { observed: [{ firstHour: 0, lastHour: 1000 }], parts: [part('term:base', [run(0, 1000, '50000')], { term: 'base' })] };
  const layers = partLayers(flat, 'stability', ['#a'], '#grey');
  const segments = partsStack(flat, layers, 5, 0, 1000);
  assert.deepEqual(stackRange(segments, 200, 800), [0, 50]);
  assert.equal(stackRange(segments, 2000, 3000), null);
});

test('only the samples that draw the view are kept, with one either side', () => {
  const layers = partLayers(PARTS, 'stability', ['#a', '#b', '#c'], '#grey');
  const [segment] = partsStack(PARTS, layers, 5, 0, 9);
  assert.deepEqual(visibleSegment(segment, 4.5, 4.8)!.hours, [4, 5]);
  assert.deepEqual(visibleSegment(segment, 1, 3)!.hours, [0, 4]);
  assert.equal(visibleSegment(segment, 20, 30), null);
});

test('grouped parts of opposite sign stay on their own side of zero', () => {
  const mixed: MeasureParts = { observed: [{ firstHour: 0, lastHour: 9 }], parts: [
    part('term:base', [run(0, 9, '50000')], { term: 'base' }), part('a', [run(0, 9, '5000')]), part('b', [run(0, 9, '-5000')])] };
  const layers = partLayers(mixed, 'stability', ['#a'], '#grey');
  assert.deepEqual([...layers[1].members].sort(), ['a', 'b']);
  const [segment] = partsStack(mixed, layers, 5, 0, 9);
  assert.deepEqual(segment.layers, [0, 1, 1]);
  assert.deepEqual(segment.bands.map(band => [band.low[0], band.high[0]]), [[0, 50], [50, 55], [-5, 0]]);
});

test('a negative part closes below zero when it ends', () => {
  const bands = divergingStack([[50, 50, 50], [-10, -10, 0]]);
  assert.deepEqual([bands[1].low, bands[1].high], [[-10, -10, 0], [0, 0, 0]]);
  assert.deepEqual(divergingStack([[0, -5]])[0], { low: [0, -5], high: [0, 0] });
});


test('a thousand staggered parts are stacked by sweeping their changes', () => {
  const observed = [{ firstHour: 0, lastHour: 99_999 }];
  const many: MeasureParts = { observed, parts: Array.from({ length: 1000 }, (_, index) =>
    part(`p${index}`, Array.from({ length: 100 }, (_, runIndex) => run(runIndex * 1000 + index, runIndex * 1000 + index, '1000')))) };
  const layers = partLayers(many, 'stability', ['#a', '#b', '#c', '#d', '#e', '#f', '#g', '#h'], '#grey');
  const [segment] = partsStack(many, layers, 5, 0, 99_999);
  const at = segment.hours.indexOf(5);
  assert.equal(Math.max(...segment.bands.map(band => band.high[at])), 1);
});


test('parts add up as the engine adds them, wrapping at 64 bits', () => {
  assert.equal(engineSum(['9223372036854775000', '1000', '9223372036854775616']), 0n);
  assert.equal(engineSum(['100', '-40']), 60n);
});

test('a view past the cutoff fits and draws nothing', () => {
  const layers = partLayers(PARTS, 'stability', ['#a', '#b', '#c'], '#grey');
  const segments = partsStack(PARTS, layers, 5, 0, 29);
  assert.equal(stackRange(segments, 25, 5), null);
  assert.equal(visibleSegment(segments[0], 8, 2), null);
});

