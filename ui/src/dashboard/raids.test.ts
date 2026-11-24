import { test } from 'node:test';
import assert from 'node:assert/strict';
import { eventsThrough, raidAt, raidChangeHours, raidPreparationAt, raidPreparationText, raidStatus, raidTally, recordedSpans, validateRaidLog } from './raids.ts';
import { phaseRun, syntheticEpisode, syntheticRaidLog } from './raids.fixtures.ts';
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
  assert.equal(tally.unattributed, 1);
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
  const tally = raidTally(episodes, SPAN, [], [slot, 'ENG'], null, (raider, hour) => (raider === 'ENG' && hour <= 35) || (raider === 'PRC' && hour >= 50) ? slot : raider);
  assert.deepEqual([at(tally, slot, 100), at(tally, 'ENG', 100)], [[2, 2], [1, 0]]);
});

test('a top keeps each country for the spans it is among the leaders, ties in tag order', () => {
  const tally = raidTally([...episodes, syntheticEpisode('6', 65, 'PRC', 70, null, false)], SPAN, [], [], { count: 1, metric: 'launched' });
  const membership = new Map(tally.series.map(item => [item.tag, item.membership]));
  assert.deepEqual(membership.get('ENG'), [{ firstHour: 20, lastHour: 69 }]);
  assert.deepEqual(membership.get('PRC'), [{ firstHour: 70, lastHour: 200 }]);
  const wins = raidTally(episodes, SPAN, [], [], { count: 5, metric: 'succeeded' });
  assert.deepEqual(wins.series.map(item => [item.tag, item.membership]), [['ENG', [{ firstHour: 30, lastHour: 200 }]], ['PRC', [{ firstHour: 60, lastHour: 200 }]]]);
});

test('a raid at an hour shows its phase run, dated from where that phase began, and its last one after its row is gone', () => {
  const raid = syntheticEpisode('9', 10, 'ENG', 20, 30, true, { runs: [phaseRun(10, 14, 2), phaseRun(17, 19, 2), phaseRun(20, 29, 4), phaseRun(30, 40, 5, 3)] });
  assert.equal(raidAt(raid, 9), null);
  assert.deepEqual([raidAt(raid, 18)!.since, raidAt(raid, 18)!.current?.firstHour], [10, 17], 'joined back over the gap');
  assert.equal(raidAt(raid, 15)!.current, null);
  const data = syntheticRaidLog([raid]);
  assert.equal(raidStatus(data, raidAt(raid, 15)!), 'Last known: Phase 2');
  assert.equal(raidStatus(data, raidAt(raid, 35)!), 'Phase 5 · Outcome 3');
  assert.equal(raidStatus(data, raidAt(raid, 50)!), 'Last known: Phase 5 · Outcome 3');
  assert.deepEqual(raidChangeHours([raid]), [10, 15, 17, 20, 20, 30, 30, 41]);
  assert.equal(eventsThrough(raidChangeHours([raid]), 20), 5);
});

test('preparation hours are read only at an hour of their runs, and shown only while preparing', () => {
  const raid = syntheticEpisode('9', 10, 'ENG', 20, null, false, { preparation: [[10, 14, 3, 2], [17, 19, 40, 0]] });
  assert.deepEqual([raidPreparationAt(raid, 12), raidPreparationAt(raid, 15), raidPreparationAt(raid, 18)], [7, null, 40]);
  const data = syntheticRaidLog([raid]);
  assert.equal(raidPreparationText(data, raidAt(raid, 12)!, 12), '7 h prepared');
  assert.equal(raidPreparationText(data, raidAt(raid, 15)!, 15), 'Preparation not known');
  assert.equal(raidPreparationText(data, raidAt(raid, 25)!, 25), null);
});


test('a raid whose victim is neither named nor said missing, won without ending, or with phases out of order is refused', () => {
  const reply = syntheticRaidLog(episodes);
  assert.deepEqual(validateRaidLog(reply).episodes, reply.episodes);
  assert.throws(() => validateRaidLog({ ...reply, episodes: [{ ...episodes[0], victim: null }] }));
  assert.throws(() => validateRaidLog({ ...reply, episodes: [{ ...episodes[2], succeeded: true }] }));
  assert.throws(() => validateRaidLog({ ...reply, episodes: [{ ...episodes[0], runs: [...episodes[0].runs].reverse() }] }));
});

test('vanished raids report unrecorded outcomes rather than canceled, and unsupported codes keep raw values', () => {
  const vanished = syntheticEpisode('10', 10, 'ENG', 20, null, false, {
    lastHour: 29,
    runs: [phaseRun(10, 19, 2), phaseRun(20, 29, 4)],
    disappearedHour: 30,
    disappearedExact: true,
    outcomeMissing: 'raid outcome was not recorded',
  });
  const unsupported = syntheticEpisode('11', 10, 'ENG', 20, 30, false, {
    runs: [phaseRun(10, 19, 2), phaseRun(20, 29, 4), { ...phaseRun(30, 40, 5, 99), outcomeName: 'Unsupported outcome (99)' }],
    outcome: 99,
    outcomeName: 'Unsupported outcome (99)',
    outcomeMissing: 'Unsupported raid outcome (99)',
  });
  const data = syntheticRaidLog([vanished, unsupported]);
  const vanishedAt = raidAt(vanished, 35)!;
  assert.equal(vanishedAt.since, 30);
  assert.equal(vanishedAt.sinceExact, true);
  assert.equal(raidStatus(data, vanishedAt), 'Last known: Phase 4 · raid outcome was not recorded');
  assert.equal(raidStatus(data, raidAt(unsupported, 35)!), 'Phase 5 · Unsupported outcome (99)');
});
