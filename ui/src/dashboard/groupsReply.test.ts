import test from 'node:test';
import assert from 'node:assert/strict';
import { validateCountryGroupReply } from './groupsReply.ts';
import type { CountryGroupReply } from '../generated/dashboard';

const good: CountryGroupReply = {
  branchId: '1', warnings: [], humans: [{ tag: 'GER', firstHour: 0, lastHour: 9 }],
  factions: [{ id: '3', name: null, recordedName: '', firstHour: 0, lastHour: 9, nameReason: 'Faction template not recorded', names: [{ firstHour: 0, lastHour: 9, name: null, reason: 'Faction template not recorded', token: null }], members: [{ tag: 'GER', firstHour: 0, lastHour: 9 }], leaders: [], incomplete: [] }],
  wars: [{ first: 'GER', second: 'POL', firstHour: 5, lastHour: 9 }],
  warStarts: [{ hour: 5, attacking: 'GER', defending: 'POL', attackingMissing: null, defendingMissing: null }], capitulations: [{ hour: 8, tag: 'POL' }], identityChanges: [{ hour: 7, from: 'AAA', to: 'BBB' }],
  countryIdentities: ['GER', 'POL'].map((tag, slot) => ({ tag, identities: [{ firstHour: 0, lastHour: 9, slot, countryId: slot + 1 }] })),
  unobserved: { humans: [], factions: [{ firstHour: 2, lastHour: 3 }], wars: [], unattributed: [] },
};

test('a war start retains either known side and requires a reason for each unresolved side', () => {
  const check = (start: Record<string, unknown>) => validateCountryGroupReply({ ...good, warStarts: [{ hour: 5, ...start }] });
  for (const known of ['attacking', 'defending']) {
    const unknown = known === 'attacking' ? 'defending' : 'attacking';
    const start = { [known]: 'AAA', [`${known}Missing`]: null, [unknown]: null, [`${unknown}Missing`]: 'Country link not resolved' };
    assert.equal(check(start).warStarts.length, 1);
    assert.throws(() => check({ ...start, [`${unknown}Missing`]: null }), /malformed/);
    assert.throws(() => check({ ...start, [`${unknown}Missing`]: '' }), /malformed/);
    assert.throws(() => check({ ...start, [`${known}Missing`]: 'Contradiction' }), /malformed/);
  }
});


test('a reply missing a list, or with a malformed entry, is refused', () => {
  const { warStarts: _dropped, ...older } = good;
  assert.throws(() => validateCountryGroupReply({ ...older, declarations: [] }), /malformed/);
  assert.throws(() => validateCountryGroupReply({ ...good, capitulations: [{ hour: '8', tag: 'POL' }] }), /malformed/);
  assert.throws(() => validateCountryGroupReply({ ...good, identityChanges: [{ hour: 7, from: 'AAA' }] }), /malformed/);
  assert.throws(() => validateCountryGroupReply({ ...good, countryIdentities: undefined }), /malformed/);
  assert.throws(() => validateCountryGroupReply({ ...good, countryIdentities: [{ tag: 'GER', identities: [{ firstHour: 0, lastHour: 9, slot: '0', countryId: 1 }] }] }), /malformed/);
  assert.throws(() => validateCountryGroupReply({ ...good, wars: [{ first: 'GER', second: 'POL', firstHour: 9, lastHour: 5 }] }), /malformed/);
  assert.throws(() => validateCountryGroupReply({ ...good, unobserved: { ...good.unobserved, wars: [{ firstHour: 0, lastHour: Number.MAX_SAFE_INTEGER + 1 }] } }), /malformed/);
  assert.throws(() => validateCountryGroupReply({ ...good, unobserved: { humans: [], factions: [] } }), /malformed/);
  assert.throws(() => validateCountryGroupReply({ ...good, factions: [{ ...good.factions[0], leaders: undefined }] }), /malformed/);
  assert.throws(() => validateCountryGroupReply({ ...good, factions: [{ ...good.factions[0], leaders: [{ tag: 'GER', firstHour: 5 }] }] }), /malformed/);
});

test('dated faction names must be ordered, disjoint and say why a name is unavailable', () => {
  const faction = { ...good.factions[0], nameReason: 'Template missing', names: [
    { firstHour: 0, lastHour: 3, name: 'Opening pact', reason: null, token: null },
    { firstHour: 5, lastHour: 9, name: null, reason: 'Template missing', token: null },
  ] };
  const check = (patch: Record<string, unknown>) => validateCountryGroupReply({ ...good, factions: [{ ...faction, ...patch }] });
  assert.equal(check({}).factions[0].names.length, 2);
  assert.throws(() => check({ names: undefined }), /malformed/);
  assert.throws(() => check({ names: [...faction.names].reverse() }), /malformed/);
  assert.throws(() => check({ names: [faction.names[0], { ...faction.names[1], firstHour: 3 }] }), /malformed/);
  assert.throws(() => check({ names: [{ ...faction.names[1], reason: null }] }), /malformed/);
  assert.throws(() => check({ names: [{ ...faction.names[0], reason: 'Template missing' }] }), /malformed/);
  assert.throws(() => check({ nameReason: undefined }), /malformed/);
});
