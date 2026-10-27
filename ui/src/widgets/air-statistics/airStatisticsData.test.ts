import test from 'node:test';
import assert from 'node:assert/strict';
import { pairRatio, pairsAt, rankedPairs, scaled, statisticName, theatreRegions, validateAirStatistics } from './airStatisticsData.ts';

const reply = (series: { name: string; runs: [number, number, string, boolean][] }[], known = [{ firstHour: 0, lastHour: 100 }]) => validateAirStatistics({
  window: { firstHour: 0, lastHour: 100, known, reads: [[0, 47, 0], [48, 100, 48]], series },
  channels: [{ id: '2', ours: 'Ships bombed', enemy: 'Ships sunk', divisor: 100000 }, { id: '3', ours: 'Fighters lost', enemy: 'Fighters destroyed', divisor: 100000 }],
  regions: [{ id: '10', name: 'North Sea' }, { id: '11', name: 'Channel' }, { id: '12', name: 'Biscay' }],
  theatres: [{ id: 'west', name: 'Western', regions: ['10', '11'] }],
  warnings: [],
});

test('pairs read the chosen channel and its enemy counterpart at the hour, keeping recorded zeros', () => {
  const data = reply([
    { name: 'GER@7|10|c3', runs: [[0, 47, '100000', true], [48, 100, '250000', false]] },
    { name: 'GER@7|10|c15', runs: [[0, 100, '0', true]] },
    { name: 'GER@7|11|c3', runs: [[48, 100, '-100000', true]] },
    { name: 'GER@7|10|c2', runs: [[0, 100, '5', true]] },
    { name: 'ITA@8|12|c3', runs: [[0, 100, '7', true]] },
  ]);
  const early = pairsAt(data, 'GER', '3', 10)!;
  assert.deepEqual(early.rows, [{ region: '10', name: 'North Sea', ours: 100000n, enemy: 0n }]);
  assert.equal(early.readHour, 0);
  const late = pairsAt(data, 'GER', '3', 60)!;
  assert.deepEqual(late.rows.map(row => [row.region, row.ours, row.enemy]), [['10', 250000n, 0n], ['11', -100000n, null]]);
  assert.equal(late.readHour, 48);
});

test('a region without a record is absent and an unrecorded hour is unknown', () => {
  const data = reply([{ name: 'GER@7|10|c3', runs: [[0, 10, '1', true]] }], [{ firstHour: 0, lastHour: 20 }]);
  assert.deepEqual(pairsAt(data, 'GER', '3', 15)!.rows, []);
  assert.equal(pairsAt(data, 'GER', '3', 50), null);
});

test('regions rank by ours plus enemy and narrow to chosen theatres', () => {
  const rows = [
    { region: '10', name: 'North Sea', ours: 1n, enemy: 1n },
    { region: '11', name: 'Channel', ours: 5n, enemy: null },
    { region: '12', name: 'Biscay', ours: -9n, enemy: 0n },
  ];
  assert.deepEqual(rankedPairs(rows, null, 2).map(row => row.region), ['12', '11']);
  const regions = theatreRegions(reply([]).theatres, ['west']);
  assert.deepEqual(rankedPairs(rows, regions, null).map(row => row.region), ['11', '10']);
  assert.equal(theatreRegions(undefined, []), null);
});

test('values keep their exact display scale and the ratio needs both sides', () => {
  assert.equal(scaled(123456789n, 100000), '1,234.56789');
  assert.equal(scaled(-5000n, 1000), '-5');
  assert.equal(scaled(1n, 1000), '0.001');
  assert.equal(pairRatio({ region: '1', name: 'A', ours: 200000n, enemy: 300000n }, 100000), '1.5');
  assert.equal(pairRatio({ region: '1', name: 'A', ours: 0n, enemy: 3n }, 100000), '—');
  assert.equal(pairRatio({ region: '1', name: 'A', ours: 1n, enemy: null }, 100000), '—');
});

test('a malformed reply is refused', () => {
  assert.throws(() => validateAirStatistics({ window: {}, channels: [], regions: [], warnings: [] }));
  assert.throws(() => validateAirStatistics({ ...reply([]), channels: [{ id: '1', ours: 'A', enemy: 'B', divisor: 0 }] }));
});

test('statistic names drop the colon the game puts before its numbers', () => {
  assert.equal(statisticName('Our fighters lost:'), 'Our fighters lost');
  assert.equal(statisticName('Fighters lost'), 'Fighters lost');
});
