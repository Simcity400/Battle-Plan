import assert from 'node:assert/strict';
import test from 'node:test';
import type { RaidTallySeries } from '../../dashboard/raids';
import { tallyLines, tallyLineTooltip, tallyModel } from './raidTallyData.ts';
import { definition, OPTIONS } from './definition.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { resolveCountries } from '../../dashboard/groups.ts';

const synthetic = (): RaidTallySeries => ({ tag: 'MOD A', segments: [{ firstHour: 100, lastHour: 101 }, { firstHour: 103, lastHour: 105 }], membership: null,
  runs: [{ firstHour: 100, lastHour: 101, readHour: 100, launched: 0, succeeded: 0 }, { firstHour: 103, lastHour: 105, readHour: 103, launched: 0, succeeded: 1 }] });

test('raid tally lines preserve gaps and break across unrecorded segments', () => {
  const item = synthetic();
  assert.equal(tallyLines([item], 'succeeded', { firstHour: 100, lastHour: 105 }, null)[0].runs.length, 2);
  const equalItem = { ...synthetic(), runs: [{ firstHour: 100, lastHour: 105, readHour: 100, launched: 0, succeeded: 0 }],
    segments: [{ firstHour: 100, lastHour: 101 }, { firstHour: 102, lastHour: 105 }] };
  const runs = tallyLines([equalItem], 'launched', { firstHour: 100, lastHour: 105 }, null)[0].runs;
  assert.deepEqual(runs.map(run => [run.firstHour, run.lastHour, run.value, run.breakBefore]), [[100, 101, 0, true], [102, 105, 0, true]]);
});

test('Both metric line tooltip shows launched and succeeded counts with note', () => {
  assert.equal(resolveOptions(OPTIONS, {}).metric, 'both');
  const item = synthetic();
  item.runs[1].launched = 4;
  const overflow = { ...item, tag: 'MOD B', runs: [{ ...item.runs[1], launched: 0, succeeded: 7 }] };
  const series = tallyModel([item, overflow]);
  const tip = tallyLineTooltip(series, 104, ['MOD A', 'MOD A', 'MOD B'], () => tag => ({ name: `${tag} at hover`, flag: 'hover flag' }), () => 'country', 'both');
  assert.deepEqual(tip.rows.map(row => [row.label, row.countryFlag?.flag, row.value]),
    [['MOD A at hover', 'hover flag', '4+ launched · 1+ succeeded'], ['MOD B at hover', 'hover flag', '0+ launched · 7+ succeeded']]);
  assert.match(tip.note!, /\+ means at least this many/);
});

test('chosen raid countries have no arbitrary row cap', () => {
  const all = Array.from({ length: 5000 }, (_, index) => ({ ...synthetic(), tag: `M${index}` }));
  const series = tallyModel(all);
  const resolved = resolveCountries({ kind: 'tags', tags: [...series.keys()] }, [], undefined, 104, definition.countries.max);
  assert.equal(resolved.tags.length, 5000);
  assert.equal(resolved.dropped, 0);
});
