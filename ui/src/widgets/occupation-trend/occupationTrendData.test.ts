import assert from 'node:assert/strict';
import { test } from 'node:test';
import { liveKey } from '../../charts/live.ts';
import { valuesOf } from '../../test-support/liveValues.ts';
import { heldCoverage, occupationMean, occupationTooltip, totalsOf, validateOccupationDetail } from './occupationTrendData.ts';
import { occupationHoverItems } from './occupationLinesProjection.ts';
import type { DetailAt } from './occupationTrendData';
import type { OccupationDetailReply } from '../../generated/dashboard';

const labelAt = () => (tag: string) => ({ name: `Name ${tag}` });
const cells = (rows: Record<string, Record<string, number>>) => valuesOf(new Map(Object.entries(rows).flatMap(([tag, columns]) =>
  Object.entries(columns).map(([column, value]) => [liveKey(tag, column), value] as const))), 'Not recorded at this hour');
const reply: OccupationDetailReply = { occupiers: [{ country: 'GER', missing: null, collaboration: [{ slot: 4, tag: 'POL', progressRaw: '25000' }, { slot: 9, tag: null, progressRaw: null }] },
  { country: 'ITA', missing: null, collaboration: [] }, { country: 'SOV', missing: 'No country holds this tag at this hour', collaboration: [] }], collaborationMissing: null, warnings: [] };
const detail = (hour: number): DetailAt => ({ hour, reply, values: cells({
  GER: { resistanceSum: 3000001, complianceSum: 900000, states: 3, heldStates: 4, heldComplete: 1 },
  ITA: { resistanceSum: 5000000, complianceSum: 0, states: 1, heldStates: 1, heldComplete: 0 } }) });

test('the means truncate the fixed5 sum over the states as the game does, and none without states', () => {
  assert.equal(occupationMean(3000001, 3), '1000000');
  assert.equal(occupationMean(-7, 2), '-3');
  assert.equal(occupationMean(5, 0), null);
  const totals = totalsOf(detail(10).values, 'GER')!;
  assert.deepEqual([totals.resistanceRaw, totals.complianceRaw, heldCoverage(totals)], ['1000000', '300000', '3/4 states']);
  assert.equal(heldCoverage(totalsOf(detail(10).values, 'ITA')!), '1 states; total unknown');
  assert.equal(totalsOf(detail(10).values, 'SOV'), null);
});

test('a tooltip is of the hour read: it waits for that hour, says why a total is missing, and lists collaboration when asked', () => {
  assert.equal(occupationTooltip('GER', 11, detail(10), labelAt, 'both', true).rows[0].label, 'Reading this hour…');
  const tip = occupationTooltip('GER', 10, detail(10), labelAt, 'both', true);
  assert.deepEqual(tip.rows.map(row => [row.label, row.value]), [['Average resistance', '10%'], ['Average compliance', '3%'], ['States', '3/4 states'],
    ['Country collaboration: Name POL', '25%'], ['Country collaboration: Country name unavailable', 'Not recorded']]);
  assert.deepEqual(occupationTooltip('SOV', 10, detail(10), labelAt, 'resistance', false).rows.map(row => row.value), ['No country holds this tag at this hour']);
});

test('the lines hover lists every drawn occupier in the chart order, both measures when both are shown, and a missing total with its reason', () => {
  const lines = ['ITA', 'GER', 'SOV'].map(tag => ({ tag, name: `Name ${tag}`, color: '#000' }));
  const items = occupationHoverItems(reply, detail(10).values, lines, { shown: 'both', rank: 'resistance', collaboration: true, digits: 0 }, tag => `Name ${tag}`);
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
