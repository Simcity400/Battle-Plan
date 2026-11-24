import assert from 'node:assert/strict';
import test from 'node:test';
import type { RaidTallySeries } from '../../dashboard/raids';
import { tallyEntries, tallyLines, tallyLineTooltip, tallyModel, tallyTooltip } from './raidTallyData.ts';
import { definition, OPTIONS } from './definition.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { resolveCountries } from '../../dashboard/groups.ts';
import { countryRaceItems } from '../../charts/race/countryRace.ts';
import { stepRace } from '../../charts/race/race.ts';

const synthetic = (): RaidTallySeries => ({ tag: 'MOD A', segments: [{ firstHour: 100, lastHour: 101 }, { firstHour: 103, lastHour: 105 }], membership: null,
  runs: [{ firstHour: 100, lastHour: 101, readHour: 100, launched: 0, succeeded: 0 }, { firstHour: 103, lastHour: 105, readHour: 103, launched: 0, succeeded: 1 }] });

test('raid tally keeps recorded zero apart from missing, gaps, and uncertain launches', () => {
  const item = synthetic();
  const series = tallyModel([item]);
  assert.equal(tallyEntries(series, ['MOD A', 'missing'], 101, 'launched', 10).length, 1);
  assert.equal(tallyEntries(series, ['MOD A'], 102, 'launched', 10).length, 0);
  assert.equal(tallyEntries(series, ['MOD A'], 103, 'succeeded', 10)[0].label, '1');
  assert.equal(tallyEntries(series, ['MOD A'], 103, 'launched', 10)[0].label, '0');
  const tip = tallyTooltip(series, 'MOD A', 104, 'Renamed country at the hovered hour');
  assert.equal(tip.title, 'Renamed country at the hovered hour');
  assert.doesNotMatch(tip.note!, /\+ means|at least/i);
  assert.match(tip.note!, /Success includes critical success and excludes limited success/);
  assert.deepEqual(tallyTooltip(series, 'MOD A', 102, 'Country').rows, [{ label: 'Not recorded at this time', tone: 'muted' }]);
  assert.equal(tallyLines([item], 'succeeded', { firstHour: 100, lastHour: 105 }, null)[0].runs.length, 2);
});

test('raid tally ranks every one of 5000 countries before applying a display selection', () => {
  const base = synthetic();
  const all = Array.from({ length: 5000 }, (_, index) => ({ ...base, tag: `M${String(index).padStart(4, '0')}`, runs: [{ ...base.runs[1], launched: index }] }));
  const ranked = tallyEntries(tallyModel(all), all.map(item => item.tag), 104, 'launched', 10);
  assert.equal(ranked[0].tag, 'M4999');
  assert.equal(ranked.length, 10);
});

test('raid top ranks never pad zero or nonexistent rows and a chosen recorded zero remains', () => {
  const base = synthetic();
  const all = [{ ...base, tag: 'zero' }, { ...base, tag: 'absent', segments: [] }, { ...base, tag: 'one', runs: [{ ...base.runs[0], launched: 1 }] }];
  const series = tallyModel(all);
  assert.deepEqual(tallyEntries(series, [...series.keys()], 101, 'launched', 10, true).map(entry => [entry.tag, entry.value]), [['one', 1]]);
  assert.deepEqual(tallyEntries(series, ['zero', 'absent', 'missing'], 101, 'launched', 10).map(entry => [entry.tag, entry.value]), [['zero', 0]]);
  assert.equal(tallyEntries(series, [...series.keys()], 102, 'launched', 10, true).length, 0);
});

test('equal raid counts stay separate across a gap in the recording', () => {
  const item = { ...synthetic(), runs: [{ firstHour: 100, lastHour: 105, readHour: 100, launched: 0, succeeded: 0 }],
    segments: [{ firstHour: 100, lastHour: 101 }, { firstHour: 102, lastHour: 105 }] };
  const runs = tallyLines([item], 'launched', { firstHour: 100, lastHour: 105 }, null)[0].runs;
  assert.deepEqual(runs.map(run => [run.firstHour, run.lastHour, run.value, run.breakBefore]), [[100, 101, 0, true], [102, 105, 0, true]]);
});

test('Both defaults to launched ranking with exact inner successes, including success-only countries', () => {
  assert.equal(resolveOptions(OPTIONS, {}).metric, 'both');
  const item = synthetic();
  item.runs[1].launched = 4;
  const overflow = { ...item, tag: 'MOD B', runs: [{ ...item.runs[1], launched: 0, succeeded: 7 }] };
  const series = tallyModel([item, overflow]);
  const entries = tallyEntries(series, [...series.keys()], 104, 'both', undefined, true);
  assert.deepEqual(entries.map(entry => [entry.tag, entry.value, entry.inset, entry.label]),
    [['MOD A', 4, 1, '4 launched · 1 succeeded'], ['MOD B', 0, 7, '0 launched · 7 succeeded']]);
  assert.equal(countryRaceItems(entries, new Map(), () => 'country', 'unknown', false)[1].inset?.value, 7);
  const tip = tallyLineTooltip(series, 104, ['MOD A', 'MOD A', 'MOD B'], () => tag => ({ name: `${tag} at hover`, flag: 'hover flag' }), () => 'country', 'both');
  assert.deepEqual(tip.rows.map(row => [row.label, row.countryFlag?.flag, row.value]),
    [['MOD A at hover', 'hover flag', '4 launched · 1 succeeded'], ['MOD B at hover', 'hover flag', '0 launched · 7 succeeded']]);
});


test('chosen raid countries have no arbitrary row cap', () => {
  const all = Array.from({ length: 5000 }, (_, index) => ({ ...synthetic(), tag: `M${index}` }));
  const series = tallyModel(all);
  const resolved = resolveCountries({ kind: 'tags', tags: [...series.keys()] }, [], undefined, 104, definition.countries.max);
  assert.equal(resolved.tags.length, 5000);
  assert.equal(resolved.dropped, 0);
  assert.equal(tallyEntries(series, [...series.keys()], 104, 'both').length, 5000);
});

test('tally bar easing stops across a missing raid read even when the country remains', () => {
  const item = { ...synthetic(), segments: [{ firstHour: 100, lastHour: 105 }] };
  item.runs = [{ ...item.runs[0], launched: 2 }, { ...item.runs[1], launched: 10 }];
  const series = tallyModel([item]);
  const items = (hour: number) => countryRaceItems(tallyEntries(series, ['MOD A'], hour, 'both'), new Map(), () => 'country', 'missing', false);
  let state = stepRace(null, items(101), 0, 16, 100);
  state = stepRace(state, items(103), 16, 16, 100);
  assert.equal(state.rows[0].widths.get('total')!.solid, 10);
});
