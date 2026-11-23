import test from 'node:test';
import assert from 'node:assert/strict';
import { autonomyLanes, formatProgress, formatScore, headKey, holderSince, laneIdentity, laneLabel, levelAt, levelOrder, progressText } from './autonomyData.ts';
import { reachedLines } from '../country-measure/measureSeriesData.ts';
import type { SubjectReply } from '../../generated/dashboard';

const levels = [
  { token: 'autonomy_puppet', name: 'Puppet', ordered: true },
  { token: 'autonomy_dominion', name: 'Dominion', ordered: true },
  { token: 'autonomy_mod_unordered', name: null, ordered: false },
];

const reply: SubjectReply = {
  branchId: '1', levels, warnings: [], unobserved: [{ firstHour: 300, lastHour: 309 }],
  runs: [
    { overlord: 'ENG', subject: 'RAJ', startHour: 50, firstHour: 100, lastHour: 199,
      steps: [{ firstHour: 100, lastHour: 149, level: 'autonomy_puppet' }, { firstHour: 150, lastHour: 199, level: 'autonomy_dominion' }] },
    { overlord: 'GER', subject: 'RAJ', startHour: 200, firstHour: 200, lastHour: 249, steps: [{ firstHour: 200, lastHour: 249, level: null }] },
    { overlord: 'ENG', subject: 'CAN', startHour: 10, firstHour: 90, lastHour: 299, steps: [{ firstHour: 90, lastHour: 299, level: 'autonomy_dominion' }] },
  ],
};

test('a lane per subject joins its runs under every overlord and keeps its level changes', () => {
  const lanes = autonomyLanes(reply, null, false);
  assert.deepEqual(lanes.map(lane => lane.tag), ['CAN', 'RAJ']);
  const raj = lanes[1];
  assert.deepEqual(raj.within, [{ firstHour: 100, lastHour: 249 }]);
  assert.deepEqual(raj.changes, [150]);
  assert.equal(levelAt(raj, 120), 'autonomy_puppet');
  assert.equal(levelAt(raj, 220), null);
  assert.equal(levelAt(raj, 260), undefined);
});

test('chosen overlords keep the subjects they held at any hour; chosen subjects keep themselves', () => {
  assert.deepEqual(autonomyLanes(reply, new Set(['GER']), false).map(lane => lane.tag), ['RAJ']);
  assert.deepEqual(autonomyLanes(reply, new Set(['CAN']), true).map(lane => lane.tag), ['CAN']);
  assert.deepEqual(autonomyLanes(reply, new Set(['CAN']), false), []);
});

test('an unrecorded level, an unobserved hour and a released subject are said, never shown as a level', () => {
  const [can, raj] = autonomyLanes(reply, null, false);
  assert.equal(laneLabel(raj, levels, 220, reply.unobserved).label, 'Level not recorded');
  assert.equal(laneLabel(raj, levels, 260, reply.unobserved).label, 'No longer a subject');
  assert.equal(laneLabel(can, levels, 305, reply.unobserved).label, 'Not recorded at this hour');
  assert.equal(laneLabel(can, levels, 50, reply.unobserved).label, 'Not yet a subject');
  const order = levelOrder(levels);
  assert.deepEqual(order.get('autonomy_puppet'), { index: 0, lowest: true, highest: false });
  assert.deepEqual(order.get('autonomy_dominion'), { index: 1, lowest: false, highest: true });
  assert.deepEqual(order.get('autonomy_mod_unordered'), { index: 2, lowest: false, highest: false });
  assert.equal(formatProgress('34960'), '35.0%');
  assert.equal(formatProgress(null), 'Not recorded');
  assert.equal(formatProgress(null, 'No autonomy range'), 'No autonomy range');
  const [line] = reachedLines(['GER'], {
    branchId: '1', measure: 'progress', decimals: 5,
    states: [
      { country: 'GER', hour: 100, run: null, drop: null, missing: 'No autonomy range', notApplicable: true },
      { country: 'GER', hour: 101, run: null, drop: null, missing: "GER's progress was not recorded at this hour", notApplicable: false },
      { country: 'GER', hour: 102, run: null, drop: null, missing: "GER's progress was not recorded at this hour" },
    ],
  });
  assert.equal(progressText(line, 100), 'No autonomy range');
  assert.equal(progressText(line, 101), 'Not recorded', 'unrecorded complete hour keeps main Not recorded wording');
  assert.equal(progressText(line, 102), 'Not recorded', 'truncated or older-host hour without notApplicable keeps main Not recorded wording');
});

test('a score is the game’s one decimal, and a missing one says why', () => {
  assert.equal(formatScore('33495000', 5, null), '335.0');
  assert.equal(formatScore(null, 5, 'Autonomy was not recorded at this hour'), 'Autonomy was not recorded at this hour');
  assert.equal(formatScore('1', undefined, null), 'Not recorded');
});

test('the head key changes when a lane’s level changes, not while it holds', () => {
  const lanes = autonomyLanes(reply, null, true);
  assert.equal(headKey(lanes, levels, 120, reply.unobserved), headKey(lanes, levels, 140, reply.unobserved));
  assert.notEqual(headKey(lanes, levels, 149, reply.unobserved), headKey(lanes, levels, 150, reply.unobserved));
});

test("a lane is named as its tag's holder at the hour shown: a subject no country holds yet is its tag, never a later holder's name", () => {
  const holders = new Map([['SLO', [{ firstHour: 5_000, lastHour: 9_000 }]], ['GER', [{ firstHour: 0, lastHour: 2_999 }, { firstHour: 3_000, lastHour: 9_000 }]]]);
  assert.equal(holderSince(holders, 'SLO', 100), null);
  assert.equal(holderSince(holders, 'SLO', 6_000), 5_000);
  assert.equal(holderSince(holders, 'GER', 3_500), 3_000);
  assert.equal(holderSince(holders, 'XXX', 100), null);
  assert.equal(holderSince(null, 'SLO', 100), undefined, 'holders not read yet: unknown, not absent');
  const named = { name: 'Slovakia', flag: 'flag:slo' };
  assert.deepEqual(laneIdentity('SLO', holderSince(holders, 'SLO', 100), named), { label: 'SLO', countryFlag: { flagReason: 'Country unavailable at this time.' } });
  assert.deepEqual(laneIdentity('SLO', holderSince(holders, 'SLO', 6_000), named), { label: 'Slovakia', countryFlag: named });
  assert.deepEqual(laneIdentity('SLO', undefined, named), { label: 'Slovakia', countryFlag: named });
});
