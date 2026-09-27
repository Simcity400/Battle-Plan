import test from 'node:test';
import assert from 'node:assert/strict';
import { validateCountryGroupReply } from './groupsReply.ts';

const good = {
  branchId: '1', warnings: [], humans: [{ tag: 'GER', firstHour: 0, lastHour: 9 }],
  factions: [{ id: '3', name: null, firstHour: 0, lastHour: 9, members: [{ tag: 'GER', firstHour: 0, lastHour: 9 }] , incomplete: [] }],
  wars: [{ first: 'GER', second: 'POL', firstHour: 5, lastHour: 9 }],
  warStarts: [{ hour: 5, attacking: 'GER', defending: 'POL' }], capitulations: [{ hour: 8, tag: 'POL' }], identityChanges: [{ hour: 7, from: 'AAA', to: 'BBB' }],
  unobserved: { humans: [], factions: [{ firstHour: 2, lastHour: 3 }], wars: [] },
};

test('a country groups reply with every list is accepted', () => {
  assert.equal(validateCountryGroupReply(good).warStarts.length, 1);
});

test('a reply missing a list, or with a malformed entry, is refused', () => {
  const { warStarts: _dropped, ...older } = good;
  assert.throws(() => validateCountryGroupReply({ ...older, declarations: [] }), /malformed/);
  assert.throws(() => validateCountryGroupReply({ ...good, capitulations: [{ hour: '8', tag: 'POL' }] }), /malformed/);
  assert.throws(() => validateCountryGroupReply({ ...good, identityChanges: [{ hour: 7, from: 'AAA' }] }), /malformed/);
  assert.throws(() => validateCountryGroupReply({ ...good, wars: [{ first: 'GER', second: 'POL', firstHour: 9, lastHour: 5 }] }), /malformed/);
  assert.throws(() => validateCountryGroupReply({ ...good, unobserved: { humans: [], factions: [] } }), /malformed/);
});
