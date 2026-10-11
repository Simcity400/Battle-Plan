import assert from 'node:assert/strict';
import test from 'node:test';
import { syntheticRaidRun, syntheticRaids } from '../../dashboard/raids.fixtures.ts';
import { raidDescription, raidLogRows, raidLogTypes, raidPreparation, raidStatus, raidTooltip } from './raidLogData.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { resolveCountries } from '../../dashboard/groups.ts';
import { definition } from './definition.ts';

test('raid log country choices retain every one of 5000 recorded tags', () => {
  const tags = Array.from({ length: 5000 }, (_, index) => `MOD ${index}`);
  const resolved = resolveCountries({ kind: 'tags', tags }, [], undefined, 100, definition.countries.max);
  assert.deepEqual(resolved.tags, tags);
  assert.equal(resolved.dropped, 0);
});

test('raid log dates each observation and never backfills a later outcome or preparation across a gap', () => {
  const data = syntheticRaids();
  assert.equal(raidLogRows(data, 99, [], () => true, null).length, 0);
  const earlier = raidLogRows(data, 101, [], () => true, null)[0];
  assert.equal(earlier.hour, 100);
  assert.equal(earlier.by, true);
  assert.match(raidStatus(data, earlier.item), /preparation/);
  assert.doesNotMatch(raidStatus(data, earlier.item), /success/);
  const gap = raidLogRows(data, 102, [], () => true, null)[0];
  assert.match(raidStatus(data, gap.item), /Last observed:.*not recorded at this time/);
  assert.equal(raidPreparation(data, gap.item), 'Preparation not known');
  const later = raidLogRows(data, 105, ['mod/sky_raid'], () => true, null)[0];
  assert.match(raidDescription(data, later.item, 'New country name', 'Mod victim'), /Mod victim.*Sky raid from a mod.*Modded success/);
  assert.equal(later.hour, 103);
  assert.equal(raidLogRows(data, 105, [], () => false, null).length, 0);
});

test('hourly preparation and launch witnesses update values without rekeying or redating the log', () => {
  const data = syntheticRaids();
  data.episodes[0].runs = [syntheticRaidRun(100), syntheticRaidRun(101, 105, { launched: true, iconMissing: 'Changed icon refusal' })];
  data.episodes[0].preparation = [[100, 101, 1139, 1], [103, 105, 1142, 1]];
  const first = raidLogRows(data, 100, [], () => true, null)[0];
  const prepared = raidLogRows(data, 101, [], () => true, null)[0];
  assert.equal(prepared.key, first.key);
  assert.equal(prepared.hour, 100);
  assert.equal(raidPreparation(data, prepared.item), '1,140 h prepared');
  assert.doesNotMatch(raidStatus(data, prepared.item), /No result|None/);
  assert.equal(raidPreparation(data, raidLogRows(data, 102, [], () => true, null)[0].item), 'Preparation not known');
  const tooltip = raidTooltip(data, prepared.item, 'Raider', 'Victim');
  assert.equal(tooltip.rows.find(row => row.label === 'Preparation read')?.value, formatDateTime(101));
  data.readHours = [100];
  assert.equal(raidTooltip(data, prepared.item, 'Raider', 'Victim').rows.find(row => row.label === 'Last read')?.value, formatDateTime(101));
  assert.match(raidDescription(data, prepared.item, 'Raider', 'Victim'), /1,140 h prepared/);
  assert.doesNotMatch(raidDescription(data, prepared.item, 'Raider', 'Victim'), /has a/);
  data.episodes[0].runs.push(syntheticRaidRun(106, 106, { province: 9 }));
  assert.equal(raidLogRows(data, 106, [], () => true, null)[0].hour, 106);
});

test('raid chips contain only branch-observed types, disambiguated, regardless of current filters', () => {
  const data = syntheticRaids();
  const type = data.types[0];
  data.types = [{ ...type, name: 'Air raid' }, { ...type, token: 'mod/other', name: 'Air raid' },
    { ...type, token: 'mod/unused', name: 'Unused type' }];
  data.episodes[0].runs[1].type = 'mod/other';
  assert.deepEqual(raidLogTypes(data).map(choice => [choice.token, choice.label]),
    [['mod/sky_raid', 'Air raid (1)'], ['mod/other', 'Air raid (2)']]);
  assert.equal(raidLogRows(data, 105, ['mod/unused'], () => true, null).length, 0);
  assert.equal(raidLogRows(data, 105, [], () => true, null).length, 1);
});

test('an unchanged post-gap observation keeps the episode row date and key without filling the gap', () => {
  const data = syntheticRaids();
  data.episodes[0].runs = [syntheticRaidRun(100, 101), syntheticRaidRun(103, 105)];
  const first = raidLogRows(data, 101, [], () => true, null)[0];
  const gap = raidLogRows(data, 102, [], () => true, null)[0];
  const resumed = raidLogRows(data, 103, [], () => true, null)[0];
  assert.equal(resumed.hour, first.hour);
  assert.equal(resumed.key, first.key);
  assert.equal(gap.item.current, null);
  assert.equal(raidPreparation(data, gap.item), 'Preparation not known');
  assert.equal(raidPreparation(data, resumed.item), '8 h prepared');
  assert.equal(raidTooltip(data, resumed.item, 'Raider', 'Victim').rows.find(row => row.label === 'Last read')?.value, formatDateTime(103));
});

test('ended raid text uses the outcome and omits preparation', () => {
  const data = syntheticRaids();
  const row = raidLogRows(data, 104, [], () => true, null)[0];
  assert.match(raidStatus(data, row.item), /Modded success/);
  assert.equal(raidPreparation(data, row.item), null);
  assert.equal(raidTooltip(data, row.item, 'Raider', 'Victim').rows.some(row => row.label === 'Hours prepared'), false);
});

test('raid text takes preparation, ended and no-outcome meanings from the product', () => {
  const data = syntheticRaids({ phasePreparing: 17, phaseEnded: 29, outcomeNone: 41 });
  data.episodes[0].runs[0].phase = data.phasePreparing;
  data.episodes[0].runs[0].outcome = data.outcomeNone;
  data.episodes[0].runs[1].phase = data.phaseEnded;
  const preparing = raidLogRows(data, 101, [], () => true, null)[0].item;
  assert.equal(raidPreparation(data, preparing), '8 h prepared');
  assert.doesNotMatch(raidStatus(data, preparing), /No result/);
  assert.match(raidDescription(data, preparing, 'Raider', 'Victim'), /8 h prepared/);
  assert.equal(raidTooltip(data, preparing, 'Raider', 'Victim').rows.find(row => row.label === 'Hours prepared')?.value, '8');
  const ended = raidLogRows(data, 104, [], () => true, null)[0].item;
  assert.match(raidStatus(data, ended), /Modded success/);
  assert.equal(raidPreparation(data, ended), null);
  data.episodes[0].runs[1].outcome = data.outcomeNone;
  assert.equal(raidStatus(data, ended), 'Modded ended');
});

test('raid country membership uses the read that supplied the row facts', () => {
  const data = syntheticRaids();
  const hours: number[] = [];
  const rows = raidLogRows(data, 105, [], (_tag, hour) => { hours.push(hour); return true; }, null);
  assert.equal(rows[0].hour, 103);
  assert.deepEqual(hours, [103]);
});

test('raid category and range filters retain the campaign semantics', () => {
  const data = syntheticRaids();
  assert.equal(raidLogRows(data, 105, ['other-type'], () => true, null).length, 0);
  assert.equal(raidLogRows(data, 105, [], () => true, 104).length, 0);
  assert.equal(raidLogRows(data, 105, [], () => true, 103).length, 1);
});
