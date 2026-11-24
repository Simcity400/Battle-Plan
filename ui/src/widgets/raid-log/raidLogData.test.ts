import { test } from 'node:test';
import assert from 'node:assert/strict';
import { raidDescription, raidLogGroups, raidLogSections, raidLogTypes, raidTooltip, targetNameRequest, targetNames, targetText } from './raidLogData.ts';
import { raidAt } from '../../dashboard/raids.ts';
import { observedEnd, phaseRun, syntheticEpisode, syntheticRaidLog } from '../../dashboard/raids.fixtures.ts';

const raidLogRows = (...args: Parameters<typeof raidLogGroups>) => raidLogGroups(...args).flatMap(group => group.rows);

const CODES = syntheticRaidLog([]);

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

test('the description and tooltip carry victim provenance, hide future endings at earlier hours, and show hours prepared while preparing', () => {
  const raid = syntheticEpisode('1', 10, 'ENG', 20, 30, true, {
    victim: 'GER',
    victimSource: 'controllerAtFirstEndedRead',
    victims: [
      { hour: 10, tag: 'FRA', missing: null, source: 'controllerAtObservedRead' },
      { hour: 30, tag: 'GER', missing: null, source: 'controllerAtFirstEndedRead' },
    ],
    runs: [phaseRun(10, 19, 2, 0, true, 9, 'ENG'), phaseRun(20, 29, 4, 0, true, 19, 'ENG'), phaseRun(30, 40, 5, 3, true, 29, 'ENG')],
    preparation: [[10, 19, 0, 1]],
  });
  const data = syntheticRaidLog([raid]);
  const at12 = raidAt(CODES, raid, 12)!;
  assert.equal(at12.ended, false);
  assert.equal(at12.victim.tag, 'FRA');
  assert.equal(at12.victim.source, 'controllerAtObservedRead');
  assert.equal(raidDescription(data, at12, 'England', 'France', 'Kent', 12), 'England targets Kent (target controller: France) with Sky raid from a mod. Phase 2. 2 h prepared.');
  const tip12 = raidTooltip(data, at12, 'England', 'France', 'Kent', 12);
  assert.equal(tip12.rows.find(row => row.label.startsWith('Ended')), undefined, 'preparing tooltip must not leak future end');
  assert.deepEqual(tip12.rows[1], { label: 'Target controller', value: 'France' });
  assert.deepEqual(tip12.rows.map(row => [row.label, row.value]).slice(3, 5), [['Phase / outcome', 'Phase 2'], ['Hours prepared', '2']]);

  const at35 = raidAt(CODES, raid, 35)!;
  assert.equal(at35.ended, true);
  assert.equal(at35.victim.tag, 'GER');
  assert.equal(at35.victim.source, 'controllerAtFirstEndedRead');
  assert.equal(raidDescription(data, at35, 'England', 'Germany', 'Kent', 35), 'England targets Kent (controller at first ended read: Germany) with Sky raid from a mod. Phase 5 · Outcome 3.');
  const tip35 = raidTooltip(data, at35, 'England', 'Germany', 'Kent', 35);
  assert.deepEqual(tip35.rows[1], { label: 'Target controller at first ended read', value: 'Germany' });
  assert.ok(tip35.rows.some(row => row.label === 'Ended (observed)'));

  const nativeEnded = syntheticEpisode('2', 10, 'ENG', 20, 30, true, {
    ends: [{ hour: 30, endedHour: 30, exact: true, source: 'nativeEndDate', raider: 'ENG' }],
    victim: 'POL',
    victimSource: 'completionCountry',
    victims: [
      { hour: 10, tag: 'FRA', missing: null, source: 'controllerAtObservedRead' },
      { hour: 30, tag: 'POL', missing: null, source: 'completionCountry' },
    ],
  });
  const nativeData = syntheticRaidLog([nativeEnded]);
  const nativeTip = raidTooltip(nativeData, raidAt(CODES, nativeEnded, 35)!, 'England', 'Poland', 'Kent', 35);
  assert.deepEqual(nativeTip.rows[1], { label: 'Country at completion', value: 'Poland' });
  assert.ok(nativeTip.rows.some(row => row.label === 'Ended (recorded)'));
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

test('interval ordering preserves precedence between disjoint intervals and puts mutually overlapping changes in a visible interval group', () => {
  const r1 = syntheticEpisode('1', 10, 'ENG', null, 15, false, {
    firstHourExact: false,
    ends: [observedEnd(15, 'ENG', false)],
    runs: [
      { ...phaseRun(10, 10, 2), firstExact: false, afterHour: null },
      { ...phaseRun(15, 35, 5, 1), firstExact: false, afterHour: 10 },
    ],
  });
  const r2 = syntheticEpisode('2', 1, 'ENG', null, 30, false, {
    firstHourExact: false,
    ends: [observedEnd(30, 'ENG', false)],
    runs: [
      { ...phaseRun(1, 1, 2), firstExact: false, afterHour: null },
      { ...phaseRun(30, 35, 5, 2), firstExact: false, afterHour: 1 },
    ],
  });
  const r3 = syntheticEpisode('3', 25, 'ENG', null, 30, false, {
    firstHourExact: false,
    ends: [observedEnd(30, 'ENG', false)],
    runs: [
      { ...phaseRun(25, 25, 2), firstExact: false, afterHour: null },
      { ...phaseRun(30, 35, 5, 3), firstExact: false, afterHour: 25 },
    ],
  });
  const data = syntheticRaidLog([r1, r2, r3]);
  const groups = raidLogGroups(data, 35, [], all, null);
  const orderedIds = groups.flatMap(g => g.rows.map(row => row.id));
  assert.ok(orderedIds.indexOf('3') < orderedIds.indexOf('1'), `provably newer 3 must precede 1, got ${orderedIds}`);
  assert.equal(groups[0].unordered, true);
  assert.deepEqual(groups[0].rows.map(row => row.id), ['2', '3']);
  assert.ok(groups[0].label?.startsWith('Unordered · between '), `expected shared interval label, got ${groups[0].label}`);
  assert.equal(groups[1].unordered, false);
  assert.deepEqual(groups[1].rows.map(row => row.id), ['1']);
  const sections = raidLogSections(groups, true)!;
  assert.equal(sections.length, 2);
  assert.equal(sections[0].months, false);
  assert.ok(sections[0].label.startsWith('Unordered · between '));
  assert.deepEqual(sections[0].rows?.map(row => row.id), ['2', '3']);
  assert.deepEqual(sections[1].rows?.map(row => row.id), ['1']);

  const fixed = syntheticEpisode('20', 10, 'ENG', 20, 30, true, {
    victimSource: 'fixedTarget',
    victims: [{ hour: 10, tag: 'GER', missing: null, source: 'fixedTarget' }],
  });
  const captor = syntheticEpisode('21', 10, 'ENG', 20, 30, true, {
    victimSource: 'captorAtFirstEndedRead',
    victims: [
      { hour: 10, tag: 'FRA', missing: null, source: 'captorAtObservedRead' },
      { hour: 30, tag: 'GER', missing: null, source: 'captorAtFirstEndedRead' },
    ],
  });
  const provData = syntheticRaidLog([fixed, captor]);
  assert.equal(raidDescription(provData, raidAt(CODES, fixed, 35)!, 'England', 'Germany', 'Kent', 35), 'England targets Germany at Kent with Sky raid from a mod. Phase 5 · Outcome 3.');
  assert.equal(raidTooltip(provData, raidAt(CODES, fixed, 35)!, 'England', 'Germany', 'Kent', 35).rows[1].label, 'Fixed target country');
  assert.equal(raidDescription(provData, raidAt(CODES, captor, 15)!, 'England', 'France', 'Kent', 15), 'England targets Kent (captor: France) with Sky raid from a mod. Phase 2. 5 h prepared.');
  assert.equal(raidTooltip(provData, raidAt(CODES, captor, 35)!, 'England', 'Germany', 'Kent', 35).rows[1].label, 'Leader captor at first ended read');
  assert.equal(raidLogSections(raidLogGroups(provData, 35, [], all, null), true), null, 'exact rows produce no unordered sections');
});
