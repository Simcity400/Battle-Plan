import assert from 'node:assert/strict';
import test from 'node:test';
import { countryIdentityIndex, overlayCountryIdentities } from './countryIdentity.ts';

test('exact reached holders clip rough spans and preserve aliases without bridging unknown hours', () => {
  const rough = [{ tag: 'mod/alias', identities: [{ firstHour: 0, lastHour: 100, slot: 7, countryId: 10 }] },
    { tag: 'other alias', identities: [{ firstHour: 75, lastHour: 100, slot: 7, countryId: 10 }] }];
  const exact = [{ tag: 'mod/alias', identities: [{ firstHour: 50, lastHour: 50, slot: 7, countryId: 20 },
    { firstHour: 80, lastHour: 85, slot: 7, countryId: 10 }] },
    { tag: 'new holder alias', identities: [{ firstHour: 50, lastHour: 50, slot: 7, countryId: 20 }] }];
  const combined = overlayCountryIdentities(rough, exact);
  assert.deepEqual(combined[0].identities, [
    { firstHour: 0, lastHour: 49, slot: 7, countryId: 10 },
    { firstHour: 50, lastHour: 50, slot: 7, countryId: 20 },
    { firstHour: 51, lastHour: 100, slot: 7, countryId: 10 },
  ]);
  const index = countryIdentityIndex(combined);
  assert.equal(index.keyAt('mod/alias', 50), index.keyAt('new holder alias', 50));
  assert.equal(index.keyAt('mod/alias', 90), index.keyAt('other alias', 90));
  assert.equal(index.keyAt('new holder alias', 51), null);
  assert.equal(rough[0].identities[0].lastHour, 100, 'overlay never mutates held geometry');
  assert.equal(exact[0].identities[0].countryId, 20);
});

test('an exact span can cover many rough changes and empty exact evidence preserves unknown gaps', () => {
  const rough = [{ tag: 'A', identities: [{ firstHour: 0, lastHour: 1, slot: 1, countryId: 1 },
    { firstHour: 3, lastHour: 4, slot: 1, countryId: 2 }] }];
  assert.deepEqual(overlayCountryIdentities(rough, [{ tag: 'A', identities: [] }]), rough);
  assert.equal(countryIdentityIndex(overlayCountryIdentities(rough, [])).keyAt('A', 2), null);
  assert.deepEqual(overlayCountryIdentities(rough, [{ tag: 'A', identities: [{ firstHour: -2, lastHour: 6, slot: 9, countryId: 9 }] }]),
    [{ tag: 'A', identities: [{ firstHour: -2, lastHour: 6, slot: 9, countryId: 9 }] }]);
});

test('an exact alias clips every rough holder on its slot across tags while other slots and gaps remain', () => {
  const rough = [
    { tag: 'A', identities: [{ firstHour: 0, lastHour: 9, slot: 7, countryId: 11 },
      { firstHour: 12, lastHour: 14, slot: 7, countryId: 11 }] },
    { tag: 'stale holder', identities: [{ firstHour: 0, lastHour: 9, slot: 7, countryId: 12 }] },
    { tag: 'other slot', identities: [{ firstHour: 0, lastHour: 9, slot: 8, countryId: 11 }] },
  ];
  const exact = [{ tag: 'B', identities: [{ firstHour: 5, lastHour: 5, slot: 7, countryId: 11 }] }];
  const combined = overlayCountryIdentities(rough, exact);
  assert.deepEqual(combined.find(history => history.tag === 'A')!.identities, [
    { firstHour: 0, lastHour: 4, slot: 7, countryId: 11 },
    { firstHour: 6, lastHour: 9, slot: 7, countryId: 11 },
    { firstHour: 12, lastHour: 14, slot: 7, countryId: 11 },
  ]);
  assert.deepEqual(combined.find(history => history.tag === 'stale holder')!.identities, [
    { firstHour: 0, lastHour: 4, slot: 7, countryId: 12 },
    { firstHour: 6, lastHour: 9, slot: 7, countryId: 12 },
  ]);
  assert.deepEqual(combined.find(history => history.tag === 'other slot'), rough[2]);
  const index = countryIdentityIndex(combined);
  assert.deepEqual(index.aliasesAt(index.selected(['A']), 5), ['B']);
  assert.equal(index.keyAt('stale holder', 5), null);
  assert.equal(index.keyAt('A', 10), null);
  assert.equal(index.keyAt('A', 11), null);
  assert.equal(index.keyAt('A', 4), index.keyAt('A', 6));
  assert.equal(rough[0].identities[0].lastHour, 9, 'held geometry is never mutated');
  assert.deepEqual(exact[0].identities, [{ firstHour: 5, lastHour: 5, slot: 7, countryId: 11 }]);
});

test('exact aliases join adjacent equal holders after clipping all rough aliases', () => {
  const rough = [{ tag: 'A', identities: [{ firstHour: 0, lastHour: 9, slot: 7, countryId: 11 }] },
    { tag: 'B', identities: [{ firstHour: 4, lastHour: 4, slot: 7, countryId: 11 },
      { firstHour: 7, lastHour: 8, slot: 7, countryId: 11 }] }];
  const exact = [{ tag: 'B', identities: [{ firstHour: 5, lastHour: 5, slot: 7, countryId: 11 },
    { firstHour: 6, lastHour: 6, slot: 7, countryId: 11 }] }];
  const combined = overlayCountryIdentities(rough, exact), index = countryIdentityIndex(combined);
  assert.deepEqual(combined[0].identities, [{ firstHour: 0, lastHour: 4, slot: 7, countryId: 11 },
    { firstHour: 7, lastHour: 9, slot: 7, countryId: 11 }]);
  assert.deepEqual(combined[1].identities, [{ firstHour: 4, lastHour: 8, slot: 7, countryId: 11 }]);
  assert.deepEqual(index.aliasesAt(index.selected(['A']), 5), ['B']);
  assert.deepEqual(index.aliasesAt(index.selected(['A']), 7), ['A', 'B'], 'aliases outside exact evidence remain');
});


test('5,000 exact aliases replace rough aliases at their hours with one holder per slot', () => {
  const rough = Array.from({ length: 5000 }, (_, slot) => ({ tag: `rough-${slot}`,
    identities: [{ firstHour: 0, lastHour: 100, slot, countryId: slot }] }));
  const exact = rough.map((history, slot) => ({ tag: `exact-${slot}`,
    identities: [{ ...history.identities[0], firstHour: 50, lastHour: 50 }] }));
  const combined = overlayCountryIdentities(rough, exact), index = countryIdentityIndex(combined);
  assert.equal(combined.length, 10_000);
  assert.ok(combined.slice(0, 5000).every(history => history.identities.length === 2));
  const keys = index.selected(rough.map(history => history.tag));
  assert.deepEqual(index.aliasesAt(keys, 50), exact.map(history => history.tag));
  assert.deepEqual(index.aliasesAt(keys, 49), rough.map(history => history.tag));
});
