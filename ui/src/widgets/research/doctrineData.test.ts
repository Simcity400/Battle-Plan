import assert from 'node:assert/strict';
import test from 'node:test';
import { masteryText, validateDoctrines } from './doctrineData.ts';

test('doctrine mastery displays exact signed fixed5 beyond number precision', () => {
  assert.equal(masteryText('9223372036854775807'), '92233720368547.75807');
  assert.equal(masteryText('-1'), '\u22120.00001');
  assert.equal(masteryText('100000'), '1');
});

test('doctrine replies require effective and unit-source gain independently', () => {
  const reply = { branchId: '1', tag: 'MOD', hour: 1, warnings: [], inventory: [], folders: [{ ordinal: 0, name: 'Mod folder', grandDoctrine: null, completed: [], benefits: '', authoredCost: '', milestones: [], tracks: [{ ordinal: 0, name: 'Mod track', subDoctrine: null, rewards: 0, mastery: '0', masteryBank: '-1', dailyMastery: '34', unitDailyMastery: '12', rewardRequirements: [], rewardBenefits: [], benefits: '', authoredCost: '', commanders: [] }] }] };
  assert.equal(validateDoctrines(reply), reply);
  assert.throws(() => validateDoctrines({ ...reply, folders: [{ ...reply.folders[0], tracks: [{ ...reply.folders[0].tracks[0], unitDailyMastery: undefined }] }] }));
  assert.throws(() => validateDoctrines({ ...reply, folders: [{ ...reply.folders[0], tracks: [{ ...reply.folders[0].tracks[0], mastery: '9223372036854775808' }] }] }));
});
