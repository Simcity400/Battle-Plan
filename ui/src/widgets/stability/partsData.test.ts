import test from 'node:test';
import assert from 'node:assert/strict';
import type { MeasureParts, StabilityPart, StabilityPartName } from './types';
import { engineSum, namesAt, partLayers, partsAtStates, partsWithin, REMAINDER_LABEL, SMALLER_LABEL, validateStabilityPartNames } from './partsData.ts';
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

const named = (part: string, kind: string, id: string | null, extra: Partial<StabilityPartName> = {}): StabilityPartName =>
  ({ part, measure: 'stability', kind, id, term: null, line: null, missing: null, ...extra });

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
  assert.equal(partsWithin(PARTS, null), PARTS);
});

test('parts add up as the engine adds them, wrapping at 64 bits', () => {
  assert.equal(engineSum(['9223372036854775000', '1000', '9223372036854775616']), 0n);
  assert.equal(engineSum(['100', '-40']), 60n);
});
