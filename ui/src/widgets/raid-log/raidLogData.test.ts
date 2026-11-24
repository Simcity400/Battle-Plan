import { test } from 'node:test';
import assert from 'node:assert/strict';
import { raidDescription, raidLogRows, raidLogTypes, raidTooltip, targetNameRequest, targetNames, targetText } from './raidLogData.ts';
import { raidAt } from '../../dashboard/raids.ts';
import { phaseRun, syntheticEpisode, syntheticRaidLog } from '../../dashboard/raids.fixtures.ts';

const all = () => true;

test('the log shows the raids at the hour, each dated by the phase it shows, newest first', () => {
  const data = syntheticRaidLog([syntheticEpisode('1', 10, 'ENG', 20, 30, true), syntheticEpisode('2', 15, 'PRC', 25, null, false), syntheticEpisode('3', 30, 'GER', null, null, false)]);
  assert.deepEqual(raidLogRows(data, 22, [], all, null).map(row => [row.id, row.hour]), [['1', 20], ['2', 15]]);
  assert.deepEqual(raidLogRows(data, 31, [], all, null).map(row => [row.id, row.hour]), [['1', 30], ['3', 30], ['2', 25]]);
  assert.deepEqual(raidLogRows(data, 31, [], tag => tag === 'PRC', null).map(row => row.id), ['2']);
  assert.deepEqual(raidLogRows(data, 31, [], all, 26).map(row => row.id), ['1', '3']);
  const unnamed = syntheticRaidLog([syntheticEpisode('4', 10, null, null, null, false)]);
  assert.equal(raidLogRows(unnamed, 12, [], (tag) => tag !== null, null).length, 0);
});

test('the type filter keeps the chosen types, and its chips are the types the raids show, with their icons', () => {
  const sea = syntheticEpisode('2', 15, 'PRC', 25, null, false, { raidType: 'mod/sea_raid', raidTypeName: null, icon: null });
  const odd = syntheticEpisode('3', 16, 'PRC', 25, null, false, { raidType: 'mod/undeclared', raidTypeName: 'Undeclared raid', icon: null });
  const data = syntheticRaidLog([syntheticEpisode('1', 10, 'ENG', 20, 30, true), sea, odd]);
  assert.deepEqual(raidLogRows(data, 22, ['mod/sea_raid'], all, null).map(row => row.id), ['2']);
  assert.deepEqual(raidLogTypes(data).map(type => [type.token, type.label, type.icon]),
    [['mod/sky_raid', 'Sky raid from a mod', null], ['mod/sea_raid', 'Raid name unavailable', null], ['mod/undeclared', 'Undeclared raid', null]]);
});

test('the description and tooltip say the phase, its outcome once ended, and the hours prepared while preparing', () => {
  const raid = syntheticEpisode('1', 10, 'ENG', 20, 30, true, { runs: [phaseRun(10, 19, 2), phaseRun(20, 29, 4), phaseRun(30, 40, 5, 3)], preparation: [[10, 19, 0, 1]] });
  const data = syntheticRaidLog([raid]);
  assert.equal(raidDescription(data, raidAt(raid, 12)!, 'England', 'Germany', 'Kent', 12), 'England targets Germany at Kent with Sky raid from a mod. Phase 2. 2 h prepared.');
  assert.equal(raidDescription(data, raidAt(raid, 35)!, 'England', 'Germany', 'Kent', 35), 'England targets Germany at Kent with Sky raid from a mod. Phase 5 · Outcome 3.');
  const tip = raidTooltip(data, raidAt(raid, 12)!, 'England', 'Germany', 'Kent', 12);
  assert.deepEqual(tip.rows.map(row => [row.label, row.value]).slice(3, 5), [['Phase / outcome', 'Phase 2'], ['Hours prepared', '2']]);
});

test('a target is named by its state, else its province, all at one hour, and says why when it is not', () => {
  const base = syntheticEpisode('1', 10, 'ENG', null, null, false);
  const inState = { ...base, state: 7, province: 70 };
  const atSea = { ...base, id: '2', firstHour: 20, state: null, province: 900 };
  const again = { ...base, id: '3', firstHour: 0, state: 7 };
  assert.deepEqual(targetNameRequest([inState, atSea, again], 500), { places: [{ provinceId: 900, hour: 500 }], states: [{ stateId: 7, hour: 500 }] });
  const names = targetNames({ warnings: [], places: [{ provinceId: 900, hour: 500, name: null, missing: 'Province not recorded' }],
    states: [{ stateId: 7, hour: 500, name: 'Kent', missing: null }] });
  assert.equal(targetText(inState, names), 'Kent');
  assert.equal(targetText(atSea, names), 'Province 900 (Province not recorded)');
  assert.equal(targetText({ ...inState, state: 8 }, names), 'State 8 (Name not read yet)');
  assert.equal(targetText(inState), 'State 7', 'only its id while names are loading');
  assert.equal(targetText({ ...base, state: null, province: null }), 'Target not known');
});

test('every raid’s target is asked, past the 4,096 the log once stopped at', () => {
  const many = Array.from({ length: 5_000 }, (_, at) => ({ ...syntheticEpisode(String(at), at, 'ENG', null, null, false), state: at + 1 }));
  const asked = targetNameRequest(many, 9_000);
  assert.equal(asked.states.length, 5_000);
  assert.ok(asked.states.some(state => state.stateId === 1), 'the oldest raid’s target too');
});

test('exact phase dates omit by, bounded dates keep by, overlapping unread intervals never order by time, and victim labels match provenance', () => {
  const exactRaid = syntheticEpisode('20', 10, 'ENG', 20, 30, true, {
    firstHourExact: true,
    endedExact: true,
    victimSource: 'fixedTarget',
    runs: [
      { ...phaseRun(10, 19, 2), firstExact: true, afterHour: 9 },
      { ...phaseRun(20, 29, 4), firstExact: true, afterHour: 19 },
      { ...phaseRun(30, 40, 5, 3), firstExact: true, afterHour: 29 },
    ],
  });
  const boundedLater = syntheticEpisode('10', 100, 'ENG', null, 120, false, {
    firstHourExact: false,
    endedExact: false,
    victimSource: 'controllerAtFirstEndedRead',
    runs: [
      { ...phaseRun(100, 100, 2), firstExact: false, afterHour: null },
      { ...phaseRun(120, 125, 5, 5), firstExact: false, afterHour: 100 },
    ],
  });
  const boundedEarlier = syntheticEpisode('15', 100, 'ENG', null, 115, false, {
    firstHourExact: false,
    endedExact: false,
    victimSource: 'lastObservedController',
    runs: [
      { ...phaseRun(100, 100, 2), firstExact: false, afterHour: null },
      { ...phaseRun(115, 125, 5, 2), firstExact: false, afterHour: 100 },
    ],
  });
  const data = syntheticRaidLog([exactRaid, boundedLater, boundedEarlier]);
  const rows = raidLogRows(data, 125, [], all, null);
  assert.deepEqual(
    rows.map(row => [row.id, row.hour, row.by]),
    [['10', 120, true], ['15', 115, true], ['20', 30, false]],
  );
  const reversedInput = syntheticRaidLog([exactRaid, boundedEarlier, boundedLater]);
  assert.deepEqual(
    raidLogRows(reversedInput, 125, [], all, null).map(row => row.id),
    ['10', '15', '20'],
    'overlapping unread intervals cluster together and tie-break by id rather than guessed time order',
  );
  const fixedTip = raidTooltip(data, raidAt(exactRaid, 35)!, 'England', 'Germany', 'Kent', 35);
  assert.equal(fixedTip.rows[1].label, 'Fixed target country');
  assert.equal(fixedTip.rows.find(row => row.label === 'First seen')?.label, 'First seen');
  const endedTip = raidTooltip(data, raidAt(boundedLater, 125)!, 'England', 'Germany', 'Kent', 125);
  assert.equal(endedTip.rows[1].label, 'Target controller at first ended read');
  assert.equal(endedTip.rows.find(row => row.label === 'First seen by')?.label, 'First seen by');
});
