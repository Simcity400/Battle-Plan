import test from 'node:test';
import assert from 'node:assert/strict';
import { countryIdentityKey, countryIdentityIndex, validCountryIdentityHistories } from './countryIdentity.ts';
import { validateCountryGroupReply } from './groupsReply.ts';

const countryAliasesAt = (histories: Parameters<typeof countryIdentityIndex>[0], chosen: readonly string[], hour: number) => {
  const index = countryIdentityIndex(histories);
  return index.aliasesAt(index.selected(chosen), hour);
};

test('recorded identity keeps manual picks across opaque tag changes without guessing gap hours', () => {
  const old = { tag: 'opaque one', identities: [{ firstHour: 10, lastHour: 10, slot: 4, countryId: 5 }] };
  const renamed = { tag: 'opaque two', identities: [{ firstHour: 12, lastHour: 13, slot: 4, countryId: 5 }] };
  const later = { tag: 'another', identities: [{ firstHour: 12, lastHour: 13, slot: 4, countryId: 99 }] };
  assert.equal(countryIdentityKey(old.identities[0]), countryIdentityKey(renamed.identities[0]));
  assert.notEqual(countryIdentityKey(old.identities[0]), countryIdentityKey(later.identities[0]));
  assert.deepEqual(countryAliasesAt([old, renamed, later], ['opaque one'], 10), ['opaque one']);
  assert.deepEqual(countryAliasesAt([old, renamed, later], ['opaque one'], 11), []);
  assert.deepEqual(countryAliasesAt([old, renamed, later], ['opaque one'], 12), ['opaque two']);
});

test('literal manual picks include every holder and all its earlier aliases, excluding unrelated alias holders', () => {
  const a = { slot: 4, countryId: 5 }, c = { slot: 4, countryId: 6 }, unrelated = { slot: 7, countryId: 8 };
  const histories = [
    { tag: 'earlier alias', identities: [{ firstHour: 0, lastHour: 2, ...a }] },
    { tag: 'X', identities: [{ firstHour: 3, lastHour: 5, ...a }, { firstHour: 8, lastHour: 12, ...c }] },
    { tag: 'Y', identities: [{ firstHour: 0, lastHour: 5, ...unrelated }, { firstHour: 6, lastHour: 6, ...a }, { firstHour: 8, lastHour: 9, ...a }] },
    { tag: 'later alias', identities: [{ firstHour: 13, lastHour: 15, ...c }] },
  ];
  const index = countryIdentityIndex(histories), chosen = ['X'];
  assert.equal(countryIdentityIndex(histories), index);
  assert.equal(index.selected(chosen), index.selected(chosen));
  assert.deepEqual(index.selected(chosen), new Set([countryIdentityKey(a), countryIdentityKey(c)]));
  assert.deepEqual(countryAliasesAt(histories, chosen, 1), ['earlier alias']);
  assert.deepEqual(countryAliasesAt(histories, chosen, 6), ['Y']);
  assert.deepEqual(countryAliasesAt(histories, chosen, 7), []);
  assert.deepEqual(new Set(countryAliasesAt(histories, chosen, 8)), new Set(['Y', 'X']));
  assert.deepEqual(countryAliasesAt(histories, chosen, 14), ['later alias']);
  assert.deepEqual(index.aliasRuns(index.selected(chosen), 1, 14), [
    { tag: 'earlier alias', firstHour: 1, lastHour: 2 }, { tag: 'X', firstHour: 3, lastHour: 5 },
    { tag: 'Y', firstHour: 6, lastHour: 6 }, { tag: 'X', firstHour: 8, lastHour: 12 },
    { tag: 'Y', firstHour: 8, lastHour: 9 }, { tag: 'later alias', firstHour: 13, lastHour: 14 },
  ]);
  assert.equal(index.keyAt('X', 7), null);
  assert.equal(index.keyAt('Y', 1), countryIdentityKey(unrelated));
});

test('native identity catalogues reject missing, duplicate and ambiguous spans without a tag fallback', () => {
  const history = { tag: 'opaque', identities: [{ firstHour: 0, lastHour: 2, slot: 4, countryId: 5 }] };
  assert.equal(validCountryIdentityHistories([history]), true);
  for (const value of [undefined, [history, history], [{ ...history, identities: undefined }],
    [{ ...history, identities: [history.identities[0], { ...history.identities[0], firstHour: 2 }] }],
    [{ ...history, identities: [{ ...history.identities[0], firstHour: -0x8000_0001 }] }]]) {
    assert.equal(validCountryIdentityHistories(value), false);
  }
  assert.throws(() => countryIdentityIndex([history, history]), /malformed/);
  const reply = { branchId: '1', humans: [], factions: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
    countryIdentities: [history], unobserved: { humans: [], factions: [], wars: [] }, warnings: [] };
  assert.equal(validateCountryGroupReply(reply), reply);
  const { countryIdentities: _missing, ...without } = reply;
  assert.throws(() => validateCountryGroupReply(without), /malformed/);
  assert.throws(() => validateCountryGroupReply({ ...reply, countryIdentities: [history, history] }), /malformed/);
});

test('5,000 native country choices retain their full alias catalogue without per-hour projections', () => {
  const histories = Array.from({ length: 5000 }, (_, countryId) => ({ tag: `opaque ${countryId}`,
    identities: [{ firstHour: 0, lastHour: 10, slot: countryId, countryId }] }));
  const chosen = histories.map(history => history.tag), index = countryIdentityIndex(histories);
  const keys = index.selected(chosen);
  assert.equal(keys.size, 5000);
  assert.equal(index.aliasesAt(keys, 5).length, 5000);
  assert.equal(index.aliasRuns(keys, 0, 10).length, 5000);
  assert.equal(index.selected(chosen), keys);
});

test('a TAG@slot holder follows its own native country through renames, and refuses when the slot held the tag twice', () => {
  const old = { tag: 'OLD', identities: [{ firstHour: 0, lastHour: 9, slot: 4, countryId: 5 }] };
  const renamed = { tag: 'NEW', identities: [{ firstHour: 10, lastHour: 20, slot: 4, countryId: 5 }] };
  const other = { tag: 'OTHER', identities: [{ firstHour: 0, lastHour: 20, slot: 7, countryId: 8 }] };
  const index = countryIdentityIndex([old, renamed, other]);
  assert.deepEqual(index.holderAliases('OLD', 4), [{ tag: 'OLD', firstHour: 0, lastHour: 9 }, { tag: 'NEW', firstHour: 10, lastHour: 20 }]);
  assert.equal(index.holderAliases('OLD', 7), null, 'no native country held OLD in slot 7');
  const reused = countryIdentityIndex([{ tag: 'OLD', identities: [{ firstHour: 0, lastHour: 9, slot: 4, countryId: 5 }, { firstHour: 15, lastHour: 20, slot: 4, countryId: 6 }] }]);
  assert.equal(reused.holderAliases('OLD', 4), null, 'two countries held OLD in slot 4: the holder is not known');
});

