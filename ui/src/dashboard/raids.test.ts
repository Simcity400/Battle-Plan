import { test } from 'node:test';
import assert from 'node:assert/strict';
import { eventsThrough, raidAt, raidChangeHours, raidPreparationAt, raidPreparationText, raidStatus, validateRaidLog } from './raids.ts';
import { observedEnd, phaseRun, syntheticEpisode, syntheticRaidLog } from './raids.fixtures.ts';

const episodes = [
  syntheticEpisode('1', 10, 'ENG', 20, 30, true),
  syntheticEpisode('2', 15, 'PRC', 25, 60, true),
  syntheticEpisode('3', 40, 'ENG', 45, null, false),
  syntheticEpisode('4', 50, null, 55, null, false),
  syntheticEpisode('5', 52, 'PRC', 54, 58, false),
];
const CODES = syntheticRaidLog([]);

test('a raid at an hour shows only what was read by then: its type, icon, place, state, membership and end', () => {
  const raid = syntheticEpisode('6', 10, 'ENG', null, 30, true, { memberHour: 11, raidTypeHour: 12, iconHour: 13, placeHour: 14, stateHour: 15,
    ends: [observedEnd(30, 'ENG'), { hour: 40, endedHour: 25, exact: true, source: 'nativeEndDate', raider: 'ENG' }] });
  assert.equal(raidAt(CODES, raid, 10), null, 'not among the asked countries before a read names one');
  const at11 = raidAt(CODES, raid, 11)!;
  assert.deepEqual([at11.episode.raidType, at11.episode.icon, at11.episode.iconMissing, at11.episode.state], [null, null, "The raid's type is not recorded", null]);
  const at12 = raidAt(CODES, raid, 12)!.episode;
  assert.deepEqual([at12.raidType, at12.icon, at12.iconMissing], ['mod/sky_raid', null, "The raid's target is not recorded in full"]);
  assert.deepEqual([raidAt(CODES, raid, 13)!.episode.icon, raidAt(CODES, raid, 13)!.episode.province, raidAt(CODES, raid, 14)!.episode.state, raidAt(CODES, raid, 15)!.episode.state],
    ['GFX_mod_raid_target', null, null, 7]);
  assert.deepEqual([raidAt(CODES, raid, 29)!.end, raidAt(CODES, raid, 35)!.since, raidAt(CODES, raid, 45)!.since], [null, 30, 25]);
});

test('a raid at an hour shows its phase run, dated from where that phase began, and its last one after its row is gone', () => {
  const raid = syntheticEpisode('9', 10, 'ENG', 20, 30, true, { runs: [phaseRun(10, 14, 2), phaseRun(17, 19, 2), phaseRun(20, 29, 4), phaseRun(30, 40, 5, 3)] });
  assert.equal(raidAt(CODES, raid, 9), null);
  assert.deepEqual([raidAt(CODES, raid, 18)!.since, raidAt(CODES, raid, 18)!.current?.firstHour], [10, 17], 'joined back over the gap');
  assert.equal(raidAt(CODES, raid, 15)!.current, null);
  const data = syntheticRaidLog([raid]);
  assert.equal(raidStatus(data, raidAt(CODES, raid, 15)!), 'Last known: Phase 2');
  assert.equal(raidStatus(data, raidAt(CODES, raid, 35)!), 'Phase 5 · Outcome 3');
  assert.equal(raidStatus(data, raidAt(CODES, raid, 50)!), 'Last known: Phase 5 · Outcome 3');
  assert.deepEqual(raidChangeHours([raid]), [10, 15, 17, 20, 20, 30, 30, 41]);
  assert.equal(eventsThrough(raidChangeHours([raid]), 20), 5);
});

test('preparation hours are read only at an hour of their runs, and shown only while preparing', () => {
  const raid = syntheticEpisode('9', 10, 'ENG', 20, null, false, { preparation: [[10, 14, 3, 2], [17, 19, 40, 0]] });
  assert.deepEqual([raidPreparationAt(raid, 12), raidPreparationAt(raid, 15), raidPreparationAt(raid, 18)], [7, null, 40]);
  const data = syntheticRaidLog([raid]);
  assert.equal(raidPreparationText(data, raidAt(CODES, raid, 12)!, 12), '7 h prepared');
  assert.equal(raidPreparationText(data, raidAt(CODES, raid, 15)!, 15), 'Preparation not known');
  assert.equal(raidPreparationText(data, raidAt(CODES, raid, 25)!, 25), null);
});


test('a raid whose victim is neither named nor said missing, won without ending, or with phases out of order is refused', () => {
  const reply = syntheticRaidLog(episodes);
  assert.deepEqual(validateRaidLog(reply).episodes, reply.episodes);
  assert.throws(() => validateRaidLog({ ...reply, episodes: [{ ...episodes[0], victim: null }] }));
  assert.throws(() => validateRaidLog({ ...reply, episodes: [{ ...episodes[2], succeededHour: 50 }] }));
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
  const vanishedAt = raidAt(CODES, vanished, 35)!;
  assert.equal(vanishedAt.since, 30);
  assert.equal(vanishedAt.sinceExact, true);
  assert.equal(raidStatus(data, vanishedAt), 'Last known: Phase 4 · raid outcome was not recorded');
  assert.equal(raidStatus(data, raidAt(CODES, unsupported, 35)!), 'Phase 5 · Unsupported outcome (99)');
});
