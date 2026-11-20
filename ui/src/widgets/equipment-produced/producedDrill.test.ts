import assert from 'node:assert/strict';
import test from 'node:test';
import { producedDrillData, validateCountryReply } from './producedDrill.ts';
import type { CountryRef, ProducedCountryReply, ProducedModel, ProducedSeries } from '../../generated/dashboard';
import { countryKey } from '../../dashboard/countryKey.ts';

const series = (runs: [number, number, number | null][]): ProducedSeries => ({
  firstHours: runs.map(run => run[0]), lastHours: runs.map(run => run[1]), values: runs.map(run => run[2]),
  unknown: runs.flatMap((run, index) => run[2] === null ? [{ run: index, reason: 'notRecorded' as const }] : []), breaks: [] });

const model = (variant: string, typeKey: string, runs: [number, number, number | null][], typeRecorded = true): ProducedModel => ({
  variant, rocket: false, typeKey, typeName: null, domain: 'land', typeRecorded, names: [{ firstHour: 0, name: `Model ${variant}`, known: true, design: null }], series: series(runs) });

const reply = (count: 'units' | 'ic', models: ProducedModel[], total: [number, number, number | null][], asked: CountryRef = 'AAA'): ProducedCountryReply => validateCountryReply({
  branchId: '1', tag: 'AAA', asked, count, measure: 'total', decimals: 0, identities: [{ firstHour: 0, lastHour: 100, slot: 1, countryId: 1 }],
  models, total: series(total), reads: [0, 24, 48], observations: [0], warnings: [] });

test('one type stacks its models and their sum; a design whose type was not recorded makes the sum unknown while it holds something', () => {
  const data = producedDrillData(reply('units', [
    model('1', 'infantry', [[0, 49, 2], [50, 100, 6]]),
    model('2', 'infantry', [[0, 100, 1]]),
    model('3', 'artillery', [[0, 100, 4]]),
    model('9', ':variant/9', [[0, 59, 0], [60, 100, 3]], false),
  ], [[0, 100, 99]]), { count: 'units', type: 'infantry' });
  assert.deepEqual(data.parts(0).map(part => part.label), ['Model 1', 'Model 2']);
  assert.equal(data.at('AAA', 10)!.total, 3);
  assert.deepEqual(data.at('AAA', 10)!.values, [2, 1]);
  assert.equal(data.at('AAA', 70)!.total, null);
  assert.equal(data.at('BBB', 10), null);
  const bands = data.bands('AAA', { firstHour: 0, lastHour: 100 });
  assert.deepEqual(bands[0].map(run => [run.firstHour, run.lastHour, run.value]), [[0, 49, 2], [50, 59, 6]]);
  assert.deepEqual(bands[2].map(run => run.value), [0]);
});

test('a breakdown asked by slot answers that slot whatever tag it goes by, and never another country', () => {
  // The service names the country by its tag at the hour; the drill names it by the key it asked with.
  const bySlot = producedDrillData(reply('units', [model('1', 'infantry', [[0, 100, 2]])], [[0, 100, 2]], 1), { count: 'units', type: 'infantry' });
  assert.equal(bySlot.at(countryKey(1), 10)!.total, 2);
  assert.deepEqual(bySlot.bands(countryKey(1), { firstHour: 0, lastHour: 100 })[0].map(run => run.value), [2]);
  assert.equal(bySlot.segmentAt(countryKey(1), 10), 0);
  assert.equal(bySlot.at(countryKey(2), 10), null, 'a reply asked for one country is never taken for another');
  assert.equal(bySlot.at('AAA', 10), null, 'nor for a tag chosen in its own right');
  assert.equal(producedDrillData(reply('units', [model('1', 'infantry', [[0, 100, 2]])], [[0, 100, 2]]), { count: 'units', type: 'infantry' }).at(countryKey(1), 10), null);
});

test('IC with no type stacks every type against the whole, the rest of which is its own band', () => {
  const data = producedDrillData(reply('ic', [
    model('1', 'infantry', [[0, 100, 5]]),
    model('3', 'artillery', [[0, 100, null]]),
  ], [[0, 100, 12]]), { count: 'ic', type: null });
  assert.deepEqual(data.parts(0).map(part => part.id), ['infantry', 'artillery']);
  assert.deepEqual(data.at('AAA', 5)!.values, [5, null]);
  assert.equal(data.at('AAA', 5)!.total, 12);
  const bands = data.bands('AAA', { firstHour: 0, lastHour: 100 });
  assert.deepEqual(bands.map(runs => runs.map(run => run.value)), [[5], [0], [7]]);
});
