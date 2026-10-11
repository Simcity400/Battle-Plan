import assert from 'node:assert/strict';
import test from 'node:test';
import { raidIcon, raidPreparationRead, raidReadAt, raidTypeChoices, raidTypeName, validateRaidsReply } from './raids.ts';
import { syntheticRaids } from './raids.fixtures.ts';

test('raid content names remain modded and artwork keys preserve the selected target sprite', () => {
  const data = validateRaidsReply(syntheticRaids());
  assert.equal(raidTypeName(data.types[0]), 'Sky raid from a mod');
  assert.equal(raidTypeName({ ...data.types[0], name: null }), 'Raid name unavailable');
  assert.equal(raidIcon(data, data.episodes[0].runs[0]), null);
  assert.throws(() => validateRaidsReply({ ...data, readHours: [100, 99] }), /malformed/);
  assert.throws(() => validateRaidsReply({ ...data, available: undefined }), /malformed/);
  assert.throws(() => validateRaidsReply({ ...data, series: [...data.series, ...data.series] }), /malformed/);
});

test('raid filter labels distinguish equally localised content types', () => {
  const type = syntheticRaids().types[0];
  assert.deepEqual(raidTypeChoices([{ ...type, token: 'mod/a', name: 'Air raid' }, { ...type, token: 'mod/b', name: 'Air raid' }, type]),
    [{ token: 'mod/a', label: 'Air raid (1)' }, { token: 'mod/b', label: 'Air raid (2)' },
      { token: type.token, label: 'Sky raid from a mod' }]);
});

test('raid reply refuses malformed, unsafe or overlapping preparation runs', () => {
  for (const preparation of [[[100, 2]], [[100, 99, 2, 0]], [[100, 101, 2, 1], [101, 102, 3, 1]],
    [[101, 102, 2, 0], [100, 100, 3, 0]], [[100, 101, 2, 0.5]], [[100, 101, Number.MAX_SAFE_INTEGER, 1]], [null]]) {
    const data = syntheticRaids();
    data.episodes[0].preparation = preparation as typeof data.episodes[0]['preparation'];
    assert.throws(() => validateRaidsReply(data), /malformed/);
  }
});

test('compressed preparation keeps exact read dates and values without filling missing hours', () => {
  const data = validateRaidsReply(syntheticRaids());
  const episode = data.episodes[0];
  assert.deepEqual(raidReadAt(episode, 99), null);
  assert.deepEqual(raidReadAt(episode, 100), { readHour: 100, hours: 7 });
  assert.deepEqual(raidReadAt(episode, 101.75), { readHour: 101, hours: 8 });
  assert.deepEqual(raidReadAt(episode, 102), { readHour: 101, hours: 8 });
  const at = { episode, latest: episode.runs[0], current: episode.runs[0], hour: 102 };
  assert.equal(raidPreparationRead(at), null);
  assert.deepEqual(raidPreparationRead({ ...at, latest: episode.runs[1], current: episode.runs[1], hour: 104 }), { readHour: 104, hours: 8 });
  assert.deepEqual(raidReadAt(episode, 106), { readHour: 105, hours: 8 });
});

test('raid reply requires the product phase and outcome semantics', () => {
  const data = syntheticRaids();
  for (const key of ['phasePreparing', 'phaseEnded', 'outcomeNone'] as const) {
    assert.throws(() => validateRaidsReply({ ...data, [key]: undefined }), /malformed/);
  }
});

test('raid reply validation accepts 5000 countries without silently dropping any', () => {
  const fixture = syntheticRaids();
  const series = Array.from({ length: 5000 }, (_, index) => ({ ...fixture.series[0], tag: `MOD ${index}` }));
  assert.equal(validateRaidsReply({ ...fixture, series }).series.length, 5000);
});
