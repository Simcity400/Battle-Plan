import test from 'node:test';
import assert from 'node:assert/strict';
import { liveKey, valuesOf } from '../../charts/live.ts';
import { channelIndex, pairRatio, pairsOf, rankedPairs, scaled, statisticName, validateAirStatistics } from './airStatisticsData.ts';

const reply = (regions: { region: string; name: string | null; missing: string | null }[], missing: string | null = null) => validateAirStatistics({
  readHour: 48, missing, warnings: [], theatres: [{ id: '7', name: 'West', regions: ['10'] }], theatresMissing: null,
  channels: [{ id: '2', ours: 'Ships bombed', enemy: 'Ships sunk', divisor: 100000 }, { id: '3', ours: 'Fighters lost:', enemy: 'Fighters destroyed', divisor: 100000 }],
  regions,
});

/** The live cells of regions' words: `[region, ours, enemy]`, a word left out not recorded. */
const words = (pairs: [string, number | null, number | null][]) => valuesOf(new Map(pairs.flatMap(([region, ours, enemy]) => [
  ...ours === null ? [] : [[liveKey(region, 'ours'), ours] as const], ...enemy === null ? [] : [[liveKey(region, 'enemy'), enemy] as const]])),
'Not recorded at this hour');

test('pairs keep recorded zeros, name a region without a name by its id and why, and a word not recorded is missing', () => {
  const data = reply([{ region: '10', name: 'North Sea', missing: null }, { region: '11', name: null, missing: 'Region 11 name unsupported' }]);
  assert.deepEqual(pairsOf(data, words([['10', 250000, 0], ['11', -100000, null]])).map(row => [row.region, row.name, row.ours, row.enemy, row.enemyWhy]),
    [['10', 'North Sea', 250000n, 0n, 'Not recorded at this hour'], ['11', 'Region 11 (Region 11 name unsupported)', -100000n, null, 'Not recorded at this hour']]);
  assert.equal(scaled(null, 100000), '—', 'never zero');
  assert.equal(statisticName(data.channels[1].ours), 'Fighters lost');
});

test('statistics missing at the hour hold no rows, and malformed replies are refused', () => {
  assert.equal(reply([], 'Air statistics are not recorded at this hour').regions.length, 0);
  assert.throws(() => reply([{ region: '1', name: 'A', missing: null }], 'not recorded'), /malformed/);
  assert.throws(() => reply([{ region: '1', name: 'A', missing: 'both' }]), /malformed/);
  assert.equal(channelIndex('11'), 11);
  assert.equal(channelIndex('12'), null);
});

test('regions rank by magnitude, are narrowed and cut, and values scale exactly', () => {
  const data = reply([{ region: '1', name: 'B', missing: null }, { region: '2', name: 'A', missing: null }, { region: '3', name: 'C', missing: null }]);
  const rows = pairsOf(data, words([['1', 100, 0], ['2', -300, 0], ['3', 50, 50]]));
  assert.deepEqual(rankedPairs(rows, null, null).map(row => row.region), ['2', '1', '3']);
  assert.deepEqual(rankedPairs(rows, new Set(['1', '3']), 1).map(row => row.region), ['1']);
  assert.equal(scaled(1234567n, 1000), '1,234.567');
  assert.equal(scaled(-100000n, 100000), '-1');
  assert.equal(pairRatio(rows[2], 100000), '1');
  assert.equal(pairRatio(rows[1], 100000), '—');
});

test('the theatres at the hour validate with the reply, and malformed ones refuse it', () => {
  assert.deepEqual(reply([]).theatres, [{ id: '7', name: 'West', regions: ['10'] }]);
  const base = { readHour: 48, missing: null, warnings: [], channels: [], regions: [] };
  assert.equal(validateAirStatistics({ ...base, theatres: null, theatresMissing: 'Air-theatre owner is unresolved' }).theatresMissing, 'Air-theatre owner is unresolved');
  assert.throws(() => validateAirStatistics({ ...base, theatres: [{ id: 7, name: 'West', regions: [] }], theatresMissing: null }), /malformed/);
  assert.throws(() => validateAirStatistics({ ...base, theatresMissing: null }), /malformed/);
});
