import assert from 'node:assert/strict';
import { test } from 'node:test';
import { liveKey } from '../../charts/live.ts';
import { valuesOf } from '../../test-support/liveValues.ts';
import { heldCoverage, occupationMean, totalsOf, validateOccupationDetail } from './occupationTrendData.ts';
import { occupationHoverItems } from './occupationLinesProjection.ts';
import type { OccupationDetailReply } from '../../generated/dashboard';

const cells = (rows: Record<string, Record<string, number>>) => {
  const values = valuesOf(new Map(Object.entries(rows).flatMap(([tag, columns]) =>
    Object.entries(columns).map(([column, value]) => [liveKey(tag, column), value] as const))), 'Not recorded at this hour');
  return { ...values, raw: (key: string) => Number.isSafeInteger(values.get(key)) ? BigInt(values.get(key)!) : undefined };
};
const reply: OccupationDetailReply = { occupiers: [{ country: 'GER', missing: null, collaboration: [{ slot: 4, tag: 'POL', progressRaw: '25000' }, { slot: 9, tag: null, progressRaw: null }] },
  { country: 'ITA', missing: null, collaboration: [] }, { country: 'SOV', missing: 'No country holds this tag at this hour', collaboration: [] }], collaborationMissing: null, warnings: [] };
const values = cells({
  GER: { resistanceSum: 3000001, complianceSum: 900000, states: 3, heldStates: 4, heldComplete: 1 },
  ITA: { resistanceSum: 5000000, complianceSum: 0, states: 1, heldStates: 1, heldComplete: 0 },
});

test('the means truncate the fixed5 sum over the states as the game does, and none without states', () => {
  assert.equal(occupationMean(3000001n, 3n), '1000000');
  assert.equal(occupationMean(-7n, 2n), '-3');
  assert.equal(occupationMean(5n, 0n), null);
  const totals = totalsOf(values, 'GER')!;
  assert.deepEqual([totals.resistanceRaw, totals.complianceRaw, heldCoverage(totals)], ['1000000', '300000', '3/4 states']);
  assert.equal(heldCoverage(totalsOf(values, 'ITA')!), '1 states; total unknown');
  assert.equal(totalsOf(values, 'SOV'), null);
});

test('the lines hover lists every drawn occupier in the chart order, both measures when both are shown, and a missing total with its reason', () => {
  const lines = ['ITA', 'GER', 'SOV'].map(tag => ({ tag, name: `Name ${tag}`, color: '#000' }));
  const items = occupationHoverItems(reply, values, lines, { shown: 'both', rank: 'resistance', collaboration: true, digits: 0 }, tag => `Name ${tag}`);
  assert.deepEqual(items.map(item => [item.label, item.value]), [
    ['Name ITA', '50%'], ['Average compliance', '0%'], ['States', '1 states; total unknown'],
    ['Name GER', '10%'], ['Average compliance', '3%'], ['States', '3/4 states'],
    ['Country collaboration: Name POL', '25%'], ['Country collaboration: Country name unavailable', 'Not recorded'],
    ['Name SOV', 'Not recorded'],
  ]);
  assert.equal(items.at(-1)!.missingReason, 'No country holds this tag at this hour');
});

test('the detail reply is checked whole', () => {
  assert.equal(validateOccupationDetail(reply).occupiers.length, 3);
  assert.throws(() => validateOccupationDetail({ ...reply, occupiers: [{ country: 'GER', missing: null, collaboration: [{ slot: 'x', tag: null, progressRaw: null }] }] }));
});

test('occupation hover retains exact large sums for both measures and shared lines and mix details', () => {
  const sums = { resistanceSum: (1n << 60n) + 7n, complianceSum: (1n << 63n) - 2n, states: 2n, heldStates: 3n, heldComplete: 1n };
  const exact = new Map(Object.entries(sums).map(([column, value]) => [liveKey('GER', column), value]));
  const large = { get: (key: string) => exact.has(key) ? Number(exact.get(key)) : undefined,
    raw: (key: string) => exact.get(key), missing: () => 'Not recorded at this hour' };
  const totals = totalsOf(large, 'GER')!;
  assert.deepEqual([totals.resistanceRaw, totals.complianceRaw], ['576460752303423491', '4611686018427387903']);
  const items = occupationHoverItems(reply, large, [{ tag: 'GER', name: 'Name GER', color: '#000' }],
    { shown: 'both', rank: 'resistance', collaboration: false, digits: 5 }, tag => tag);
  assert.deepEqual(items.map(item => item.value), ['5,764,607,523,034.23491%', '46,116,860,184,273.87903%', '2/3 states']);
});

test('an inexact occupation operand is unavailable with the reason for that operand', () => {
  const unsafe = cells({ GER: { resistanceSum: Number((1n << 60n) + 7n), complianceSum: 4, states: 2 } });
  const items = occupationHoverItems(reply, unsafe, [{ tag: 'GER', name: 'Name GER', color: '#000' }],
    { shown: 'both', rank: 'resistance', collaboration: false, digits: 5 }, tag => tag);
  assert.equal(items[0]!.value, 'Not recorded');
  assert.match(items[0]!.missingReason!, /Resistance sum.*exact integer/);
});
