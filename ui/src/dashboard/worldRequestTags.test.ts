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
