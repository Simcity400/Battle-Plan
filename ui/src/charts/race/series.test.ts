import assert from 'node:assert/strict';
import { test } from 'node:test';
import { raceSeriesIndex, seriesAt, seriesRaceEntries, seriesTooltip } from './series.ts';


test('race adapters rank exact values with UTF-8 ties and keep gaps and identity breaks', () => {
  const model = raceSeriesIndex([{ tag: 'Renamed realm', runs: [{ firstHour: 100, lastHour: 101, value: 9 },
    { firstHour: 103, lastHour: 103, value: 0 }, { firstHour: 104, lastHour: 104, value: 0, breakBefore: true }] },
  { tag: 'a,"tag', runs: [{ firstHour: 100, lastHour: 100, value: 9 }] }]);
  const items = (hour: number) => seriesRaceEntries(model, hour, 10, String);
  assert.deepEqual(items(100).map(item => item.tag), ['Renamed realm', 'a,"tag']);
  assert.deepEqual(items(102), []); assert.equal(items(103)[0].value, 0);
  assert.equal(items(103)[0].stretch, 103); assert.equal(items(104)[0].stretch, 104);
  const tip = seriesTooltip(model, 100, tag => ({ name: `${tag} at 100`, flag: 'recorded flag at 100' }), () => 'colour', String, 'date', 'measure');
  assert.equal(tip.rows[0].label, 'Renamed realm at 100'); assert.equal(tip.rows[0].countryFlag?.flag, 'recorded flag at 100');
  assert.equal(seriesTooltip(model, 102, tag => ({ name: tag }), () => 'colour', String, 'date', 'measure').rows[0].label, 'Not recorded');
  const native = { name: 'Recorded country', flagReason: 'Native flag read refused', flagPending: false };
  assert.deepEqual(seriesTooltip(model, 100, () => native, () => 'colour', String, 'date', 'measure').rows[0].countryFlag, native);
  const aggregate = seriesTooltip(model, 100, () => ({ name: 'Group total' }), () => 'colour', String, 'date', 'measure', false);
  assert.ok(aggregate.rows.every(row => row.countryFlag === undefined));
  assert.deepEqual(aggregate.rows.map(row => [row.label, row.value]), [['Group total', '9'], ['Group total', '9']]);
});

test('5,000 countries taking turns in top 10 use indexed hourly lookups', () => {
  const lines = Array.from({ length: 5000 }, (_, index) => ({ tag: `realm ${index}`, runs: [
    { firstHour: Math.floor(index / 10), lastHour: Math.floor(index / 10), value: index },
  ] }));
  const model = raceSeriesIndex(lines);
  assert.equal(seriesAt(model, 250).length, 10);
  model.index.comparisons = 0;
  const format = (value: number) => value.toLocaleString('en-US', { maximumFractionDigits: 0 });
  for (let iteration = 0; iteration < 1000; iteration++) {
    const hour = iteration % 500;
    assert.deepEqual(seriesRaceEntries(model, hour, 10, format), Array.from({ length: 10 }, (_, rank) => {
      const value = hour * 10 + 9 - rank;
      return { tag: `realm ${value}`, value, label: format(value), stretch: hour };
    }));
  }
  assert.ok(model.index.comparisons < 25000, 'lookups walk the hour index, never scan all runs');
});

test('a full race returns all 5,000 countries with exact ranked values', () => {
  const model = raceSeriesIndex(Array.from({ length: 5_000 }, (_, index) => ({ tag: String(index),
    runs: [{ firstHour: 0, lastHour: 1, value: index }] })));
  assert.deepEqual(seriesRaceEntries(model, 0, Number.MAX_SAFE_INTEGER, String), Array.from({ length: 5_000 }, (_, rank) => {
    const value = 4_999 - rank;
    return { tag: String(value), value, label: String(value), stretch: 0 };
  }));
});
