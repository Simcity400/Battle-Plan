import assert from 'node:assert/strict';
import test from 'node:test';
import { leaderRows, withheldLeaders } from './generalsData.ts';
import type { LeaderRole, LeaderRow, UnitLeadersReply } from '../../generated/dashboard';

const role = (owner: string | null, kind = 1): LeaderRole => ({ owner, kind, category: 0, rank: 'Field Marshal', attributeNames: ['A', 'D', 'P', 'L'], maximumSkill: 9 });
const row = (reference: string, owner: string | null, skill: number | null, counts: [number, number, number], kind = 1): LeaderRow => ({
  reference, name: `Leader ${reference}`, portrait: { image: null, missing: null }, role: role(owner, kind), skill, components: [1, 2, 3, 4], traits: [0], traitDates: [[100, 1]],
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
  assert.equal(rows[0]!.unresolved, 2);
  assert.equal(rows[0]!.winRate, 75);
  assert.equal(rows[0]!.traits[0]!.token, 'organizer');
  assert.deepEqual(leaderRows(data, 200, null, 'land').map(r => r.key), ['1', '2', '4']);
  assert.deepEqual(leaderRows(data, 200, null, 'navy').map(r => r.key), ['3']);
  assert.equal(leaderRows(data, 200, null, 'land')[1]!.winRate, null, 'no decided battle is no win rate, never zero');
});

test('counts read after the battle log went unrecorded are lower bounds', () => {
  const data = reply([row('1', 'GER', 5, [10, 6, 2])], 150);
  assert.equal(leaderRows(data, 149, null, 'land')[0]!.foughtPartial, false);
  assert.equal(leaderRows(data, 150, null, 'land')[0]!.foughtPartial, true);
});

test('withheld leaders are kept by kind and country', () => {
  const data = reply([]);
  assert.equal(withheldLeaders(data, new Set(['ITA']), 'navy').length, 1);
  assert.equal(withheldLeaders(data, new Set(['GER']), 'navy').length, 0);
  assert.equal(withheldLeaders(data, null, 'land').length, 0);
});
