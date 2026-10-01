import test from 'node:test';
import assert from 'node:assert/strict';
import type { CountryGroupReply } from '../generated/dashboard';
import { groupLabel, groupMembers, groupRuns, groupUnobserved, membershipChanges } from './groups.ts';
import { parseTemplate } from './storage.ts';
import { defaultTemplate } from './model.ts';
import '../widgets/definitions.ts';

const REPLY: CountryGroupReply = {
  branchId: '1', warnings: [], warStarts: [], capitulations: [], identityChanges: [], unobserved: { humans: [], factions: [{ firstHour: 190, lastHour: 200 }], wars: [] },
  humans: [{ tag: 'GER', firstHour: 100, lastHour: 200 }, { tag: 'ENG', firstHour: 150, lastHour: 200 }],
  factions: [{ id: '5', name: 'Axis', firstHour: 100, lastHour: 200, members: [
    { tag: 'GER', firstHour: 100, lastHour: 200 }, { tag: 'ITA', firstHour: 120, lastHour: 170 }, { tag: 'HUN', firstHour: 180, lastHour: 200 }] , incomplete: [] },
  { id: '6', name: null, firstHour: 190, lastHour: 200, members: [] , incomplete: [] }],
  wars: [{ first: 'FRA', second: 'GER', firstHour: 130, lastHour: 200 }, { first: 'GER', second: 'POL', firstHour: 110, lastHour: 140 }],
};

test('members at each hour, at the end, or anyone ever', () => {
  const axis = groupRuns(REPLY, { type: 'faction', id: '5' })!;
  assert.deepEqual(groupMembers(axis, 'hour', 150, 200), ['GER', 'ITA']);
  assert.deepEqual(groupMembers(axis, 'hour', 175, 200), ['GER']);
  assert.deepEqual(groupMembers(axis, 'end', 150, 200), ['GER', 'HUN']);
  assert.deepEqual(groupMembers(axis, 'ever', 150, 200), ['GER', 'ITA', 'HUN']);
  assert.deepEqual(groupMembers(groupRuns(REPLY, { type: 'humans' })!, 'hour', 120, 200), ['GER']);
  assert.equal(groupRuns(REPLY, { type: 'faction', id: '99' }), null);
  assert.equal(groupRuns(REPLY, { type: 'top', count: 5 }), null);
});

test('at war with a country takes either side of each pair', () => {
  const enemies = groupRuns(REPLY, { type: 'war', tag: 'GER' })!;
  assert.deepEqual(groupMembers(enemies, 'hour', 135, 200), ['POL', 'FRA']);
  assert.deepEqual(groupMembers(enemies, 'hour', 160, 200), ['FRA']);
  assert.deepEqual(groupMembers(groupRuns(REPLY, { type: 'war', tag: 'FRA' })!, 'ever', 0, 200), ['GER']);
});

test('joins and leaves are markers, but the recording edges are not', () => {
  const changes = membershipChanges(groupRuns(REPLY, { type: 'faction', id: '5' })!, 100, 200);
  assert.deepEqual(changes, [
    { hour: 120, tag: 'ITA', change: 'joined' }, { hour: 171, tag: 'ITA', change: 'left' }, { hour: 180, tag: 'HUN', change: 'joined' }]);
});

test('group labels use the game faction name, or say it is unnamed', () => {
  const name = (tag: string) => ({ GER: 'German Reich' } as Record<string, string>)[tag] ?? tag;
  assert.equal(groupLabel({ type: 'faction', id: '5' }, REPLY, name), 'Axis');
  assert.equal(groupLabel({ type: 'faction', id: '6' }, REPLY, name), 'Unnamed faction');
  assert.equal(groupLabel({ type: 'war', tag: 'GER' }, REPLY, name), 'At war with German Reich');
  assert.equal(groupLabel({ type: 'top', count: 8 }, REPLY, name), 'Top 8');
});

test('saved group selections round-trip and reject malformed groups', () => {
  const template = defaultTemplate();
  const group = { kind: 'group', group: { type: 'faction', id: '5' }, membership: 'ever', aggregate: true } as const;
  template.widgets[0].countries = group;
  assert.deepEqual(parseTemplate(template)?.widgets[0].countries, group);
  for (const bad of [{ type: 'faction', id: 'x' }, { type: 'top', count: 0 }, { type: 'war', tag: '' }, { type: 'side' }]) {
    template.widgets[0].countries = { ...group, group: bad } as never;
    assert.equal(parseTemplate(template), null, JSON.stringify(bad));
  }
  template.widgets[0].countries = { ...group, membership: 'sometimes' } as never;
  assert.equal(parseTemplate(template), null);
});

test('an hour whose membership was not recorded is unknown, not an empty group', () => {
  const runs = [{ tag: 'GER', firstHour: 100, lastHour: 102 }, { tag: 'GER', firstHour: 106, lastHour: 110 }];
  const gap = [{ firstHour: 103, lastHour: 105 }];
  assert.equal(groupMembers(runs, 'hour', 104, 110, Infinity, gap), null);
  assert.deepEqual(groupMembers(runs, 'hour', 107, 110, Infinity, gap), ['GER']);
  assert.equal(groupMembers(runs, 'end', 107, 105, Infinity, gap), null);
  assert.deepEqual(groupMembers(runs, 'ever', 104, 110, Infinity, gap), ['GER']);
  // Each group has its own gaps: a faction gap does not hide human players.
  assert.deepEqual(groupUnobserved(REPLY, { type: 'humans' }), []);
  assert.deepEqual(groupUnobserved(REPLY, { type: 'faction', id: '5' }), [{ firstHour: 190, lastHour: 200 }]);
});

test('breaks next to unobserved hours are gaps, not joins or leaves', () => {
  const runs = [{ tag: 'GER', firstHour: 100, lastHour: 140 }, { tag: 'GER', firstHour: 146, lastHour: 200 }];
  assert.deepEqual(membershipChanges(runs, 100, 200, [{ firstHour: 141, lastHour: 145 }]), []);
  assert.deepEqual(membershipChanges(runs, 100, 200), [{ hour: 141, tag: 'GER', change: 'left' }, { hour: 146, tag: 'GER', change: 'joined' }]);
});

test('a group gives at most the widget limit', () => {
  const runs = Array.from({ length: 30 }, (_, index) => ({ tag: `T${index}`, firstHour: index, lastHour: 200 }));
  assert.equal(groupMembers(runs, 'ever', 0, 200, 16)?.length, 16);
  assert.deepEqual(groupMembers(runs, 'ever', 0, 200, 2), ['T0', 'T1']);
});

test('a faction is unobserved where no faction was read or its own roster was incomplete', () => {
  const reply: CountryGroupReply = { ...REPLY, factions: REPLY.factions.map((faction, index) => index === 0 ? { ...faction, incomplete: [{ firstHour: 50, lastHour: 50 }] } : faction) };
  const group = { type: 'faction' as const, id: reply.factions[0].id };
  assert.deepEqual(groupUnobserved(reply, group).map(span => span.firstHour), [50, 190]);
});
