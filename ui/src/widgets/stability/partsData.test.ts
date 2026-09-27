import test from 'node:test';
import assert from 'node:assert/strict';
import { divergingStack } from '../../charts/series.ts';
import type { MeasureParts, StabilityPart } from '../../generated/dashboard';
import { engineSum, knownAt, partLayers, partName, partsAt, partsStack, partsWithin, REMAINDER_LABEL, SMALLER_LABEL, stackRange, validateStabilityPartsReply, visibleSegment } from './partsData.ts';

const run = (firstHour: number, lastHour: number, valueRaw: string) => ({ firstHour, lastHour, valueRaw, startsSegment: false });
const part = (id: string, runs: StabilityPart['runs'], extra: Partial<StabilityPart> = {}): StabilityPart => ({ id, term: null, line: null, runs, ...extra });
const line = { line: 0, kind: 'idea', identity: 'idea_a', identityId: 0, secondaryRaw: '0', valueRaw: '10000', text: null, label: 'Idea A', group: null, groupKind: 'national_spirit' };

// Hours 0-9 and 20-29 known; base 50%, a spirit +10% until hour 4, a remainder -5%.
const PARTS: MeasureParts = {
  observed: [{ firstHour: 0, lastHour: 9 }, { firstHour: 20, lastHour: 29 }],
  parts: [
    part('term:base', [run(0, 9, '50000'), run(20, 29, '50000')], { term: 'base' }),
    part('line:idea:idea_a:0:0', [run(0, 4, '10000')], { line }),
    part('remainder', [run(0, 9, '-5000')]),
  ],
};

test('parts are named as the tooltip names them', () => {
  assert.deepEqual(partName(PARTS.parts[0], 'stability'), { name: 'Base value', recorded: false });
  assert.deepEqual(partName(PARTS.parts[1], 'stability'), { name: 'National spirit: Idea A', recorded: false });
  assert.equal(partName(PARTS.parts[2], 'stability').name, REMAINDER_LABEL);
  assert.equal(partName(part('clamp', []), 'warSupport').name, 'Limited to the allowed range');
});

test('parts are known only where recorded, and a part without a run there is zero', () => {
  assert.ok(knownAt(PARTS, 5));
  assert.ok(!knownAt(PARTS, 15));
  assert.equal(partsAt(PARTS, 15), null);
  assert.deepEqual(partsAt(PARTS, 7)!.map(item => [item.part.id, item.raw]), [['term:base', '50000'], ['remainder', '-5000']]);
});

test('the largest parts take the palette in order, the rest share one grey layer', () => {
  // Every colour is used first: with one colour the other two share grey.
  const layers = partLayers(PARTS, 'stability', ['#a'], '#grey');
  assert.deepEqual(layers.map(layer => [layer.id, layer.color]), [['term:base', '#a'], ['__smaller', '#grey']]);
  assert.equal(layers[1].name, SMALLER_LABEL);
  // Equal weights (10% for 5 hours, 5% for 10) keep id order.
  assert.deepEqual(layers[1].members, ['line:idea:idea_a:0:0', 'remainder']);
  // With two colours, the one part left over keeps its own name in grey.
  const two = partLayers(PARTS, 'stability', ['#a', '#b'], '#grey');
  assert.deepEqual(two.map(layer => layer.color), ['#a', '#b', '#grey']);
  assert.equal(two[2].name, REMAINDER_LABEL);
  // With room for every part, none is folded.
  assert.equal(partLayers(PARTS, 'stability', ['#a', '#b', '#c'], '#grey').length, 3);
  // Only the hours shown count: outside them a part does not take a colour.
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
  // At hour 4 the spirit is on top of the base; at hour 5 it is gone.
  const base = segments[0].bands[layers.findIndex(layer => layer.id === 'term:base')];
  const spirit = segments[0].bands[layers.findIndex(layer => layer.id === 'line:idea:idea_a:0:0')];
  assert.deepEqual([base.high[1], spirit.high[1], spirit.high[2]], [50, 60, 50]);
  assert.deepEqual(segments[1].hours, [20, 29]);
  assert.deepEqual(partsStack(PARTS, layers, 5, 12, 18), []);
});

test('a malformed parts reply is refused', () => {
  const reply = { branchId: '1', tag: 'GER', decimals: 5, stability: PARTS, warSupport: { observed: [], parts: [] }, warnings: [] };
  assert.equal(validateStabilityPartsReply(reply).tag, 'GER');
  assert.throws(() => validateStabilityPartsReply({ ...reply, stability: { ...PARTS, observed: [{ firstHour: 5, lastHour: 1 }] } }), /malformed/);
  assert.throws(() => validateStabilityPartsReply({ ...reply, warSupport: undefined }), /malformed/);
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
  // No sample falls between hours 200 and 800, yet the band covers them.
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
  // Leading zeros sit on the side the layer takes later.
  assert.deepEqual(divergingStack([[0, -5]])[0], { low: [0, -5], high: [0, 0] });
});

test('many known stretches are stacked in one sweep', () => {
  const observed = Array.from({ length: 20_000 }, (_, index) => ({ firstHour: index * 2, lastHour: index * 2 }));
  const many: MeasureParts = { observed, parts: Array.from({ length: 10 }, (_, index) => part(`p${index}`, observed.map(span => run(span.firstHour, span.lastHour, '1000')))) };
  const layers = partLayers(many, 'stability', ['#a', '#b', '#c', '#d', '#e', '#f', '#g', '#h'], '#grey');
  const started = performance.now();
  assert.equal(partsStack(many, layers, 5, 0, 40_000).length, 20_000);
  assert.ok(performance.now() - started < 3000);
});

test('a thousand staggered parts are stacked by sweeping their changes', () => {
  const observed = [{ firstHour: 0, lastHour: 99_999 }];
  const many: MeasureParts = { observed, parts: Array.from({ length: 1000 }, (_, index) =>
    part(`p${index}`, Array.from({ length: 100 }, (_, runIndex) => run(runIndex * 1000 + index, runIndex * 1000 + index, '1000')))) };
  const layers = partLayers(many, 'stability', ['#a', '#b', '#c', '#d', '#e', '#f', '#g', '#h'], '#grey');
  const started = performance.now();
  const [segment] = partsStack(many, layers, 5, 0, 99_999);
  assert.ok(performance.now() - started < 1500);
  // At hour 5 exactly one part (p5) holds 1%.
  const at = segment.hours.indexOf(5);
  assert.equal(Math.max(...segment.bands.map(band => band.high[at])), 1);
});

test('twenty thousand parts over twenty thousand stretches are stacked quickly', () => {
  const observed = Array.from({ length: 20_000 }, (_, index) => ({ firstHour: index * 2, lastHour: index * 2 }));
  const many: MeasureParts = { observed, parts: observed.map((span, index) => part(`p${index}`, [run(span.firstHour, span.lastHour, '1000')])) };
  const layers = partLayers(many, 'stability', ['#a', '#b', '#c', '#d', '#e', '#f', '#g', '#h'], '#grey');
  const started = performance.now();
  assert.equal(partsStack(many, layers, 5, 0, 40_000).length, 20_000);
  assert.ok(performance.now() - started < 2000);
});

test('parts add up as the engine adds them, wrapping at 64 bits', () => {
  // The reviewer's case: base and modifier wrap negative; the clamp brings the value to 0.
  assert.equal(engineSum(['9223372036854775000', '1000', '9223372036854775616']), 0n);
  assert.equal(engineSum(['100', '-40']), 60n);
});

test('a view past the cutoff fits and draws nothing', () => {
  const layers = partLayers(PARTS, 'stability', ['#a', '#b', '#c'], '#grey');
  const segments = partsStack(PARTS, layers, 5, 0, 29);
  assert.equal(stackRange(segments, 25, 5), null);
  assert.equal(visibleSegment(segments[0], 8, 2), null);
});

test('observed and shown hours are intersected in one sweep', () => {
  const observed = Array.from({ length: 60_000 }, (_, index) => ({ firstHour: index * 2, lastHour: index * 2 }));
  const started = performance.now();
  const within = partsWithin({ observed, parts: [] }, observed);
  assert.equal(within.observed.length, 60_000);
  assert.ok(performance.now() - started < 1000);
});
