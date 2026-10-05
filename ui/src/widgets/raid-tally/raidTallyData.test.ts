import assert from 'node:assert/strict';
import test from 'node:test';
import { syntheticRaids } from '../../dashboard/raids.fixtures.ts';
import { tallyEntries, tallyLines, tallyLineTooltip, tallyTooltip } from './raidTallyData.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';

test('raid tally keeps recorded zero apart from missing, gaps, and uncertain launches', () => {
  const data = syntheticRaids();
  const series = new Map(data.series.map(item => [item.tag, item]));
  assert.equal(tallyEntries(series, ['MOD A', 'missing'], 101, 'launched', 10).length, 1);
  assert.equal(tallyEntries(series, ['MOD A'], 102, 'launched', 10).length, 0);
  assert.equal(tallyEntries(series, ['MOD A'], 103, 'succeeded', 10)[0].label, '1+');
  assert.equal(tallyEntries(series, ['MOD A'], 103, 'launched', 10)[0].label, '0+');
  const tip = tallyTooltip(data, 'MOD A', 104, 'Renamed country at the hovered hour');
  assert.equal(tip.title, 'Renamed country at the hovered hour');
  assert.match(tip.note!, /nonzero recorded flight distance proves launch/);
  assert.match(tip.note!, /terminal outcomes alone do not/);
  assert.equal(tallyLines(data.series, 'succeeded', { firstHour: 100, lastHour: 105 }, null)[0].runs.length, 2);
});

test('partial country reads never date a new tally to an earlier complete world read', () => {
  const data = syntheticRaids();
  data.readHours = [100];
  assert.equal(tallyTooltip(data, 'MOD A', 104, 'Country').rows.find(row => row.label === 'Read')?.value, formatDateTime(104));
  assert.equal(tallyTooltip(data, 'MOD A', 104.5, 'Country').rows.find(row => row.label === 'Read')?.value, formatDateTime(104));
  data.series.push({ ...data.series[0], tag: 'other', runs: [{ ...data.series[0].runs[1], readHour: 104, firstHour: 104 }] });
  const series = new Map(data.series.map(item => [item.tag, item]));
  const tip = tallyLineTooltip(series, 104, ['MOD A', 'other'], () => tag => ({ name: tag }), () => 'country-colour', 'succeeded');
  assert.match(tip.note!, new RegExp(`Read ${formatDateTime(104)}`));
  assert.equal(tallyTooltip(data, 'MOD A', 102, 'Country').rows.some(row => row.label === 'Read'), false);
});

test('unchanged per-country tallies date every successful read even during a partial census', () => {
  const data = syntheticRaids();
  data.readHours = [100];
  data.series[0].runs = [{ ...data.series[0].runs[0], lastHour: 105 }];
  data.series[0].segments = [{ firstHour: 100, lastHour: 105 }];
  const series = new Map(data.series.map(item => [item.tag, item]));
  for (const hour of [100, 101, 105]) {
    assert.equal(tallyTooltip(data, 'MOD A', hour, 'Country').rows.find(row => row.label === 'Read')?.value, formatDateTime(hour));
    assert.match(tallyLineTooltip(series, hour, ['MOD A'], () => () => ({ name: 'Country' }), () => 'country-colour', 'launched').note!,
      new RegExp(`Read ${formatDateTime(hour)}`));
  }
  assert.equal(tallyTooltip(data, 'MOD A', 106, 'Country').rows.some(row => row.label === 'Read'), false);
});

test('raid tally ranks every one of 5000 countries before applying a display selection', () => {
  const data = syntheticRaids();
  const all = Array.from({ length: 5000 }, (_, index) => ({ ...data.series[0], tag: `M${String(index).padStart(4, '0')}`,
    runs: [{ ...data.series[0].runs[1], launched: index }] }));
  const series = new Map(all.map(item => [item.tag, item]));
  const ranked = tallyEntries(series, all.map(item => item.tag), 104, 'launched', 10);
  assert.equal(ranked[0].tag, 'M4999');
  assert.equal(ranked.length, 10);
});

test('raid top ranks never pad zero or nonexistent rows and a chosen recorded zero remains', () => {
  const data = syntheticRaids();
  const all = [{ ...data.series[0], tag: 'zero' }, { ...data.series[0], tag: 'absent', segments: [] },
    { ...data.series[0], tag: 'one', runs: [{ ...data.series[0].runs[0], launched: 1 }] }];
  const series = new Map(all.map(item => [item.tag, item]));
  assert.deepEqual(tallyEntries(series, [...series.keys()], 101, 'launched', 10, true).map(entry => [entry.tag, entry.value]), [['one', 1]]);
  assert.deepEqual(tallyEntries(series, ['zero', 'absent', 'missing'], 101, 'launched', 10).map(entry => [entry.tag, entry.value]), [['zero', 0]]);
  assert.equal(tallyEntries(series, [...series.keys()], 102, 'launched', 10, true).length, 0);
});

test('equal raid counts stay separate when a different country takes the same tag', () => {
  const data = syntheticRaids();
  const item = data.series[0];
  item.runs = [{ ...item.runs[0], firstHour: 100, lastHour: 105, launched: 0 }];
  item.segments = [{ firstHour: 100, lastHour: 101 }, { firstHour: 102, lastHour: 105 }];
  const runs = tallyLines(data.series, 'launched', { firstHour: 100, lastHour: 105 }, null)[0].runs;
  assert.deepEqual(runs.map(run => [run.firstHour, run.lastHour, run.value, run.breakBefore]),
    [[100, 101, 0, true], [102, 105, 0, true]]);
});
