import assert from 'node:assert/strict';
import test from 'node:test';
import { raidIcon, raidTypeChoices, raidTypeName, validateRaidsReply } from './raids.ts';
import { syntheticRaids } from './raids.fixtures.ts';

test('raid content names remain modded and artwork keys preserve the selected target sprite', () => {
  const data = validateRaidsReply(syntheticRaids());
  assert.equal(raidTypeName(data.types[0]), 'Sky raid from a mod');
  assert.equal(raidTypeName({ ...data.types[0], name: null }), 'Unknown name (mod/sky_raid)');
  assert.equal(raidIcon(data, data.episodes[0].runs[0]), null);
  assert.throws(() => validateRaidsReply({ ...data, readHours: [100, 99] }), /malformed/);
  assert.throws(() => validateRaidsReply({ ...data, available: undefined }), /malformed/);
  assert.throws(() => validateRaidsReply({ ...data, series: [...data.series, ...data.series] }), /malformed/);
});

test('raid filter labels distinguish equally localised content types', () => {
  const type = syntheticRaids().types[0];
  assert.deepEqual(raidTypeChoices([{ ...type, token: 'mod/a', name: 'Air raid' }, { ...type, token: 'mod/b', name: 'Air raid' }, type]),
    [{ token: 'mod/a', label: 'Air raid (mod/a)' }, { token: 'mod/b', label: 'Air raid (mod/b)' },
      { token: type.token, label: 'Sky raid from a mod' }]);
});

test('raid reply refuses malformed or nonchronological preparation samples', () => {
  for (const preparation of [[[100, 2], [100, 3]], [[101, 2], [100, 3]], [[100, 1.5]], [[100]], [null]]) {
    const data = syntheticRaids();
    data.episodes[0].preparation = preparation as typeof data.episodes[0]['preparation'];
    assert.throws(() => validateRaidsReply(data), /malformed/);
  }
});

test('raid reply validation accepts 5000 countries without silently dropping any', () => {
  const fixture = syntheticRaids();
  const series = Array.from({ length: 5000 }, (_, index) => ({ ...fixture.series[0], tag: `MOD ${index}` }));
  assert.equal(validateRaidsReply({ ...fixture, series }).series.length, 5000);
});
