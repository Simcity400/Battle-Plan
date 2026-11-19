import test from 'node:test';
import assert from 'node:assert/strict';
import { liveKey, valuesOf } from '../../charts/live.ts';
import type { DoctrineMasteryReply, DoctrineRewardsReply } from '../../generated/dashboard';
import { activeTracks, masteryStructure, requirementMet, rewardDescription, rewardUnlocked, trackMarks, trackNumbers, trackRatio, trackTexts, unlockedRewards,
  unlockMastery, validateDoctrineMastery, validateDoctrineRewards } from './doctrineMasteryData.ts';

const sub = { id: 'sub_a', name: 'Sub A', icon: null };
const reply: DoctrineMasteryReply = validateDoctrineMastery({
  folders: [{ ordinal: 0, id: 'land', name: 'Land', icon: null, grand: { id: 'mobile', name: 'Mobile warfare', icon: null }, completed: [sub], tracks: [
    { ordinal: 0, name: 'Infantry', sub, unlocked: 1, rewards: [
      { id: 'r1', name: 'First', threshold: '5000000' }, { id: 'r2', name: 'Second', threshold: '25000000' }, { id: 'r3', name: 'Last', threshold: '10000000' }] },
    { ordinal: 1, name: 'Armour', sub: null, unlocked: 0, rewards: [] }] }],
  warnings: [],
});

const cells = (entries: Record<string, number>, reason = 'Not recorded at this hour') =>
  valuesOf(new Map(Object.entries(entries).map(([key, value]) => [liveKey('0|0', key), value])), reason);

test('each threshold is its own step: marks sit at the running sum on the whole track’s scale, reached by the recorded reward count', () => {
  const [infantry, armour] = reply.folders[0].tracks;
  assert.deepEqual(unlockMastery(infantry), [5000000n, 30000000n, 40000000n]);
  assert.equal(trackRatio(infantry, 15000000n), 0.375);
  assert.deepEqual(trackMarks(infantry, false).map(mark => [mark.at, mark.reached]), [[0.125, true], [0.75, false], [1, false]]);
  assert.equal(rewardUnlocked(infantry, 0), true);
  assert.equal(requirementMet(infantry, 1, 15000000n), false, 'the unlock is the recorded count, the requirement the mastery: kept apart');
  assert.deepEqual(unlockedRewards(infantry).map(item => item.id), ['r1']);
  assert.equal(trackRatio(armour, 0n), null, 'a track without thresholds has no scale, not a zero one');
  assert.equal(activeTracks(reply.folders), 1);
});

test('a reward without a threshold, or a mastery not recorded, leaves the meter and the requirement unknown, never invented', () => {
  const track = { ...reply.folders[0].tracks[0], rewards: [{ id: 'r1', name: null, threshold: '5000000' }, { id: 'r2', name: null, threshold: null }] };
  assert.deepEqual(unlockMastery(track), [5000000n, null]);
  assert.equal(trackRatio(track, 1n), null);
  assert.equal(requirementMet(track, 1, 1n), null);
  assert.equal(trackRatio(reply.folders[0].tracks[0], null), null);
  assert.equal(requirementMet(reply.folders[0].tracks[0], 0, null), null);
});

test('a track’s numbers are its live cells: texts painted from them, a missing mastery named, never zero', () => {
  const numbers = trackNumbers(cells({ mastery: 15000000, bank: 0, daily: 100000, effective: 250000 }), '0|0');
  assert.deepEqual([numbers.mastery, numbers.bank, numbers.daily, numbers.effective], [15000000n, 0n, 100000n, 250000n]);
  const texts = trackTexts(numbers);
  assert.equal(texts.bank, '', 'no bank shows nothing, the cell stays');
  assert.ok(texts.daily.startsWith('+') && texts.daily.endsWith(' a day'));
  const missing = trackNumbers(cells({}, 'DoctrineTracks was not available at this hour'), '0|0');
  assert.equal(missing.mastery, null);
  assert.equal(trackTexts(missing).mastery, 'DoctrineTracks was not available at this hour');
  assert.equal(trackTexts(missing).daily, '');
});

test('numbers do not change what renders; rows, names and unlocks do', () => {
  const later = structuredClone(reply);
  later.folders[0].grand = { icon: null, name: 'Mobile warfare', id: 'mobile' };
  assert.equal(masteryStructure(later), masteryStructure(reply));
  later.folders[0].tracks[0].unlocked = 2;
  assert.notEqual(masteryStructure(later), masteryStructure(reply));
});

test('a reward’s description is its place in a rewards reply of the same subdoctrine; none, or another subdoctrine’s, is null', () => {
  const rewards: DoctrineRewardsReply = validateDoctrineRewards({ sub: 'sub_a', warnings: [], rewards: [
    { id: 'r1', name: 'First', threshold: '5000000', description: null }, { id: 'r2', name: 'Second', threshold: null, description: 'More' }] });
  const infantry = reply.folders[0].tracks[0];
  assert.equal(rewardDescription(rewards, infantry, 1), 'More');
  assert.equal(rewardDescription(rewards, infantry, 0), null);
  assert.equal(rewardDescription(undefined, infantry, 0), null);
  assert.equal(rewardDescription({ ...rewards, sub: 'other' }, infantry, 1), null, 'a reply for a subdoctrine since replaced describes nothing here');
});

test('a malformed reply is refused', () => {
  assert.throws(() => validateDoctrineMastery({ ...reply, folders: [{ ...reply.folders[0], tracks: [{ ...reply.folders[0].tracks[0], unlocked: '1' }] }] }));
  assert.throws(() => validateDoctrineRewards({ sub: null, warnings: [], rewards: [{ id: 'r', name: null, threshold: null, description: 1 }] }));
});
