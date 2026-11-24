import test from 'node:test';
import assert from 'node:assert/strict';
import type { DoctrineMasteryReply } from '../../generated/dashboard';
import { activeTracks, rewardUnlocked, trackMarks, trackRatio, unlockedRewards, unlockMastery, validateDoctrineMastery, validateDoctrineRewards } from './doctrineMasteryData.ts';

const sub = { id: 'sub_a', name: 'Sub A', icon: null };
const reply: DoctrineMasteryReply = validateDoctrineMastery({
  folders: [{ ordinal: 0, id: 'land', name: 'Land', icon: null, grand: { id: 'mobile', name: 'Mobile warfare', icon: null }, completed: [sub], tracks: [
    { ordinal: 0, name: 'Infantry', sub, unlocked: 1, rewards: [
      { id: 'r1', name: 'First', threshold: '5000000' }, { id: 'r2', name: 'Second', threshold: '25000000' }, { id: 'r3', name: 'Last', threshold: '10000000' }] },
    { ordinal: 1, name: 'Armour', sub: null, unlocked: 0, rewards: [] }] }],
  warnings: [],
});

test('each threshold is its own step: marks sit at the running sum on the whole track’s scale, reached by the recorded reward count', () => {
  const [infantry, armour] = reply.folders[0].tracks;
  assert.deepEqual(unlockMastery(infantry), [5000000n, 30000000n, 40000000n]);
  assert.equal(trackRatio(infantry, 15000000n), 0.375);
  assert.deepEqual(trackMarks(infantry, false).map(mark => [mark.at, mark.reached]), [[0.125, true], [0.75, false], [1, false]]);
  assert.equal(rewardUnlocked(infantry, 0), true);
  assert.deepEqual(unlockedRewards(infantry).map(item => item.id), ['r1']);
  assert.equal(trackRatio(armour, 0n), null, 'a track without thresholds has no scale, not a zero one');
  assert.equal(activeTracks(reply.folders), 1);
});

test('a reward without a threshold, or a mastery not recorded, leaves the meter unknown, never invented', () => {
  const track = { ...reply.folders[0].tracks[0], rewards: [{ id: 'r1', name: null, threshold: '5000000' }, { id: 'r2', name: null, threshold: null }] };
  assert.deepEqual(unlockMastery(track), [5000000n, null]);
  assert.equal(trackRatio(track, 1n), null);
  assert.equal(trackRatio(reply.folders[0].tracks[0], null), null);
});

test('a malformed reply is refused', () => {
  assert.throws(() => validateDoctrineMastery({ ...reply, folders: [{ ...reply.folders[0], tracks: [{ ...reply.folders[0].tracks[0], unlocked: '1' }] }] }));
  assert.throws(() => validateDoctrineRewards({ sub: null, warnings: [], rewards: [{ id: 'r', name: null, threshold: null, description: 1 }] }));
});
