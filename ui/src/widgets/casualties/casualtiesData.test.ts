import assert from 'node:assert/strict';
import test from 'node:test';
import type { SeriesStateReply } from '../../generated/dashboard';
import { casualtyRaceEntries, casualtyRatio, casualtyRows, formatCasualties, rankCasualties, stateHeld } from './casualtiesData.ts';
import type { HeldMeasures } from './casualtiesData';

const state = (measure: string, values: Record<string, string | null>): SeriesStateReply => ({ branchId: '1', measure, decimals: 0,
  states: Object.entries(values).map(([tag, raw]) => ({ country: tag, hour: 10, drop: null, missing: raw === null ? 'Not recorded' : null,
    run: raw === null ? null : { valueRaw: raw } as never })) });

const held = (): HeldMeasures => ({
  dealt: stateHeld(state('dealt', { AAA: '50', BBB: '9', CCC: null })),
  taken: stateHeld(state('taken', { AAA: '20', BBB: '40', CCC: '7' })),
  unknown: stateHeld(state('unknown', { AAA: '5', BBB: null, CCC: '1' })),
  partial: stateHeld(state('partial', { AAA: '0', BBB: '1', CCC: '0' })),
});

test('unknown losses add to taken only when known, and a lost lifetime marks a subtotal without a ratio', () => {
  const rows = casualtyRows(['AAA', 'BBB', 'CCC', 'DDD'], held(), 10, true);
  assert.deepEqual(rows.map(row => [row.tag, row.taken, row.atLeast]), [['AAA', 25n, false], ['BBB', null, true], ['CCC', 8n, false]],
    'a country with no casualty row is left out; an unknown total leaves taken unknown, never the known part alone');
  assert.equal(casualtyRows(['BBB'], held(), 10, false)[0].taken, 40n);
  assert.equal(casualtyRatio(rows[1]), '—');
  assert.equal(formatCasualties(rows[1].taken, rows[1].atLeast), '—');
  assert.equal(formatCasualties(25n, true), '† 25');
});

test('the top countries rank by dealt plus known taken, rows not known last', () => {
  const rows = casualtyRows(['CCC', 'BBB', 'AAA'], held(), 10, true);
  assert.deepEqual(rankCasualties(rows).map(row => row.tag), ['AAA', 'BBB', 'CCC']);
  assert.deepEqual(rankCasualties(rows, 1).map(row => row.tag), ['AAA']);
});

test('the race is of the ranked rows, ordered by known taken and drawn at taken with unknown attackers when asked', () => {
  const rows = rankCasualties(casualtyRows(['CCC', 'BBB', 'AAA'], held(), 10, true), 2);
  assert.deepEqual(rows.map(row => row.tag), ['AAA', 'BBB'], 'membership is by dealt plus known taken');
  assert.deepEqual(casualtyRaceEntries(rows).map(entry => [entry.tag, entry.value, entry.label]), [['AAA', 25, '25']],
    'a row whose taken with unknown attackers is not known is not raced');
  const known = rankCasualties(casualtyRows(['CCC', 'BBB', 'AAA'], held(), 10, false), 3);
  assert.deepEqual(casualtyRaceEntries(known).map(entry => [entry.tag, entry.value, entry.label]), [['BBB', 40, '† 40'], ['AAA', 20, '20'], ['CCC', 7, '7']],
    'ordered by known taken, not by the membership rank');
});

test('a top N ranks no country known to total zero; every country keeps it, and an unknown total is never a zero', () => {
  const values = { AAA: '0', BBB: '3', CCC: null, DDD: '0' };
  const measures: HeldMeasures = { dealt: stateHeld(state('dealt', values)), taken: stateHeld(state('taken', { AAA: '0', BBB: '0', CCC: '0', DDD: '0' })),
    unknown: stateHeld(state('unknown', { AAA: '0', BBB: '0', CCC: '0', DDD: '0' })), partial: null };
  const rows = casualtyRows(['AAA', 'BBB', 'CCC', 'DDD'], measures, 10, false);
  assert.deepEqual(rankCasualties(rows, 10, true).map(row => row.tag), ['BBB', 'CCC'], 'no 0/0 rows pad the top in tag order');
  assert.deepEqual(rankCasualties(rows, 10).map(row => row.tag), ['BBB', 'AAA', 'DDD', 'CCC']);
});
