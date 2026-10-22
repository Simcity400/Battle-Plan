import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { UnitLeadersData } from './generalsData';
import { battleHistoryComplete, battleTotals, attributeNames, careerEmptyEntry, recordedSince, careerCountryFacts, careerLanes, leaderAt, leadersAt, leaderName, leaderTooltip, pinnedCareer, leaderRequestSelection, validateUnitLeadersReply } from './generalsData.ts';
import { leaderChoices } from '../general-career/choicesData.ts';
import { createWidget, resolveWindow } from '../../dashboard/model.ts';
import type { BoundTrack } from '../../dashboard/model';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState.ts';
import { playbackDomain } from '../../charts/view.ts';
import { engineHour } from '../../dashboard/calendar.ts';
import { DEFAULT_SORT, OPTIONS } from './definition.ts';
import { OPTIONS as CAREER_OPTIONS } from '../general-career/definition.ts';
import { resolveOptions } from '../../dashboard/options.ts';
import { tableSortOf } from '../../charts/table/options.ts';
import '../definitions.ts';

function decodedStarts(deltas: readonly number[]) { let hour = 0; return deltas.map(delta => hour += delta); }

function fixture(): UnitLeadersData {
  return { branchId: '1', firstHour: 100,
    people: [{ characterId: 9, token: 'mod_character', name: 'A modded commander', portraitHour: 100, portraitReference: '21474841193' }],
    roles: [{ owner: 'country with spaces', category: 1, rank: 'Mod rank', kind: 1, maximumSkill: 12, attributeNames: ['Attack', 'Defence', 'Planning', 'Logistics'] }],
    traits: [{ token: 'extra_trait', name: 'Mod trait', icon: null, missing: null }],
    leaders: [{ reference: '21474841193', runs: [{ firstHour: 101, lastHour: 106, value: { person: 0, role: 0, skill: 3, components: [1, 2, 3, 4], progress: null, traits: [0] } }],
      traitGains: [{ hour: 101, token: 0, date: 'gained' }], commands: [], battleStarts: [], battleStartReadHours: [], unknownBattleStarts: [], partialFrom: null }],
    observed: [{ firstHour: 100, lastHour: 106, leaders: true, landBattles: true, navalBattles: false, landCommands: true, navalCommands: false }],
    resultsMissing: 'Battle results not recorded', navyCommandsMissing: 'Navy commands not recorded', warnings: [] };
}
function wire(data: UnitLeadersData) {
  return { ...data, leaders: data.leaders.map(l => ({ ...l, runs: l.runs.map(r => [r.firstHour, r.lastHour, r.value.person, r.value.role, r.value.skill, r.value.components, r.value.traits, r.value.progress]),
    traitGains: l.traitGains.map(g => [g.hour, g.token, g.date === 'heldBefore' ? 0 : g.date === 'gained' ? 1 : 2]) })) };
}
/** The first leader as a leaderboard sends it: visible hours instead of battle records. */
function asLeaderboard(data: UnitLeadersData, hours: Pick<UnitLeadersData['leaders'][number], 'foughtHours' | 'wonHours' | 'lostHours'>) {
  for (const key of ['battleStarts', 'battleStartReadHours', 'unknownBattleStarts', 'battleWon', 'battleResultHours', 'battleResultReadHours'] as const) delete data.leaders[0][key];
  Object.assign(data.leaders[0], hours);
}

test('a leaderboard read at one hour shows that hour as the whole career did', () => {
  const career = fixture(); const leader = career.leaders[0];
  leader.runs[0].lastHour = 102;
  leader.runs.push({ ...leader.runs[0], firstHour: 103, lastHour: 104, value: { ...leader.runs[0].value, skill: 4 } }, { ...leader.runs[0], firstHour: 105, lastHour: 106, value: { ...leader.runs[0].value, role: 1 } });
  career.roles.push({ ...career.roles[0], owner: 'transfer country' });
  leader.traitGains.push({ hour: 104, token: 0, date: 'gained' });
  asLeaderboard(career, { foughtHours: [101, 1, 2], wonHours: [102], lostHours: [104] });
  const full = validateUnitLeadersReply(wire(career));
  const at = fixture(); const projected = at.leaders[0];
  projected.runs = [{ ...full.leaders[0].runs[1] }];
  projected.traitGains = [{ hour: 101, token: 0, date: 'gained' }, { hour: 104, token: 0, date: 'gained' }];
  asLeaderboard(at, {});
  projected.atHour = { hour: 104, firstSeenHour: 101, fought: 3, won: 1, lost: 1 };
  const shown = validateUnitLeadersReply(wire(at));
  const view = (data: UnitLeadersData, hour = 104) => {
    const { career: row, traitCatalogue: _catalogue, ...rest } = leaderAt(data, data.leaders[0], hour)!;
    return { ...rest, gains: row.traitGains.filter(gain => gain.hour <= hour), since: recordedSince(row, data.firstHour) };
  };
  assert.deepEqual(view(shown), view(full));
  assert.equal(leaderAt(shown, shown.leaders[0], 103), null, 'it answers no other hour');
  assert.deepEqual(battleTotals(shown.leaders[0], 103), { fought: null, won: null, lost: null, winRate: null, unresolved: null });
  assert.deepEqual(battleTotals(shown.leaders[0], 104), battleTotals(full.leaders[0], 104));
  // After a transfer before the hour shown, and without recorded results.
  const transferred = fixture(); transferred.roles.push({ ...transferred.roles[0], owner: 'transfer country' });
  transferred.leaders[0].runs = [{ ...full.leaders[0].runs[2] }];
  transferred.leaders[0].traitGains = [...projected.traitGains];
  asLeaderboard(transferred, {});
  transferred.leaders[0].atHour = { hour: 106, firstSeenHour: 101, fought: 3, won: null, lost: null };
  const unrecorded = { ...career, leaders: career.leaders.map(item => ({ ...item, wonHours: undefined, lostHours: undefined })) };
  const after = validateUnitLeadersReply(wire(transferred));
  const before = validateUnitLeadersReply(wire(unrecorded));
  assert.deepEqual(view(after, 106), view(before, 106));
  assert.equal(view(after, 106).value.owner, 'transfer country');
  assert.equal(view(after, 106).won, null, 'results not recorded stay unknown, never zero');
  assert.throws(() => validateUnitLeadersReply(wire({ ...at, leaders: [{ ...projected, foughtHours: [101] }] })), /malformed/, 'never both counts and battle hours');
  projected.atHour = { ...projected.atHour, won: 3 };
  assert.throws(() => validateUnitLeadersReply(wire(at)), /malformed/, 'results never outnumber battles');
  projected.atHour = { ...projected.atHour, won: 1, hour: 107 };
  assert.throws(() => validateUnitLeadersReply(wire(at)), /malformed/, 'its run covers its hour');
});

test('battle delta spans the full signed32 hour range exactly', () => {
  const data = fixture(); const leader = data.leaders[0];
  data.firstHour = -2147483648;
  leader.runs = [{ ...leader.runs[0], firstHour: -2147483648, lastHour: 2147483647 }];
  data.observed = [{ ...data.observed[0], firstHour: -2147483648, lastHour: 2147483647 }];
  leader.battleStarts = [-2147483648, 4294967295]; leader.battleStartReadHours = decodedStarts(leader.battleStarts);
  assert.equal(battleTotals(validateUnitLeadersReply(wire(data)).leaders[0], 2147483647).fought, 2);
});

test('results preceding reconstructed participation are refused and same-hour results stay valid', () => {
  const data = fixture(); const leader = data.leaders[0];
  leader.runs[0].firstHour = 100; leader.runs[0].lastHour = 200; data.observed[0].lastHour = 200;
  leader.battleStarts = [100]; leader.battleStartReadHours = decodedStarts(leader.battleStarts); leader.battleWon = [true]; leader.battleResultHours = [50]; leader.battleResultReadHours = [...leader.battleResultHours!];
  assert.throws(() => validateUnitLeadersReply(wire(data)), /malformed/);
  leader.battleResultHours = [100]; leader.battleResultReadHours = [...leader.battleResultHours!];
  let accepted = validateUnitLeadersReply(wire(data)).leaders[0];
  assert.deepEqual(battleTotals(accepted, 75), { fought: 0, won: 0, lost: 0, unresolved: 0, winRate: null });
  assert.deepEqual(battleTotals(accepted, 100), { fought: 1, won: 1, lost: 0, unresolved: 0, winRate: 100 });
  leader.battleStarts = [100, 100]; leader.battleStartReadHours = decodedStarts(leader.battleStarts); leader.battleWon = [true, false]; leader.battleResultHours = [100, 199]; leader.battleResultReadHours = [...leader.battleResultHours!];
  assert.throws(() => validateUnitLeadersReply(wire(data)), /malformed/);
  leader.battleResultHours = [100, 200]; leader.battleResultReadHours = [...leader.battleResultHours!]; accepted = validateUnitLeadersReply(wire(data)).leaders[0];
  assert.deepEqual(battleTotals(accepted, 199), { fought: 1, won: 1, lost: 0, unresolved: 0, winRate: 100 });
  assert.deepEqual(battleTotals(accepted, 200), { fought: 2, won: 1, lost: 1, unresolved: 0, winRate: 50 });
});

test('known decided outcomes survive unresolved battles and zero denominator has no rate', () => {
  const leader = fixture().leaders[0];
  leader.battleStarts = [101, 1, 1]; leader.battleStartReadHours = decodedStarts(leader.battleStarts);
  leader.battleWon = [true, false, null]; leader.battleResultHours = [102, 104, null]; leader.battleResultReadHours = [...leader.battleResultHours!];
  assert.deepEqual(battleTotals(leader, 103), { fought: 3, won: 1, lost: 0, unresolved: 2, winRate: 100 });
  assert.deepEqual(battleTotals(leader, 104), { fought: 3, won: 1, lost: 1, unresolved: 1, winRate: 50 });
  assert.deepEqual(battleTotals(leader, 101), { fought: 1, won: 0, lost: 0, unresolved: 1, winRate: null });
});

test('nine-year outcome histories build once and 5000 playback queries never reread every outcome', () => {
  const leader = fixture().leaders[0];
  leader.battleStarts = Array.from({ length: 80000 }, (_, index) => index ? 1 : 100); leader.battleStartReadHours = decodedStarts(leader.battleStarts);
  let reads = 0; let querying = false;
  leader.battleWon = new Proxy(Array.from({ length: 80000 }, (_, index) => index % 3 === 0 ? null : index % 2 === 0), {
    get(target, property, receiver) { if (typeof property === 'string' && /^[0-9]+$/.test(property)) { assert.equal(querying, false, 'Playback reread a prior outcome'); reads++; } return Reflect.get(target, property, receiver); },
  });
  leader.battleResultHours = Array.from({ length: 80000 }, (_, index) => index % 3 === 0 ? null : 101 + index); leader.battleResultReadHours = [...leader.battleResultHours!];
  assert.deepEqual(battleTotals(leader, 99999), { fought: 80000, won: 26666, lost: 26667, unresolved: 26667, winRate: 26666 * 100 / 53333 });
  reads = 0; querying = true;
  for (let index = 0; index < 5000; index++) battleTotals(leader, 100 + index * 16);
  assert.equal(reads, 0); querying = false;
  for (const count of [1500, 5000]) {
    const data = fixture(); const base = data.leaders[0];
    data.roles = Array.from({ length: count }, (_, index) => ({ ...data.roles[0], owner: `country ${index}` }));
    data.leaders = Array.from({ length: count }, (_, index) => ({ ...base, reference: String(index + 1),
      runs: [{ ...base.runs[0], firstHour: 100, lastHour: 80101, value: { ...base.runs[0].value, role: index } }],
      battleStarts: [100, 40000, 40000], battleStartReadHours: [100, 40100, 80100], battleResultHours: [100, 40101, null], battleResultReadHours: [100, 40101, null],
      battleWon: new Proxy([true, false, null], {
        get(target, property, receiver) { if (typeof property === 'string' && /^[0-9]+$/.test(property)) reads++; return Reflect.get(target, property, receiver); },
      }),
    }));
    for (const career of data.leaders) battleTotals(career, 80101);
    reads = 0;
    const rows = leadersAt(data, 80101, null, 'all');
    assert.equal(rows.length, count);
    assert.ok(rows.every(row => row.won === 1 && row.lost === 1 && row.unresolved === 1));
    assert.equal(reads, 0);
  }
});

test('large smart groups retain every requested country and still filter by current membership', () => {
  const tags = Array.from({ length: 5000 }, (_, i) => `mod country ${i}`);
  tags.push('country with spaces');
  assert.deepEqual(leaderRequestSelection(tags), { kind: 'countries', tags });
  assert.deepEqual(leaderRequestSelection(null), { kind: 'all' });
  assert.equal(leaderRequestSelection(undefined), undefined);
  const data = fixture();
  assert.equal(leadersAt(data, 101, new Set(tags), 'all').length, 1);
  assert.equal(leadersAt(data, 101, new Set(tags.slice(0, 2)), 'all').length, 0);
});

test('compact runs and interned traits validate and expand once; malformed parallel arrays are refused', () => {
  const data = fixture(); assert.deepEqual(validateUnitLeadersReply(wire(data)), data);
  for (const mutate of [
    (d: UnitLeadersData) => { d.leaders[0].reference = '18446744073709551616'; },
    (d: UnitLeadersData) => { d.leaders[0].runs[0].value.person = 7; },
    (d: UnitLeadersData) => { d.leaders[0].runs[0].value.role = 7; },
    (d: UnitLeadersData) => { d.leaders[0].runs[0].value.traits = [7]; },
    (d: UnitLeadersData) => { d.leaders[0].traitGains[0].token = 7; },
    (d: UnitLeadersData) => { d.roles[0].maximumSkill = -1; },
    (d: UnitLeadersData) => { d.roles[0].owner = ''; },
    (d: UnitLeadersData) => { d.roles[0].owner = 'country\0suffix'; },
    (d: UnitLeadersData) => { d.leaders[0].runs[0].lastHour = 99; },
    (d: UnitLeadersData) => { d.leaders.push(structuredClone(d.leaders[0])); },
    (d: UnitLeadersData) => { d.leaders[0].runs.push(structuredClone(d.leaders[0].runs[0])); },
    (d: UnitLeadersData) => { d.leaders[0].battleStarts = [101, -1]; d.leaders[0].battleStartReadHours = decodedStarts(d.leaders[0].battleStarts); },
    (d: UnitLeadersData) => { d.leaders[0].unknownBattleStarts = [0]; },
    (d: UnitLeadersData) => { d.leaders[0].battleWon = [true]; },
    (d: UnitLeadersData) => { d.leaders[0].foughtHours = [101]; },
    (d: UnitLeadersData) => asLeaderboard(d, { foughtHours: [101, -1] }),
    (d: UnitLeadersData) => asLeaderboard(d, { foughtHours: [99] }),
    (d: UnitLeadersData) => asLeaderboard(d, { foughtHours: [101], wonHours: [101] }),
    (d: UnitLeadersData) => asLeaderboard(d, { foughtHours: [102], wonHours: [101], lostHours: [] }),
  ]) { const bad = fixture(); mutate(bad); assert.throws(() => validateUnitLeadersReply(wire(bad)), /malformed/); }
  const bad = wire(data); bad.leaders[0].runs[0].push(4); assert.throws(() => validateUnitLeadersReply(bad), /malformed/);
});

test('new arrivals are exact after complete reads; unrelated unresolved owners never change their counts', () => {
  const data = fixture(); const leader = data.leaders[0];
  assert.equal(leaderAt(data, leader, 101)?.fought, 0);
  data.observed[0].leaders = false;
  assert.equal(leaderAt(data, leader, 101)?.foughtPartial, false);
  assert.equal(leadersAt(data, 101, new Set(['country with spaces']), 'land').length, 1);
  assert.equal(leadersAt(data, 101, null, 'navy').length, 0);
  leader.partialFrom = 104;
  leader.runs[0].lastHour = 102;
  leader.runs.push({ ...leader.runs[0], firstHour: 104, lastHour: 106 });
  assert.equal(leaderAt(data, leader, 103), null);
  assert.equal(leaderAt(data, leader, 104)?.foughtPartial, true);
  assert.equal(leaderAt(data, leader, 104)?.fought, 0);
  leader.battleStarts = [101, 3]; leader.battleStartReadHours = decodedStarts(leader.battleStarts);
  assert.equal(leaderAt(data, leader, 106)?.fought, 2);
  data.observed[0].landBattles = false;
  assert.equal(battleHistoryComplete(data, 1, 106), false);
});

test('leaderboard visible hours give the career records’ rows, counts and tooltips at every hour', () => {
  const career = fixture(); const leader = career.leaders[0];
  leader.runs[0].lastHour = 130; career.observed[0].lastHour = 130;
  // An unknown start, shared hours, delayed reads, an undecided battle and results read before or after their participation.
  Object.assign(leader, { battleStarts: [98, 4, 0, 3, 5, 0], battleStartReadHours: [100, 102, 106, 105, 110, 111], unknownBattleStarts: [0],
    battleWon: [true, false, true, true, false, null], battleResultHours: [101, 103, 103, 105, 120, null], battleResultReadHours: [104, 103, 104, 108, 125, null] });
  const board = structuredClone(career);
  asLeaderboard(board, { foughtHours: [100, 2, 3, 1, 4, 1], wonHours: [104, 2, 2], lostHours: [103, 22] });
  for (const results of [true, false]) {
    if (!results) for (const data of [career, board]) for (const key of ['battleWon', 'battleResultHours', 'battleResultReadHours', 'wonHours', 'lostHours'] as const) delete data.leaders[0][key];
    const [records, counts] = [career, board].map(data => validateUnitLeadersReply(wire(data)));
    for (let hour = 99; hour <= 131; hour++) {
      const [expected, actual] = [records, counts].map(reply => leadersAt(reply, hour, null, 'all')
        .map(row => ({ ...row, career: null, tooltip: leaderTooltip(row, { name: 'Country' }) })));
      assert.deepEqual(actual, expected, `hour ${hour}`);
    }
    assert.deepEqual(battleTotals(counts.leaders[0], 125), results ? { fought: 6, won: 3, lost: 2, unresolved: 1, winRate: 60 }
      : { fought: 6, won: null, lost: null, unresolved: 6, winRate: null });
  }
});

test('dated joined results exclude unresolved battles from the win rate', () => {
  const leader = fixture().leaders[0];
  leader.battleStarts = [101, 1]; leader.battleStartReadHours = decodedStarts(leader.battleStarts); leader.battleWon = [true, false]; leader.battleResultHours = [102, 103]; leader.battleResultReadHours = [...leader.battleResultHours!];
  assert.equal(battleTotals(leader, 102).winRate, 100);
  assert.equal(battleTotals(leader, 102).unresolved, 1);
  assert.deepEqual(battleTotals(leader, 103), { fought: 2, won: 1, lost: 1, winRate: 50, unresolved: 0 });
});

test('tooltip follows the paired attributes, battles, traits and career hint of leaderTip', () => {
  const data = fixture(); const row = leaderAt(data, data.leaders[0], 101)!;
  const tip = leaderTooltip(row, { name: 'Mod country' });
  assert.deepEqual(tip.rows.slice(1, 4).map(r => r.label), ['Attack · Defence', 'Planning · Logistics', 'Battles won / fought']);
  assert.equal(tip.rows.at(-1)?.label, 'Mod trait'); assert.equal(tip.note, undefined);
  assert.equal(tip.description, undefined);
  const emptyTraits = { ...row, value: { ...row.value, traits: [] } };
  assert.deepEqual(leaderTooltip(emptyTraits, { name: 'Country' }).rows.at(-1), { label: 'Traits', value: 'None recorded', divider: true });
  assert.equal(leaderTooltip({ ...row, value: { ...row.value, traits: null } }, { name: 'Country' }).rows.at(-1)?.label, 'Traits not recorded');
  data.roles[0].kind = 2;
  data.roles[0].attributeNames = ['Naval attack', 'Naval defence', 'Maneuvering', 'Coordination'];
  assert.equal(leaderTooltip(leaderAt(data, data.leaders[0], 101)!, { name: 'Country' }).rows[2].label, 'Maneuvering · Coordination');
});

test('trait dates retain honest held bounds and ordinary career spans omit supplying dates', () => {
  const data = fixture(); const leader = data.leaders[0];
  for (const [date, label] of [['heldBefore', 'Held before'], ['firstObserved', 'Held by'], ['gained', 'Gained']] as const) {
    leader.traitGains[0].date = date;
    const tip = leaderTooltip(leaderAt(data, leader, 101)!, { name: 'Country' });
    assert.ok(tip.rows.at(-1)?.value?.startsWith(label));
    assert.ok(!tip.rows.some(row => row.label === 'Read'));
  }
  leader.runs[0].lastHour = 102;
  leader.runs.push({ ...leader.runs[0], firstHour: 103, lastHour: 106 });
  const ordinary = careerLanes(data, leader, 101, 106)[0].spans[1];
  assert.equal(ordinary.label, 'Skill 3');
});

test('the picker uses the last owner date and sorts names', () => {
  const data = fixture(); const leader = data.leaders[0];
  leader.runs[0].lastHour = 102;
  leader.runs.push({ ...leader.runs[0], firstHour: 103, lastHour: 104 }, { ...leader.runs[0], firstHour: 105, lastHour: 106, value: { ...leader.runs[0].value, role: 1 } });
  data.roles.push({ ...data.roles[0], owner: 'transfer country' });
  assert.equal(leadersAt(data, 106, new Set(['country with spaces']), 'all').length, 0);
  data.people.push({ ...data.people[0], name: 'Zed' });
  data.leaders.unshift({ ...leader, reference: '8', runs: [{ ...leader.runs[0], value: { ...leader.runs[0].value, person: 1 } }] });
  const called: unknown[] = [];
  const choices = leaderChoices(data, (hour, tag) => { called.push([hour, tag]); return 'Named country'; });
  assert.match(choices[0].label, /^A modded commander · Named country/);
  assert.ok(called.some(c => JSON.stringify(c) === JSON.stringify([106, 'transfer country'])));
});

test('a departing row retains its trait identities when a new selection remaps the catalogue', () => {
  const before = fixture(); const row = leaderAt(before, before.leaders[0], 101)!;
  const after = fixture(); after.traits[0].name = 'Another country’s trait';
  assert.equal(leaderTooltip(row, { name: 'Country' }).rows.at(-1)?.label, 'Mod trait');
});

test('a continuing command never displays a later name before its first recorded read', () => {
  const data = fixture(); const leader = data.leaders[0];
  leader.commands = [{ formation: '70', kind: 'army', firstHour: 101, lastHour: 106,
    name: 'Recorded army name', nameReadHour: 103, nameUnknown: [{ firstHour: 101, lastHour: 102 }] }];
  const early = careerLanes(data, leader, 101, 102)[2].spans;
  const later = careerLanes(data, leader, 101, 104)[2].spans;
  assert.equal(early.length, 1); assert.equal(later.length, 1);
  assert.equal(early[0].id, later[0].id);
  assert.equal(early[0].label, 'Formation 70');
  assert.equal(later[0].label, 'Recorded army name');
});

test('career identity reads include exact header and hover hours within one owner run', () => {
  const data = fixture(); const leader = data.leaders[0];
  assert.deepEqual(careerCountryFacts(data, leader, 106, 104), [
    { hour: 101, tag: 'country with spaces' }, { hour: 106, tag: 'country with spaces' }, { hour: 104, tag: 'country with spaces' },
  ]);
  assert.deepEqual(careerCountryFacts(data, leader, 101, 100), [{ hour: 101, tag: 'country with spaces' }]);
  leader.runs[0].lastHour = 102;
  data.roles.push({ ...data.roles[0], owner: 'Transferred country' });
  leader.runs.push({ ...leader.runs[0], firstHour: 104, lastHour: 106, value: { ...leader.runs[0].value, role: 1 } });
  assert.deepEqual(careerCountryFacts(data, leader, 106, 103), [
    { hour: 101, tag: 'country with spaces' }, { hour: 104, tag: 'Transferred country' }, { hour: 106, tag: 'Transferred country' },
  ]);
});

test('monthly neutral fought bars preserve gaps and are prepared for won and lost bars', () => {
  const data = fixture(); const leader = data.leaders[0];
  const jan = engineHour({ year: 1936, month: 1, day: 1, hour: 1 })!;
  const feb = engineHour({ year: 1936, month: 2, day: 1, hour: 1 })!;
  leader.runs = [{ ...leader.runs[0], firstHour: jan, lastHour: jan + 1 }, { ...leader.runs[0], firstHour: jan + 4, lastHour: feb + 1 }];
  data.firstHour = jan; data.observed = [{ firstHour: jan, lastHour: jan + 1, leaders: true, landBattles: true, navalBattles: false, landCommands: true, navalCommands: false },
    { firstHour: jan + 4, lastHour: feb + 1, leaders: true, landBattles: true, navalBattles: false, landCommands: true, navalCommands: false }];
  leader.battleStarts = [jan, feb - jan]; leader.battleStartReadHours = decodedStarts(leader.battleStarts); leader.unknownBattleStarts = [0];
  const spans = careerLanes(data, leader, jan, feb + 1)[3].spans;
  assert.equal(spans.length, 3); assert.equal(spans[0].lastHour, jan + 1); assert.equal(spans[1].firstHour, jan + 4);
  assert.match(spans[0].label, /Fought 1.*1 unknown combat start.*Battle results are not in this recording/);
  assert.equal(spans[0].positiveCount, undefined);
  leader.battleWon = [true, false]; leader.battleResultHours = [jan + 1, feb + 1]; leader.battleResultReadHours = [...leader.battleResultHours!];
  const result = careerLanes(data, leader, jan, feb + 1)[3].spans;
  assert.equal(result[0].positiveCount, 1); assert.equal(result.at(-1)?.negativeCount, 1);
});

test('skill dots, trait markers, defaults and pinning retain the shared kit behavior', () => {
  const data = fixture(); const leader = data.leaders[0]; leader.runs[0].lastHour = 102;
  leader.runs.push({ ...leader.runs[0], firstHour: 103, lastHour: 104, value: { ...leader.runs[0].value, skill: 4 } },
    { ...leader.runs[0], firstHour: 106, lastHour: 107, value: { ...leader.runs[0].value, skill: 5 } });
  const lanes = careerLanes(data, leader, 101, 107);
  assert.equal(lanes[0].spans[1].dot, true); assert.equal(lanes[0].spans[2].dot, false);
  assert.match(lanes[0].spans[2].label, /First seen/);
  assert.deepEqual(lanes[1].spans[0].icons, [undefined]);
  const source = createWidget('generals', 'main'); assert.deepEqual(source.countries, { kind: 'all' });
  const options = resolveOptions(OPTIONS, source.options); assert.deepEqual(tableSortOf(options.tableSort, DEFAULT_SORT), DEFAULT_SORT);
  const track = { id: 'main', branch: { firstHour: 100 } } as BoundTrack;
  const pinned = pinnedCareer(source, leader.reference, track, 107, true); assert.equal(pinned.kind, 'general-career');
  assert.deepEqual(pinned.options, { leader: leader.reference, growWithPlayback: false }); assert.deepEqual(pinned.time, { mode: 'follow' });
});

test('Pin career retains the selected day while Follow keeps the shared playback time', () => {
  const source = { ...createWidget('generals', 'main'), campaign: 'tracks' as const,
    countries: { kind: 'tags' as const, tags: ['country with spaces'] }, contentScale: 150,
    time: { mode: 'follow' as const }, options: { growWithPlayback: true } };
  const track = { id: 'main', axisOffset: 0, branch: { firstHour: 100, lastHour: 500 } } as BoundTrack;
  const opened = reduceDrill(CLOSED, { type: 'open', breakdown: '21474841193', hour: 300, playhead: 300, playing: true });
  const picked = reduceDrill(opened, { type: 'pick', hour: 120.9 });
  const pinned = pinnedCareer(source, picked.breakdown!, track, drillHour(picked, 300)!, picked.follow);
  assert.deepEqual(pinned.time, { mode: 'pin', windows: { main: { firstOffset: 0, lastOffset: 20 } } });
  assert.deepEqual(resolveWindow(pinned, track, [track], 300), { firstHour: 100, lastHour: 120 });
  assert.equal(pinned.campaign, source.campaign); assert.deepEqual(pinned.trackIds, source.trackIds);
  assert.deepEqual(pinned.countries, source.countries); assert.equal(pinned.contentScale, 150);
  assert.deepEqual(pinned.options, { leader: '21474841193', growWithPlayback: true });
  const following = reduceDrill(picked, { type: 'follow' });
  const live = pinnedCareer(source, following.breakdown!, track, drillHour(following, 301)!, following.follow);
  assert.deepEqual(live.time, { mode: 'follow' });
  assert.equal(resolveWindow(live, track, [track], 301)?.lastHour, 301);
});

test('career and drill default to a full axis and share the grow with playback choice', () => {
  for (const fields of [OPTIONS, CAREER_OPTIONS]) {
    const full = resolveOptions(fields, {});
    assert.equal(full.growWithPlayback, false);
    assert.deepEqual(playbackDomain(full.growWithPlayback ? 'draw-in' : 'cursor', [100, 501], 201, 0, 1), [100, 501]);
    const grow = resolveOptions(fields, { growWithPlayback: true });
    assert.equal(grow.growWithPlayback, true);
    assert.deepEqual(playbackDomain(grow.growWithPlayback ? 'draw-in' : 'cursor', [100, 501], 201, 0, 1), [100, 201]);
  }
});

test('a known empty leader name survives leaderboard, career tooltip and picker fallback', () => {
  const data = fixture(); const person = data.people[0]; person.name = '';
  const row = leaderAt(data, data.leaders[0], 101)!;
  assert.equal(leaderName(row), '');
  assert.equal(leaderTooltip(row, { name: 'Country' }).title, '');
  assert.equal(leaderChoices(data, () => 'Country')[0].label, ' · Country');
  person.name = null; assert.equal(leaderName(row), 'Character name unavailable');
  person.token = ''; assert.equal(leaderName(row), 'Character name unavailable');
  person.token = null; assert.equal(leaderName(row), 'Character name unavailable');
});


test('recorded battle totals remain visible for initial holdings and missing coverage', () => {
  const data = fixture(); const leader = data.leaders[0];
  leader.partialFrom = data.firstHour;
  data.observed[0].landBattles = false;
  assert.equal(leaderAt(data, leader, 101)?.fought, 0);
  const initialTip = leaderTooltip(leaderAt(data, leader, 101)!, { name: 'Country' });
  assert.equal(initialTip.rows[3].value, 'Result not recorded / 0');
  assert.equal(initialTip.description, undefined);
  leader.battleStarts = [101]; leader.battleStartReadHours = decodedStarts(leader.battleStarts); leader.battleWon = [true]; leader.battleResultHours = [102]; leader.battleResultReadHours = [...leader.battleResultHours!];
  assert.equal(leaderAt(data, leader, 102)?.won, 1);
  assert.equal(leaderAt(data, leader, 102)?.winRate, 100);
  assert.match(recordedSince(leader, data.firstHour), /^Battles since/);
  assert.equal(careerEmptyEntry(data, leader, 'battles', 102), 'Battle data unavailable.');
  data.observed[0].landBattles = true;
  assert.equal(careerEmptyEntry(data, leader, 'battles', 102), 'No recorded participation at this hour');
  data.observed[0].landCommands = false;
  assert.equal(careerEmptyEntry(data, leader, 'commands', 102), 'Command data unavailable.');
  data.observed[0].landCommands = true;
  assert.match(careerEmptyEntry(data, leader, 'commands', 102), /No command/);
  data.roles[0].attributeNames = ['Mod attack', 'Mod defence', 'Mod planning', null];
  assert.deepEqual(attributeNames(data.roles[0]), ['Mod attack', 'Mod defence', 'Mod planning', 'Name not recorded']);
});


test('delayed outcomes and result-only participation become eligible at their first read, preserving native dates', () => {
  const data = fixture(); const leader = data.leaders[0];
  const first = engineHour({ year: 1936, month: 1, day: 1, hour: 1 })!;
  data.firstHour = first; data.people[0].portraitHour = first;
  leader.runs[0] = { ...leader.runs[0], firstHour: first, lastHour: first + 6 };
  leader.traitGains[0].hour = first + 1;
  data.observed[0] = { ...data.observed[0], firstHour: first, lastHour: first + 6 };
  leader.battleStarts = [first, 2]; leader.battleStartReadHours = [first, first + 4];
  leader.unknownBattleStarts = [1]; leader.battleWon = [true, false];
  leader.battleResultHours = [first + 2, first + 2]; leader.battleResultReadHours = [first + 4, first + 4];
  const accepted = validateUnitLeadersReply(wire(data)); const career = accepted.leaders[0];
  assert.deepEqual(battleTotals(career, first + 2), { fought: 1, won: 0, lost: 0, unresolved: 1, winRate: null });
  assert.deepEqual(battleTotals(career, first + 4), { fought: 2, won: 1, lost: 1, unresolved: 0, winRate: 50 });
  const pending = careerLanes(accepted, career, first, first + 2, first + 2).at(-1)!.spans[0];
  assert.equal(pending.value, 1); assert.equal(pending.positiveCount, 0); assert.equal(pending.negativeCount, 0);
  assert.match(pending.label, /1 unresolved/);
  const span = careerLanes(accepted, career, first, first + 2, first + 4).at(-1)!.spans[0];
  assert.equal(span.value, 2); assert.equal(span.positiveCount, 1); assert.equal(span.negativeCount, 1);
  assert.equal(span.firstHour, first); assert.equal(span.lastHour, first + 2);
  const malformed = wire(data);
  malformed.leaders[0].battleStartReadHours = [first];
  assert.throws(() => validateUnitLeadersReply(malformed), /malformed/);
  malformed.leaders[0].battleStartReadHours = [first, first + 4];
  malformed.leaders[0].battleResultReadHours = [first + 1, first + 4];
  assert.throws(() => validateUnitLeadersReply(malformed), /malformed/);
});

test('current leader facts never turn preparing battle counts into zero', () => {
  const data = fixture(); data.preparing = ['battles', 'history'];
  const leader = leadersAt(data, 105, null, 'all')[0];
  assert.equal(leader.value.skill, 3);
  assert.equal(leader.preparing, true);
  for (const field of ['fought', 'won', 'lost', 'unresolved', 'winRate'] as const) assert.equal(leader[field], null);
  const tooltip = leaderTooltip(leader, { name: 'Country' });
  assert.equal(tooltip.rows?.find(row => row.label === 'Battles won / fought')?.value, 'Still being prepared');
});
