import test from 'node:test';
import assert from 'node:assert/strict';
import type { CountryGroupReply } from '../generated/dashboard';
import type { CountrySelection } from './model';
import { countryMatch, countryMatchFilter, groupLabel, groupMembers, groupRuns, groupUnobserved, membershipChanges, worldFilter } from './groups.ts';
import { createWidget } from './model.ts';
import { parseTemplate } from './storage.ts';
import { defaultTemplate } from './model.ts';
import { Lineage } from './identity.ts';
import '../widgets/definitions.ts';

const REPLY: CountryGroupReply = {
  branchId: '1', warnings: [], warStarts: [], capitulations: [], identityChanges: [], unobserved: { humans: [], factions: [{ firstHour: 190, lastHour: 200 }], wars: [] },
  humans: [{ tag: 'GER', firstHour: 100, lastHour: 200 }, { tag: 'ENG', firstHour: 150, lastHour: 200 }],
  factions: [{ id: '5', name: 'Axis', recordedName: 'Axis', firstHour: 100, lastHour: 200, nameReason: null, names: [{ firstHour: 100, lastHour: 200, name: 'Axis', reason: null }], members: [
    { tag: 'GER', firstHour: 100, lastHour: 200 }, { tag: 'ITA', firstHour: 120, lastHour: 170 }, { tag: 'HUN', firstHour: 180, lastHour: 200 }], leaders: [], incomplete: [] },
  { id: '6', name: null, recordedName: '', firstHour: 190, lastHour: 200, nameReason: 'Faction template not recorded', names: [{ firstHour: 190, lastHour: 200, name: null, reason: 'Faction template not recorded' }], members: [], leaders: [], incomplete: [] }],
  wars: [{ first: 'FRA', second: 'GER', firstHour: 130, lastHour: 200 }, { first: 'GER', second: 'POL', firstHour: 110, lastHour: 140 }],
};

test('world filters resolve countries at the counted membership hour, including tag reuse within a run', () => {
  const identityChanges = [{ hour: 10, from: 'AAA', to: 'BBB' }];
  const lineage = new Lineage(identityChanges);
  const reply: CountryGroupReply = { ...REPLY, identityChanges,
    humans: [{ tag: 'AAA', firstHour: 0, lastHour: 9 }, { tag: 'BBB', firstHour: 10, lastHour: 20 }],
  };
  const filter = (membership: 'hour' | 'end' | 'ever', data = reply) =>
    worldFilter({ kind: 'group', group: { type: 'humans' }, membership, aggregate: false }, [], data, 5, 20, lineage);
  assert.deepEqual([...filter('end').tags!], ['BBB']);
  assert.deepEqual(filter('end').countryIds, new Set([lineage.countryAt('AAA', 5)]));
  assert.deepEqual(filter('hour').countryIds, filter('end').countryIds);
  assert.deepEqual(filter('ever').countryIds, filter('end').countryIds);
  // A tag's continuous membership can name two countries during the run.
  const reused = { ...reply, humans: [{ tag: 'BBB', firstHour: 0, lastHour: 20 }] };
  assert.deepEqual(filter('ever', reused).countryIds, new Set([lineage.countryAt('BBB', 0), lineage.countryAt('BBB', 10)]));
});

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
  // Anyone ever: none recorded is empty only when every hour was recorded.
  assert.equal(groupMembers([], 'ever', 104, 110, Infinity, gap), null);
  assert.deepEqual(groupMembers([], 'ever', 104, 110), []);
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
  const reply: CountryGroupReply = { ...REPLY, factions: REPLY.factions.map((faction, index) => index === 0 ? { ...faction, leaders: [], incomplete: [{ firstHour: 50, lastHour: 50 }] } : faction) };
  const group = { type: 'faction' as const, id: reply.factions[0].id };
  assert.deepEqual(groupUnobserved(reply, group).map(span => span.firstHour), [50, 190]);
});

test('a log row passes the country choice at its own hour, and all countries pass everything', () => {
  const all = countryMatch({ kind: 'all' }, [], undefined, 200);
  assert.equal(all.ready && all.matches('XYZ', 1), true);
  const chosen = countryMatch({ kind: 'tags', tags: ['GER'] }, ['ENG'], undefined, 200);
  assert.deepEqual([chosen.matches('GER', 1), chosen.matches('ENG', 1)], [true, false]);
  assert.equal(countryMatch({ kind: 'selected' }, ['ENG'], undefined, 200).matches('ENG', 1), true);
  // A mod's tag may hold a space: it stays one tag.
  const spaced = countryMatch({ kind: 'selected' }, ['A B'], undefined, 200);
  assert.deepEqual([spaced.matches('A B', 1), spaced.matches('A', 1), spaced.matches('B', 1)], [true, false, false]);
  assert.equal(countryMatch({ kind: 'selected' }, [], undefined, 200).reason, 'Choose a country in the widget settings.');
  const axis = { kind: 'group', group: { type: 'faction', id: '5' }, aggregate: false } as const;
  assert.equal(countryMatch({ ...axis, membership: 'hour' }, [], undefined, 200).ready, false);
  const atHour = countryMatch({ ...axis, membership: 'hour' }, [], REPLY, 200);
  // Italy's rows pass only while it was a member; none pass where membership was not recorded.
  assert.deepEqual([atHour.matches('ITA', 150), atHour.matches('ITA', 175), atHour.matches('GER', 195), atHour.matches('GER', 150)], [true, false, false, true]);
  const atEnd = countryMatch({ ...axis, membership: 'end' }, [], REPLY, 180);
  assert.deepEqual([atEnd.matches('ITA', 150), atEnd.matches('HUN', 100)], [false, true]);
  // Factions were not recorded at the branch's last hour: its members then are unknown, not none.
  const unknownEnd = countryMatch({ ...axis, membership: 'end' }, [], REPLY, 200);
  assert.deepEqual([unknownEnd.matches('GER', 150), unknownEnd.reason], [false, 'This group’s members were not recorded at the end of this campaign branch.']);
  assert.equal(countryMatch({ ...axis, membership: 'ever' }, [], REPLY, 200).matches('ITA', 100), true);
  assert.equal(countryMatch({ ...axis, group: { type: 'faction', id: '99' }, membership: 'ever' }, [], REPLY, 200).reason, 'This group is not in this campaign branch.');
});

test('a widget about the whole world shows all countries by default, narrowed by chosen ones or a group', () => {
  assert.deepEqual(createWidget('factions', 'main').countries, { kind: 'all' });
  assert.deepEqual(createWidget('construction-log', 'main').countries, { kind: 'all' });
  assert.deepEqual(createWidget('political-power', 'main').countries, { kind: 'selected' });
  const filter = (selection: Parameters<typeof worldFilter>[0], fallback: string[] = [], reply: CountryGroupReply | undefined = REPLY, hour = 150) =>
    worldFilter(selection, fallback, reply, hour, 200);
  assert.deepEqual(filter({ kind: 'all' }, ['GER']), { tags: null, reason: null });
  assert.deepEqual([...filter({ kind: 'selected' }, ['GER']).tags!], ['GER']);
  assert.deepEqual([...filter({ kind: 'tags', tags: ['D01', 'MY TAG'] }, ['GER']).tags!], ['D01', 'MY TAG']);
  assert.match(filter({ kind: 'selected' }).reason!, /Choose a country/);
  assert.match(filter({ kind: 'group', group: { type: 'faction', id: '9' }, membership: 'hour', aggregate: false }).reason!, /not in this campaign branch/);
  // While the groups load, nothing is shown rather than everything.
  assert.deepEqual(worldFilter({ kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false }, [], undefined, 150, 200), { tags: new Set(), reason: null });
});

test('a whole-world widget narrowed to a group honours its membership setting', () => {
  const humans = (membership: 'hour' | 'end' | 'ever', hour: number) =>
    worldFilter({ kind: 'group', group: { type: 'humans' }, membership, aggregate: false }, [], REPLY, hour, 200);
  // At each hour: the members at the moment shown. ENG is human only from 150.
  assert.deepEqual([...humans('hour', 120).tags!], ['GER']);
  assert.deepEqual([...humans('hour', 160).tags!].sort(), ['ENG', 'GER']);
  // At the end: the members at the branch's last hour, whatever the moment shown.
  assert.deepEqual([...humans('end', 120).tags!].sort(), ['ENG', 'GER']);
  // Anyone ever.
  assert.deepEqual([...humans('ever', 100).tags!].sort(), ['ENG', 'GER']);
  const axis = (membership: 'hour' | 'end' | 'ever', hour: number) =>
    worldFilter({ kind: 'group', group: { type: 'faction', id: '5' }, membership, aggregate: false }, [], REPLY, hour, 200);
  assert.deepEqual([...axis('hour', 150).tags!].sort(), ['GER', 'ITA']);
  assert.deepEqual([...axis('ever', 150).tags!].sort(), ['GER', 'HUN', 'ITA']);
  // The faction's roster was not recorded from 190: unknown, not empty.
  assert.match(axis('end', 150).reason!, /not recorded at the end/);
  assert.match(axis('hour', 195).reason!, /not recorded at this moment/);
  // Before any member joined, the group is empty at that moment.
  assert.match(axis('hour', 90).reason!, /no members at this moment/);
});

test('group headers use the name at the shown hour, with campaign overrides first', () => {
  const faction = { ...REPLY.factions[0], name: 'Later pact', names: [
    { firstHour: 100, lastHour: 149, name: 'Opening pact', reason: null },
    { firstHour: 160, lastHour: 200, name: 'Later pact', reason: null },
  ] };
  const reply = { ...REPLY, factions: [faction] };
  const group = { type: 'faction', id: faction.id } as const;
  assert.equal(groupLabel(group, reply, tag => tag, 120), 'Opening pact');
  assert.equal(groupLabel(group, reply, tag => tag, 155), 'Unnamed faction');
  assert.equal(groupLabel(group, reply, tag => tag, 170), 'Later pact');
  assert.equal(groupLabel(group, reply, tag => tag), 'Later pact');
  assert.equal(groupLabel(group, reply, tag => tag, 155, { [faction.id]: { name: 'My pact' } }), 'My pact');
});

test('server top-N country predicate matches the dated client country rule without a chosen country cap', () => {
  const selections: CountrySelection[] = [
    { kind: 'all' }, { kind: 'tags', tags: ['GER', 'ITA'] },
    ...(['hour', 'end', 'ever'] as const).map((membership): CountrySelection => ({ kind: 'group', group: { type: 'faction', id: '5' }, membership, aggregate: false })),
  ];
  for (const selection of selections) {
    const client = countryMatch(selection, [], REPLY, 200);
    const filter = countryMatchFilter(selection, [], REPLY, 200);
    for (const hour of [100, 125, 175, 190, 200]) for (const tag of ['GER', 'ITA', 'HUN', 'ENG']) {
      const matches = (filter.tags === null || filter.tags.includes(tag)) && (filter.membership === null ||
        !filter.unobserved.some(span => span.firstHour <= hour && span.lastHour >= hour) &&
        filter.membership.some(run => run.tag === tag && run.firstHour <= hour && run.lastHour >= hour));
      assert.equal(matches, client.matches(tag, hour));
    }
  }
  const tags = Array.from({ length: 5000 }, (_, i) => `opaque ${i}`);
  assert.equal(countryMatchFilter({ kind: 'tags', tags }, [], undefined, 200).tags?.length, 5000);
});
