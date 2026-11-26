import assert from 'node:assert/strict';
import test from 'node:test';
import type { CountryGroupReply } from '../generated/dashboard';
import { worldRequestTags } from './groups.ts';

const groups: CountryGroupReply = { branchId: '1', warnings: [], warStarts: [], capitulations: [], identityChanges: [], humans: [], wars: [], unobserved: { humans: [], factions: [], wars: [], unattributed: [] },
  countryIdentities: [{ tag: 'EARLY', identities: [{ firstHour: 10, lastHour: 15, slot: 1, countryId: 10 }] },
    { tag: 'LATE', identities: [{ firstHour: 16, lastHour: 20, slot: 2, countryId: 20 }] }],
  factions: [{ id: 'synthetic-faction', name: 'Synthetic group', recordedName: 'Synthetic group', firstHour: 10, lastHour: 20, nameReason: null, names: [{ firstHour: 10, lastHour: 20, name: 'Synthetic group', reason: null, token: null }], leaders: [], incomplete: [], members: [{ tag: 'EARLY', firstHour: 10, lastHour: 15 }, { tag: 'LATE', firstHour: 16, lastHour: 20 }] }] };

test('whole-branch group requests reuse recorded membership union while pending groups request nothing', () => {
  const choice = { kind: 'group', group: { type: 'faction', id: 'synthetic-faction' }, membership: 'hour', aggregate: false } as const;
  assert.deepEqual(worldRequestTags(choice, [], undefined, 20), { tags: [], ready: false, reason: null });
  assert.deepEqual(worldRequestTags(choice, [], groups, 20), { tags: ['EARLY', 'LATE'], ready: true, reason: null });
  assert.equal(worldRequestTags({ kind: 'selected' }, [], groups, 20).ready, false);
  assert.deepEqual(worldRequestTags({ kind: 'all' }, [], groups, 20), { tags: [], ready: true, reason: null });
});

test('a later roster gap leaves the whole-branch request ready for members known before it, in either membership', () => {
  const gapped = { ...groups, factions: groups.factions.map(faction => ({ ...faction, incomplete: [{ firstHour: 18, lastHour: 19 }] })) } as CountryGroupReply;
  for (const membership of ['hour', 'ever'] as const) {
    const choice = { kind: 'group', group: { type: 'faction', id: 'synthetic-faction' }, membership, aggregate: false } as const;
    assert.equal(worldRequestTags(choice, [], gapped, 20).ready, true, membership);
  }
  const late = { ...gapped, factions: gapped.factions.map(faction => ({ ...faction, incomplete: [{ firstHour: 12, lastHour: 13 }] })) } as CountryGroupReply;
  const ever = { kind: 'group', group: { type: 'faction', id: 'synthetic-faction' }, membership: 'ever', aggregate: false } as const;
  assert.deepEqual(worldRequestTags(ever, [], late, 20), { tags: ['EARLY', 'LATE'], ready: true, reason: null }, 'a member joining after the gap is still in the whole-branch request');
});

test('a whole-branch group request names the recorded member tags only, never another country that held a member alias', () => {
  const renamed = { ...groups,
    countryIdentities: [{ tag: 'AAA', identities: [{ firstHour: 0, lastHour: 9, slot: 1, countryId: 10 }] },
      { tag: 'BBB', identities: [{ firstHour: 0, lastHour: 9, slot: 2, countryId: 20 }, { firstHour: 10, lastHour: 20, slot: 1, countryId: 10 }] }],
    factions: groups.factions.map(faction => ({ ...faction, members: [{ tag: 'AAA', firstHour: 0, lastHour: 5 }] })) } as CountryGroupReply;
  const ever = { kind: 'group', group: { type: 'faction', id: 'synthetic-faction' }, membership: 'ever', aggregate: false } as const;
  assert.deepEqual(worldRequestTags(ever, [], renamed, 20).tags, ['AAA']);
});
