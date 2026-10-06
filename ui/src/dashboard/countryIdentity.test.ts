import test from 'node:test';
import assert from 'node:assert/strict';
import { countryAliasesAt, countryIdentityKey } from './countryIdentity.ts';

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
