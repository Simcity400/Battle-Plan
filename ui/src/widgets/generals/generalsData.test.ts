import assert from 'node:assert/strict';
import test from 'node:test';
import { careerLanes, careerTotals, leaderRows, topLeader, withheldLeaders } from './generalsData.ts';
import type { CareerBattle, LeaderCareerReply, LeaderRole, LeaderRow, UnitLeadersReply } from '../../generated/dashboard';

const role = (owner: string | null, kind = 1): LeaderRole => ({ owner, kind, category: 0, rank: 'Field Marshal', attributeNames: ['A', 'D', 'P', 'L'], maximumSkill: 9 });
const row = (reference: string, owner: string | null, skill: number | null, counts: [number, number, number], kind = 1): LeaderRow => ({
  reference, name: `Leader ${reference}`, role: role(owner, kind), skill, components: [1, 2, 3, 4], traits: [0], traitDates: [[100, 1]],
  firstSeenHour: 50, fought: counts[0], won: counts[1], lost: counts[2],
});
const reply = (leaders: LeaderRow[], partialFrom: number | null = null): UnitLeadersReply => ({
  leaders, traits: [{ token: 'organizer', name: 'Organizer', icon: null, missing: null }],
  withheld: [{ reference: '9', owner: 'ITA', kind: 2, name: null, reason: 'not decided' }], missing: null, battlesPartialFrom: partialFrom, resultsMissing: '', skillIcons: { land: [], navy: [] }, warnings: [],
});

test('leaderboard rows keep the kind and countries asked, with unresolved battles and a win rate among decided ones', () => {
  const data = reply([row('1', 'GER', 5, [10, 6, 2]), row('2', 'ITA', 7, [3, 0, 0]), row('3', 'GER', 3, [0, 0, 0], 2), row('4', null, 4, [1, 0, 0])]);
  const rows = leaderRows(data, 200, new Set(['GER']), 'land');
  assert.deepEqual(rows.map(r => r.key), ['1']);
  assert.equal(rows[0].unresolved, 2);
  assert.equal(rows[0].winRate, 75);
  assert.equal(rows[0].traits[0].token, 'organizer');
  assert.deepEqual(leaderRows(data, 200, null, 'land').map(r => r.key), ['1', '2', '4']);
  assert.deepEqual(leaderRows(data, 200, null, 'navy').map(r => r.key), ['3']);
  assert.equal(leaderRows(data, 200, null, 'land')[1].winRate, null, 'no decided battle is no win rate, never zero');
  assert.equal(topLeader(leaderRows(data, 200, null, 'land'))?.key, '2');
});

test('counts read after the battle log went unrecorded are lower bounds', () => {
  const data = reply([row('1', 'GER', 5, [10, 6, 2])], 150);
  assert.equal(leaderRows(data, 149, null, 'land')[0].foughtPartial, false);
  assert.equal(leaderRows(data, 150, null, 'land')[0].foughtPartial, true);
});

test('withheld leaders are kept by kind and country', () => {
  const data = reply([]);
  assert.equal(withheldLeaders(data, new Set(['ITA']), 'navy').length, 1);
  assert.equal(withheldLeaders(data, new Set(['GER']), 'navy').length, 0);
  assert.equal(withheldLeaders(data, null, 'land').length, 0);
});


const battle = (start: number, read: number, won: boolean | null, resultRead: number | null): CareerBattle =>
  ({ start, read, unknown: false, kind: 1, won, resultHour: resultRead === null ? null : resultRead - 1, resultRead });
const career = (battles: CareerBattle[]): LeaderCareerReply => ({
  reference: '1', name: 'Leader 1', portraitHour: 300, runs: [{ firstHour: 0, lastHour: 300, role: role('GER'), skill: 3, components: [1, 1, 1, 1] }],
  traitGains: [[0, 0, 0], [120, 1, 1]], traits: [{ token: 'a', name: 'A', icon: null, missing: null }, { token: 'b', name: 'B', icon: null, missing: null }],
  commands: [{ firstHour: 10, lastHour: 50, formation: '7', kind: 'army', name: null }], battles, observed: [[0, 300]], battlesPartialFrom: null, resultsMissing: '', missing: null, warnings: [],
});

test('a career counts each battle from its read and each result from its own read', () => {
  const data = career([battle(10, 12, true, 20), battle(30, 30, false, 40), battle(50, 55, null, null)]);
  assert.deepEqual([careerTotals(data, 11).fought, careerTotals(data, 12).fought], [0, 1]);
  const at35 = careerTotals(data, 35);
  assert.deepEqual([at35.fought, at35.won, at35.lost, at35.unresolved], [2, 1, 0, 1]);
  const at60 = careerTotals(data, 60);
  assert.deepEqual([at60.fought, at60.won, at60.lost, at60.unresolved], [3, 1, 1, 1]);
});

test('career lanes date traits held before the view at its first hour and name an unnamed formation by its handle', () => {
  const lanes = careerLanes(career([]), 100, 300);
  const traits = lanes.find(lane => lane.id === 'traits')!;
  assert.deepEqual(traits.spans.map(span => span.firstHour), [100, 120]);
  assert.match(traits.spans[0].label ?? '', /^A · Held before/);
  const commands = careerLanes(career([]), 0, 300).find(lane => lane.id === 'commands')!;
  assert.equal(commands.spans[0].label, 'Formation 7');
});
