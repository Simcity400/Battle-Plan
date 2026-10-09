import assert from 'node:assert/strict';
import test from 'node:test';
import { syntheticRaids } from '../../dashboard/raids.fixtures.ts';
import { tallyEntries, tallyIdentities, tallyLines, tallyLineTooltip, tallyModel, tallyTooltip } from './raidTallyData.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { definition, OPTIONS } from './definition.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { countryMatchFilter, projectResolvedCountries, rankedCandidates, resolveCountries } from '../../dashboard/groups.ts';
import { countryRaceEnds, countryRaceItems } from '../../charts/race/countryRace.ts';
import { SECONDARY_DASH } from '../../charts/palette.ts';
import { stepRace } from '../../charts/race/race.ts';

test('raid tally uses selected native holder intervals rather than recursively selecting reused aliases', () => {
  const data = syntheticRaids();
  const item = data.series[0];
  item.identities = [{ firstHour: 100, lastHour: 101, slot: 1, countryId: 11 }, { firstHour: 103, lastHour: 105, slot: 2, countryId: 22 }];
  data.series.push({ ...item, tag: 'alias', identities: [
    { firstHour: 100, lastHour: 101, slot: 3, countryId: 33 }, { firstHour: 103, lastHour: 105, slot: 1, countryId: 11 },
  ] });
  const selection = { kind: 'tags' as const, tags: ['MOD A'] };
  const counted = projectResolvedCountries(resolveCountries(selection, [], undefined, 105, Number.MAX_SAFE_INTEGER),
    countryMatchFilter(selection, [], undefined, 105, tallyIdentities(data.series), 100));
  assert.deepEqual(rankedCandidates(counted, 101, data.series.map(item => item.tag)), ['MOD A']);
  assert.deepEqual(new Set(rankedCandidates(counted, 104, data.series.map(item => item.tag))), new Set(['MOD A', 'alias']));
  assert.deepEqual(tallyLines(data.series, 'launched', { firstHour: 100, lastHour: 105 }, counted.membership).find(line => line.tag === 'alias')!.runs.map(run => [run.firstHour, run.lastHour]), [[103, 105]]);
  item.identities = [];
  assert.deepEqual(tallyEntries(tallyModel(data.series), ['MOD A'], 101, 'launched'), []);
});

test('raid tally keeps recorded zero apart from missing, gaps, and uncertain launches', () => {
  const data = syntheticRaids();
  const series = tallyModel(data.series);
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
  const series = tallyModel(data.series);
  const tip = tallyLineTooltip(series, 104, ['MOD A', 'other'], hour => tag => ({ name: tag, flagReason: `Unavailable ${tag} at ${hour}`, flagPending: tag === 'other' }), () => 'country-colour', 'succeeded');
  assert.match(tip.note!, new RegExp(`Read ${formatDateTime(104)}`));
  assert.deepEqual(tip.rows.map(row => row.countryFlag), [
    { name: 'MOD A', flagReason: 'Unavailable MOD A at 104', flagPending: false },
    { name: 'other', flagReason: 'Unavailable other at 104', flagPending: true },
  ]);
  assert.equal(tallyLineTooltip(series, 102, ['MOD A'], () => () => ({ name: 'Country' }), () => 'country', 'succeeded').rows[0].countryFlag, undefined);
  assert.equal(tallyTooltip(data, 'MOD A', 102, 'Country').rows.some(row => row.label === 'Read'), false);
});

test('unchanged per-country tallies date every successful read even during a partial census', () => {
  const data = syntheticRaids();
  data.readHours = [100];
  data.series[0].runs = [{ ...data.series[0].runs[0], lastHour: 105 }];
  data.series[0].segments = [{ firstHour: 100, lastHour: 105 }];
  const series = tallyModel(data.series);
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
  const series = tallyModel(all);
  const ranked = tallyEntries(series, all.map(item => item.tag), 104, 'launched', 10);
  assert.equal(ranked[0].tag, 'M4999');
  assert.equal(ranked.length, 10);
});

test('raid top ranks never pad zero or nonexistent rows and a chosen recorded zero remains', () => {
  const data = syntheticRaids();
  const all = [{ ...data.series[0], tag: 'zero' }, { ...data.series[0], tag: 'absent', segments: [] },
    { ...data.series[0], tag: 'one', runs: [{ ...data.series[0].runs[0], launched: 1 }] }];
  const series = tallyModel(all);
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

test('Both defaults to launched ranking with exact inner successes, including success-only countries', () => {
  assert.equal(resolveOptions(OPTIONS, {}).metric, 'both');
  const data = syntheticRaids();
  data.series[0].runs[1].launched = 4;
  const overflow = { ...data.series[0], tag: 'MOD B', runs: [{ ...data.series[0].runs[1], launched: 0, succeeded: 7 }] };
  const series = tallyModel([...data.series, overflow]);
  const entries = tallyEntries(series, [...series.keys()], 104, 'both', undefined, true);
  assert.deepEqual(entries.map(item => [item.tag, item.value, item.inset, item.label]),
    [['MOD A', 4, 1, '4+ launched · 1+ succeeded'], ['MOD B', 0, 7, '0+ launched · 7+ succeeded']]);
  const items = countryRaceItems(entries, new Map(), () => 'country', 'unknown', false);
  assert.equal(items[1].inset?.value, 7);
  const tip = tallyLineTooltip(series, 104, ['MOD A', 'MOD A', 'MOD B'], () => tag => ({ name: `${tag} at hover`, flag: 'hover flag' }), () => 'country', 'both');
  assert.deepEqual(tip.rows.map(row => [row.label, row.countryFlag?.flag, row.value]),
    [['MOD A at hover', 'hover flag', '4+ launched · 1+ succeeded'], ['MOD B at hover', 'hover flag', '0+ launched · 7+ succeeded']]);
});

test('Both lines use country colour with secondary dashes and exact measure end labels', () => {
  const data = syntheticRaids();
  const lines = tallyLines(data.series, 'both', { firstHour: 100, lastHour: 105 }, null);
  assert.equal(lines.length, 2);
  assert.deepEqual(lines.map(line => [line.tag, line.measure, line.dash, line.color]),
    [['MOD A', 'launched', undefined, undefined], ['MOD A', 'succeeded', SECONDARY_DASH, undefined]]);
  assert.notEqual(lines[0].id, lines[1].id);
  const ends = countryRaceEnds(lines, 100, 105, null, () => 'Hovered country', () => 'country', value => `${value}+`);
  assert.deepEqual(ends.map(end => [end.text, end.color]), [['Hovered country · launched 0+', 'country'], ['Hovered country · succeeded 1+', 'country']]);
  assert.equal(tallyLines(data.series, 'both', { firstHour: 102, lastHour: 102 }, null).length, 0);
});

test('chosen raid countries have no arbitrary row cap', () => {
  const data = syntheticRaids();
  const all = Array.from({ length: 5000 }, (_, index) => ({ ...data.series[0], tag: `M${index}` }));
  const series = tallyModel(all);
  const resolved = resolveCountries({ kind: 'tags', tags: [...series.keys()] }, [], undefined, 104, definition.countries.max);
  assert.equal(resolved.tags.length, 5000);
  assert.equal(resolved.dropped, 0);
  assert.equal(tallyEntries(series, [...series.keys()], 104, 'both').length, 5000);
});

test('tally bar easing stops across a missing raid read even when the country identity remains', () => {
  const data = syntheticRaids();
  data.series[0].segments = [{ firstHour: 100, lastHour: 105 }];
  data.series[0].runs[0].launched = 2;
  data.series[0].runs[1].launched = 10;
  const series = tallyModel(data.series);
  const items = (hour: number) => countryRaceItems(tallyEntries(series, ['MOD A'], hour, 'both'), new Map(), () => 'country', 'missing', false);
  let state = stepRace(null, items(101), 0, 16, 100);
  state = stepRace(state, items(103), 16, 16, 100);
  assert.equal(state.rows[0].widths.get('total')!.solid, 10);
});
