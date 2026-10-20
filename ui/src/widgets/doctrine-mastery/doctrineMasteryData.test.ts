import test from 'node:test';
import assert from 'node:assert/strict';
import { activeTracks, chosen, dailyText, doctrineIcons, doctrinesAt, requirementMet, rewardDescriptions, rewardKey, rewardUnlocked, trackMarks, trackRatio, unlockedRewards, validateDoctrineMastery } from './doctrineMasteryData.ts';

const reply = validateDoctrineMastery({
  window: { firstHour: 0, lastHour: 10, known: [{ firstHour: 0, lastHour: 10 }], reads: [[0, 10, 0]], series: [
    { name: 'GER@7|land|t1|mastery', runs: [[0, 10, '15000000', true]] },
    { name: 'GER@7|land|t1|bank', runs: [[0, 10, '0', true]] },
    { name: 'GER@7|land|t1|effective', runs: [[0, 10, '250000', true]] },
    { name: 'GER@7|land|t1|daily', runs: [[0, 10, '100000', true]] },
    { name: 'GER@7|land|t1|sub', runs: [[0, 10, '1', true]] },
    { name: 'GER@7|land|t1|rewards', runs: [[0, 10, '1', true]] },
    { name: 'GER@7|land|t2|mastery', runs: [[0, 10, '0', true]] },
    { name: 'GER@7|land|grand', runs: [[0, 10, '2', true]] },
    { name: 'GER@7|land|completed|0', runs: [[0, 10, '1', true]] },
  ] },
  folders: [{ id: 'land', name: 'Land', icon: null, grand: [{ id: 'mobile', tokenId: '2', name: 'Mobile warfare', icon: null }], tracks: [
    { id: 't1', name: 'Infantry', subdoctrines: [{ id: 'sub_a', tokenId: '1', name: 'Sub A', icon: null, rewards: [
      { id: 'r1', name: 'First', description: null, threshold: '5000000' },
      { id: 'r2', name: 'Second', description: 'More', threshold: '25000000' },
      { id: 'r3', name: 'Last', description: null, threshold: '40000000' }] }] },
    { id: 't2', name: 'Armour', subdoctrines: [] },
    { id: 't3', name: 'Unused', subdoctrines: [] }] },
    { id: 'mod', name: 'Mod folder', icon: null, grand: [], tracks: [] }],
  tokens: { 1: 'sub_a', 2: 'mobile' },
  warnings: [],
});

test('tracks resolve their subdoctrine and keep a track with none apart from one at zero', () => {
  const at = doctrinesAt(reply, 'GER', 5)!;
  assert.equal(at.folders.length, 1);
  const [land] = at.folders;
  assert.equal(land.grand?.name, 'Mobile warfare');
  assert.deepEqual(land.tracks.map(track => [track.track.id, track.subdoctrine?.id ?? null, track.mastery]), [['t1', 'sub_a', '15000000'], ['t2', null, '0']]);
  assert.equal(activeTracks(at.folders), 1);
  assert.deepEqual(land.tracks[0].completed.map(sub => sub.id), ['sub_a']);
});

test('marks sit at each authored threshold on the last threshold scale, reached by the recorded reward count', () => {
  const [track] = doctrinesAt(reply, 'GER', 5)!.folders[0].tracks;
  assert.equal(trackRatio(track), 0.375);
  assert.deepEqual(trackMarks(track, false).map(mark => [mark.at, mark.reached]), [[0.125, true], [0.625, false], [1, false]]);
  assert.equal(dailyText(track.effective), '+2.5 a day');
  assert.equal(dailyText(null), null);
});

test('a track without thresholds has no scale and unrecorded hours are unknown', () => {
  const [, empty] = doctrinesAt(reply, 'GER', 5)!.folders[0].tracks;
  assert.equal(trackRatio(empty), null);
  assert.deepEqual(trackMarks(empty, true), []);
  assert.equal(doctrinesAt(reply, 'GER', 11), null);
  assert.deepEqual(doctrinesAt(reply, 'ITA', 5)!.folders, []);
});

test('doctrine icons are the sprite keys of folders, grand doctrines and subdoctrines', () => {
  const keyed = { ...reply, folders: [{ ...reply.folders[0], icon: 'GFX_land',
    grand: [{ ...reply.folders[0].grand[0], icon: 'GFX_mobile' }],
    tracks: [{ ...reply.folders[0].tracks[0], subdoctrines: [{ ...reply.folders[0].tracks[0].subdoctrines[0], icon: 'GFX_sub_a' }] }] }] };
  assert.deepEqual(doctrineIcons(keyed), ['GFX_land', 'GFX_mobile', 'GFX_sub_a']);
  assert.deepEqual(doctrineIcons(undefined), []);
});

test('reward descriptions asked at one hour are keyed by folder, track, subdoctrine and position', () => {
  const descriptions = rewardDescriptions(reply);
  assert.equal(descriptions.get(rewardKey('land', 't1', 'sub_a', 1)), 'More');
  assert.equal(descriptions.get(rewardKey('land', 't1', 'sub_a', 0)), null);
  assert.equal(descriptions.has(rewardKey('land', 't1', 'sub_a', 3)), false);
});

test('unlocks follow the recorded reward count even where mastery and requirements disagree', () => {
  const [track] = doctrinesAt(reply, 'GER', 5)!.folders[0].tracks;
  const rewards = track.subdoctrine!.rewards!;
  // Mastery 150 meets the first requirement (50) and the count unlocks it.
  assert.deepEqual([rewardUnlocked(track, 0), requirementMet(track, rewards[0])], [true, true]);
  // The count says nothing was unlocked although the requirement is met: not unlocked.
  const none = { ...track, rewards: '0' };
  assert.deepEqual([rewardUnlocked(none, 0), requirementMet(none, rewards[0])], [false, true]);
  assert.deepEqual(trackMarks(none, false).map(mark => mark.reached), [false, false, false]);
  // The count unlocks a reward whose requirement mastery does not meet: unlocked.
  const ahead = { ...track, rewards: '2' };
  assert.deepEqual([rewardUnlocked(ahead, 1), requirementMet(ahead, rewards[1])], [true, false]);
  assert.deepEqual(unlockedRewards(ahead).map(reward => reward.id), ['r1', 'r2']);
  // Without a recorded count, no unlock is certified.
  const unknown = { ...track, rewards: null };
  assert.equal(rewardUnlocked(unknown, 0), null);
  assert.deepEqual(unlockedRewards(unknown), []);
  // A count beyond the authored rewards unlocks every authored one, and a negative one none.
  assert.equal(unlockedRewards({ ...track, rewards: '9' }).length, 3);
  assert.equal(unlockedRewards({ ...track, rewards: '-1' }).length, 0);
});

test('a recorded choice joins by its interned id, with or without content or a spelling', () => {
  // Recorded structure only: no names, rewards or spellings, yet mastery stays reachable.
  const bare = validateDoctrineMastery({
    window: { firstHour: 0, lastHour: 10, known: [{ firstHour: 0, lastHour: 10 }], reads: [[0, 10, 0]], series: [
      { name: 'GER@7|0|3|mastery', runs: [[0, 10, '15000000', true]] },
      { name: 'GER@7|0|3|sub', runs: [[0, 10, '44', true]] },
    ] },
    folders: [{ id: '0', name: null, icon: null, grand: [], tracks: [
      { id: '3', name: null, subdoctrines: [{ id: '44', tokenId: '44', name: null, icon: null, rewards: null }] }] }],
    tokens: {}, warnings: [],
  });
  const [track] = doctrinesAt(bare, 'GER', 5)!.folders[0].tracks;
  assert.equal(track.mastery, '15000000');
  assert.equal(track.subdoctrine?.tokenId, '44');
  assert.deepEqual(trackMarks(track, true), []);
  // An unwitnessed authored alternative joins by spelling.
  assert.equal(chosen([{ id: 'sub_b', tokenId: null, name: 'B', icon: null }], '9', { 9: 'sub_b' })?.id, 'sub_b');
  assert.equal(chosen([{ id: 'sub_b', tokenId: null, name: 'B', icon: null }], '0', { 0: 'sub_b' }), null);
});
