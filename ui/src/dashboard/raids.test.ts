import { test } from 'node:test';
import assert from 'node:assert/strict';
import { raidTally, recordedSpans, unattributedAt, validateRaidLog } from './raids.ts';
import { observedEnd, syntheticEpisode, syntheticRaidLog } from './raids.fixtures.ts';
import { countryKey } from './countryKey.ts';

const episodes = [
  syntheticEpisode('1', 10, 'ENG', 20, 30, true),
  syntheticEpisode('2', 15, 'PRC', 25, 60, true),
  syntheticEpisode('3', 40, 'ENG', 45, null, false),
  syntheticEpisode('4', 50, null, 55, null, false),
  syntheticEpisode('5', 52, 'PRC', 54, 58, false),
];
const SPAN = { firstHour: 0, lastHour: 200 };
const at = (tally: ReturnType<typeof raidTally>, tag: string, hour: number) => {
  const run = tally.series.find(item => item.tag === tag)?.runs.find(item => item.firstHour <= hour && hour <= item.lastHour);
  return run ? [run.launched, run.succeeded] : null;
};

test('a tally counts each raider over time: launches as they happen, wins as they end, unnamed raiders apart', () => {
  const tally = raidTally(episodes, SPAN, [], ['PRC', 'ENG'], null);
  assert.deepEqual(tally.series.map(item => item.tag), ['ENG', 'PRC']);
  assert.deepEqual(raidTally(episodes, SPAN, [], [], null).series, [], 'no country chosen, none counted');
  assert.deepEqual([at(tally, 'ENG', 19), at(tally, 'ENG', 20), at(tally, 'ENG', 30), at(tally, 'ENG', 45)], [[0, 0], [1, 0], [1, 1], [2, 1]]);
  assert.deepEqual([at(tally, 'PRC', 26), at(tally, 'PRC', 58), at(tally, 'PRC', 60)], [[1, 0], [2, 0], [2, 1]]);
  assert.deepEqual([unattributedAt(tally, 54), unattributedAt(tally, 55)], [0, 1], 'the unnamed raider is unattributed from its launch');
  assert.deepEqual(tally.hours, [20, 25, 30, 45, 54, 60]);
});

test('a tally is not recorded over the hours the recording misses, and its count holds across them', () => {
  const tally = raidTally(episodes, SPAN, [{ firstHour: 26, lastHour: 35 }], ['ENG'], null);
  assert.equal(at(tally, 'ENG', 30), null);
  assert.deepEqual(at(tally, 'ENG', 36), [1, 1]);
  assert.deepEqual(recordedSpans(SPAN, [{ firstHour: 26, lastHour: 35 }, { firstHour: 190, lastHour: 400 }]), [{ firstHour: 0, lastHour: 25 }, { firstHour: 36, lastHour: 189 }]);
});

test('chosen countries are listed at zero when they raided none, and only they are counted', () => {
  const tally = raidTally(episodes, SPAN, [], ['GER', 'ENG'], null);
  assert.deepEqual(tally.series.map(item => item.tag), ['ENG', 'GER']);
  assert.deepEqual(at(tally, 'GER', 100), [0, 0]);
});

test('a chosen slot counts its country under every tag it held, and never another country holding one of them', () => {
  const slot = countryKey(3);
  const raids = [
    syntheticEpisode('1', 10, 'ENG', 20, 30, true, { raiderSlot: 3 }),
    syntheticEpisode('2', 50, 'PRC', 54, null, false, { raiderSlot: 3 }),
    syntheticEpisode('3', 40, 'ENG', 45, null, false, { raiderSlot: 5 }),
    syntheticEpisode('4', 1, 'GER', null, 5, true, { raiderSlot: 3, succeededHour: 20 }),
  ];
  const tally = raidTally(raids, SPAN, [], [slot, 'ENG'], null);
  assert.deepEqual([at(tally, slot, 19), at(tally, slot, 20), at(tally, slot, 100), at(tally, 'ENG', 100)], [[0, 0], [1, 1], [2, 2], [1, 0]],
    'a success counts where it was first read, for the slot that owned it at its end, whatever the slot is called then');
});

test('a chosen slot keeps its raid when no tag names it, and a tag tally never borrows the opening tag', () => {
  const untagged = syntheticEpisode('9', 10, 'GER', 20, 30, true, { raiderSlot: 3, launchedRaider: null, ends: [observedEnd(30, null)] });
  const slot = countryKey(3);
  assert.deepEqual(at(raidTally([untagged], SPAN, [], [slot], null), slot, 100), [1, 1], 'the selected slot counts first');
  const byTag = raidTally([untagged], SPAN, [], ['GER'], null);
  assert.deepEqual([at(byTag, 'GER', 100), unattributedAt(byTag, 100)], [[0, 0], 1], 'no tag at the launch or the end is unattributed');
});

test('a raid is unattributed only while no tag is known for an event it counts: an end tag read later attributes it from that hour', () => {
  const late = syntheticEpisode('9', 10, null, null, 10, true, { ends: [observedEnd(10, null), { hour: 20, endedHour: 5, exact: true, source: 'nativeEndDate', raider: 'GER' }] });
  const tally = raidTally([late], SPAN, [], ['GER'], null);
  assert.deepEqual([unattributedAt(tally, 9), unattributedAt(tally, 10), unattributedAt(tally, 19), unattributedAt(tally, 20)], [0, 1, 1, 0]);
  assert.deepEqual([at(tally, 'GER', 19), at(tally, 'GER', 20)], [[0, 0], [0, 1]]);
});

test('a success counts for the end tag known at each hour: a native end date read later moves it, from the hour it was read', () => {
  const raid = syntheticEpisode('5', 10, 'DDR', null, 10, true, { ends: [observedEnd(10, 'DDR'), { hour: 20, endedHour: 5, exact: true, source: 'nativeEndDate', raider: 'GER' }] });
  const tally = raidTally([raid], SPAN, [], ['GER', 'DDR'], null);
  assert.deepEqual([at(tally, 'DDR', 15), at(tally, 'GER', 15), at(tally, 'DDR', 25), at(tally, 'GER', 25)], [[0, 1], [0, 0], [0, 0], [0, 1]]);
});

test('a merged raid credits each tally event to the raider tag held at that event hour', () => {
  const renamed = syntheticEpisode('7', 10, 'GER', 20, 30, true, { launchedRaider: 'DDR', ends: [observedEnd(30, 'DDR')] });
  const tally = raidTally([renamed], SPAN, [], ['GER', 'DDR'], null);
  assert.deepEqual([at(tally, 'GER', 100), at(tally, 'DDR', 100)], [[0, 0], [1, 1]]);
});

test('a success is counted from the read that first saw it, never at an earlier native end date', () => {
  const late = syntheticEpisode('8', 1, 'ENG', null, 5, true, { ends: [{ hour: 20, endedHour: 5, exact: true, source: 'nativeEndDate', raider: 'ENG' }], succeededHour: 20 });
  const tally = raidTally([late], SPAN, [], ['ENG'], null);
  assert.deepEqual([at(tally, 'ENG', 10), at(tally, 'ENG', 20)], [[0, 0], [0, 1]]);
});

test('a top keeps each country for the spans it is among the leaders, ties in tag order', () => {
  const tally = raidTally([...episodes, syntheticEpisode('6', 65, 'PRC', 70, null, false)], SPAN, [], [], { count: 1, metric: 'launched' });
  const membership = new Map(tally.series.map(item => [item.tag, item.membership]));
  assert.deepEqual(membership.get('ENG'), [{ firstHour: 20, lastHour: 69 }]);
  assert.deepEqual(membership.get('PRC'), [{ firstHour: 70, lastHour: 200 }]);
  const wins = raidTally(episodes, SPAN, [], [], { count: 5, metric: 'succeeded' });
  assert.deepEqual(wins.series.map(item => [item.tag, item.membership]), [['ENG', [{ firstHour: 30, lastHour: 200 }]], ['PRC', [{ firstHour: 60, lastHour: 200 }]]]);
});

test('a raid whose victim is neither named nor said missing, won without ending, or with phases out of order is refused', () => {
  const reply = syntheticRaidLog(episodes);
  assert.deepEqual(validateRaidLog(reply).episodes, reply.episodes);
  assert.throws(() => validateRaidLog({ ...reply, episodes: [{ ...episodes[0], victim: null }] }));
  assert.throws(() => validateRaidLog({ ...reply, episodes: [{ ...episodes[2], succeededHour: 50 }] }));
  assert.throws(() => validateRaidLog({ ...reply, episodes: [{ ...episodes[0], runs: [...episodes[0].runs].reverse() }] }));
});
