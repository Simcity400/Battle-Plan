import test from 'node:test';
import assert from 'node:assert/strict';
import type { CountryGroupReply } from '../generated/dashboard';
import type { CountrySelection } from './model';
import { aggregateGroupRuns, resolveCountries, projectResolvedCountries, rankedCandidates, countrySelectionMode, countrySelectionModes, countryMatch, countryMatchFilter, groupLabel, groupMembers, groupRuns, groupsKnownAt, groupsPendingAt, groupsSettled, groupUnobserved, membersKnown, membershipChanges, worldFilter, worldRequestTags } from './groups.ts';
import type { ResolvedCountries } from './groups';
import { createWidget } from './model.ts';
import { parseTemplate } from './storage.ts';
import { defaultTemplate } from './model.ts';
import { countryIdentityIndex, countryIdentityKey } from './countryIdentity.ts';
import '../widgets/definitions.ts';

const REPLY: CountryGroupReply = {
  branchId: '1', warnings: [], warStarts: [], capitulations: [], identityChanges: [], unobserved: { humans: [], factions: [{ firstHour: 190, lastHour: 200 }], wars: [] },
  humans: [{ tag: 'GER', firstHour: 100, lastHour: 200 }, { tag: 'ENG', firstHour: 150, lastHour: 200 }],
  factions: [{ id: '5', name: 'Axis', recordedName: 'Axis', firstHour: 100, lastHour: 200, nameReason: null, names: [{ firstHour: 100, lastHour: 200, name: 'Axis', reason: null }], members: [
    { tag: 'GER', firstHour: 100, lastHour: 200 }, { tag: 'ITA', firstHour: 120, lastHour: 170 }, { tag: 'HUN', firstHour: 180, lastHour: 200 }], leaders: [], incomplete: [] },
  { id: '6', name: null, recordedName: '', firstHour: 190, lastHour: 200, nameReason: 'Faction template not recorded', names: [{ firstHour: 190, lastHour: 200, name: null, reason: 'Faction template not recorded' }], members: [], leaders: [], incomplete: [] }],
  wars: [{ first: 'FRA', second: 'GER', firstHour: 130, lastHour: 200 }, { first: 'GER', second: 'POL', firstHour: 110, lastHour: 140 }],
  countryIdentities: ['GER', 'ENG', 'ITA', 'HUN', 'FRA', 'POL', 'D01', 'MY TAG', 'A B'].map((tag, index) => ({ tag,
    identities: [{ firstHour: 0, lastHour: 200, slot: index + 100, countryId: index + 200 }] })),
};

test('world filters resolve countries at the counted membership hour, including tag reuse within a run', () => {
  const identityChanges = [{ hour: 10, from: 'AAA', to: 'BBB' }];
  const first = { slot: 3, countryId: 4 }, other = { slot: 3, countryId: 5 };
  const reply: CountryGroupReply = { ...REPLY, identityChanges,
    humans: [{ tag: 'AAA', firstHour: 0, lastHour: 9 }, { tag: 'BBB', firstHour: 10, lastHour: 20 }],
    countryIdentities: [
      { tag: 'AAA', identities: [{ firstHour: 0, lastHour: 9, ...first }] },
      { tag: 'BBB', identities: [{ firstHour: 0, lastHour: 9, ...other }, { firstHour: 10, lastHour: 20, ...first }] },
    ],
  };
  const filter = (membership: 'hour' | 'end' | 'ever', data = reply) =>
    worldFilter({ kind: 'group', group: { type: 'humans' }, membership, aggregate: false }, [], data, 5, 20);
  assert.deepEqual([...filter('end').tags!], ['AAA']);
  assert.deepEqual(filter('end').countryIds, new Set([countryIdentityKey(first)]));
  assert.deepEqual(filter('hour').countryIds, filter('end').countryIds);
  assert.deepEqual(filter('ever').countryIds, filter('end').countryIds);
  // A tag's continuous membership can name two countries during the run.
  const reused = { ...reply, humans: [{ tag: 'BBB', firstHour: 0, lastHour: 20 }] };
  assert.deepEqual(filter('ever', reused).countryIds, new Set([countryIdentityKey(other), countryIdentityKey(first)]));
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
  const chosen = countryMatch({ kind: 'tags', tags: ['GER'] }, ['ENG'], REPLY, 200);
  assert.deepEqual([chosen.matches('GER', 1), chosen.matches('ENG', 1)], [true, false]);
  assert.equal(countryMatch({ kind: 'selected' }, ['ENG'], REPLY, 200).matches('ENG', 1), true);
  // A mod's tag may hold a space: it stays one tag.
  const spaced = countryMatch({ kind: 'selected' }, ['A B'], REPLY, 200);
  assert.deepEqual([spaced.matches('A B', 1), spaced.matches('A', 1), spaced.matches('B', 1)], [true, false, false]);
  assert.equal(countryMatch({ kind: 'selected' }, [], REPLY, 200).reason, 'Choose a country in the widget settings.');
  assert.equal(countryMatch({ kind: 'tags', tags: ['GER'] }, [], undefined, 200).ready, false);
  const axis = { kind: 'group', group: { type: 'faction', id: '5' }, aggregate: false } as const;
  assert.equal(countryMatch({ ...axis, membership: 'hour' }, [], undefined, 200).ready, false);
  const atHour = countryMatch({ ...axis, membership: 'hour' }, [], REPLY, 200);
  // Italy's rows pass only while it was a member; none pass where membership was not recorded.
  assert.deepEqual([atHour.matches('ITA', 150), atHour.matches('ITA', 175), atHour.matches('GER', 195), atHour.matches('GER', 150)], [true, false, false, true]);
  const atEnd = countryMatch({ ...axis, membership: 'end' }, [], REPLY, 180);
  assert.deepEqual([atEnd.matches('ITA', 150), atEnd.matches('HUN', 100)], [false, true]);
  // Factions were not recorded at the branch's last hour: its members then are unknown, not none.
  const unknownEnd = countryMatch({ ...axis, membership: 'end' }, [], REPLY, 200);
  assert.deepEqual([unknownEnd.matches('GER', 150), unknownEnd.reason], [false, 'Group membership is unavailable at this time.']);
  assert.equal(countryMatch({ ...axis, membership: 'ever' }, [], REPLY, 200).matches('ITA', 100), true);
  assert.equal(countryMatch({ ...axis, group: { type: 'faction', id: '99' }, membership: 'ever' }, [], REPLY, 200).reason, 'This group is unavailable in this campaign.');
});

test('a widget about the whole world shows all countries by default, narrowed by chosen ones or a group', () => {
  assert.deepEqual(createWidget('factions', 'main').countries, { kind: 'all' });
  assert.deepEqual(createWidget('construction-log', 'main').countries, { kind: 'all' });
  assert.deepEqual(createWidget('political-power', 'main').countries, { kind: 'selected' });
  const filter = (selection: Parameters<typeof worldFilter>[0], fallback: string[] = [], reply: CountryGroupReply | undefined = REPLY, hour = 150) =>
    worldFilter(selection, fallback, reply, hour, 200);
  assert.deepEqual(filter({ kind: 'all' }, ['GER']), { tags: null, reason: null, known: true });
  assert.deepEqual([...filter({ kind: 'selected' }, ['GER']).tags!], ['GER']);
  assert.deepEqual([...filter({ kind: 'tags', tags: ['D01', 'MY TAG'] }, ['GER']).tags!], ['D01', 'MY TAG']);
  assert.match(filter({ kind: 'selected' }).reason!, /Choose a country/);
  assert.match(filter({ kind: 'group', group: { type: 'faction', id: '9' }, membership: 'hour', aggregate: false }).reason!, /unavailable in this campaign/);
  // While the groups load, nothing is shown rather than everything.
  assert.deepEqual(worldFilter({ kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false }, [], undefined, 150, 200), { tags: new Set(), countryIds: new Set(), reason: null, known: false });
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
  assert.match(axis('end', 150).reason!, /Group membership is unavailable at this time/);
  assert.match(axis('hour', 195).reason!, /Group membership is unavailable at this time/);
  // Before any member joined, the group is empty at that moment.
  assert.match(axis('hour', 90).reason!, /no members at this moment/);
});

const range = { firstHour: 100, lastHour: 107 };
const members = (): ResolvedCountries => ({ known: () => true, tags: ['realm A', 'realm B'], membership: new Map([
  ['realm A', [{ tag: 'realm A', firstHour: 100, lastHour: 107 }]],
  ['realm B', [{ tag: 'realm B', firstHour: 102, lastHour: 105 }]],
]), unobserved: [{ firstHour: 104, lastHour: 104 }], dropped: 0, reason: null });

test('group sums follow joins/leaves and leave gaps for a missing member and unknown roster', () => {
  assert.deepEqual(aggregateGroupRuns([
    { tag: 'realm A', runs: [{ firstHour: 100, lastHour: 107, value: 5 }] },
    { tag: 'realm B', runs: [{ firstHour: 102, lastHour: 102, value: 2 }, { firstHour: 104, lastHour: 105, value: 3 }] },
  ], members(), range), [
    { firstHour: 100, lastHour: 101, value: 5 }, { firstHour: 102, lastHour: 102, value: 7 },
    { firstHour: 105, lastHour: 105, value: 8 }, { firstHour: 106, lastHour: 107, value: 5 },
  ]);
});

test('an aggregate keeps a member identity break, exact zero, and gaps when every member is missing', () => {
  const resolved = { ...members(), tags: ['realm A'], membership: null };
  assert.deepEqual(aggregateGroupRuns([{ tag: 'realm A', runs: [
    { firstHour: 100, lastHour: 101, value: 0 }, { firstHour: 102, lastHour: 103, value: 0, breakBefore: true },
  ] }], resolved, range), [{ firstHour: 100, lastHour: 101, value: 0 }, { firstHour: 102, lastHour: 103, value: 0, breakBefore: true }]);
  assert.deepEqual(aggregateGroupRuns([], resolved, range), []);
  assert.deepEqual(aggregateGroupRuns([], { ...resolved, tags: [] }, range), []);
});

test('a faction sum uses its recorded faction id without copying a 5,000-country roster to the client sum', () => {
  const tags = Array.from({ length: 5000 }, (_, index) => index === 4999 ? 'Renamed realm' : `realm ${index}`);
  const reply: CountryGroupReply = { branchId: '7', humans: [], factions: [{ id: 'custom faction', name: 'New union', nameReason: null, names: [{ ...range, name: 'New union', reason: null }], recordedName: 'New union', leaders: [], ...range, incomplete: [],
    members: tags.map(tag => ({ tag, ...range })) }], wars: [], warStarts: [], capitulations: [], identityChanges: [],
    unobserved: { humans: [], factions: [], wars: [] }, warnings: [], countryIdentities: tags.map((tag, slot) => ({ tag,
      identities: [{ ...range, slot, countryId: slot + 10_000 }] })) };
  const resolved = resolveCountries({ kind: 'group', group: { type: 'faction', id: 'custom faction' }, membership: 'hour', aggregate: true },
    [], reply, range.lastHour, Infinity, true);
  const ordinary = resolveCountries({ kind: 'group', group: { type: 'faction', id: 'custom faction' }, membership: 'hour', aggregate: true },
    [], reply, range.lastHour, Infinity);
  assert.deepEqual(new Set(ordinary.tags), new Set(tags), 'other widgets keep their country roster');
  assert.equal(ordinary.factionId, undefined);
  assert.equal(resolved.factionId, 'custom faction');
  assert.deepEqual(resolved.tags, []);
  assert.equal(resolved.membership, null);
  assert.throws(() => aggregateGroupRuns([], resolved, range), /faction-totals/);
});

test('ranked country widgets keep main selection segments and top groups stay in Group', () => {
  // A ranked widget's limits add no segment: top groups stay under Group.
  assert.deepEqual(countrySelectionModes({}).map(mode => mode.value), ['countries', 'group']);
  assert.deepEqual(countrySelectionModes({ all: true }).map(mode => mode.value), ['all', 'countries', 'group']);
  assert.deepEqual(countrySelectionModes({ factions: true }).map(mode => mode.value), ['countries', 'group', 'factions']);
  assert.equal(countrySelectionMode({ kind: 'group', group: { type: 'top', count: 10 }, aggregate: false, membership: 'hour' }), 'group');
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
  const reply = { ...REPLY, countryIdentities: tags.map((tag, slot) => ({ tag,
    identities: [{ firstHour: 0, lastHour: 200, slot, countryId: slot + 10_000 }] })) };
  assert.equal(countryMatchFilter({ kind: 'tags', tags }, [], reply, 200).membership?.length, 5000);
});

test('dated log country choices use full native holder histories with exact server/client predicate parity', () => {
  const reply: CountryGroupReply = { ...REPLY,
    identityChanges: [{ hour: 10, from: 'AAA', to: 'BBB' }, { hour: 20, from: 'BBB', to: 'CCC' }],
    humans: [{ tag: 'AAA', firstHour: 0, lastHour: 9 }, { tag: 'BBB', firstHour: 10, lastHour: 14 }, { tag: 'CCC', firstHour: 22, lastHour: 25 }],
    unobserved: { humans: [{ firstHour: 12, lastHour: 12 }], factions: [], wars: [] },
    countryIdentities: [
      { tag: 'AAA', identities: [{ firstHour: 0, lastHour: 9, slot: 1, countryId: 11 }] },
      { tag: 'BBB', identities: [{ firstHour: 0, lastHour: 9, slot: 2, countryId: 12 }, { firstHour: 10, lastHour: 11, slot: 1, countryId: 11 },
        { firstHour: 13, lastHour: 19, slot: 1, countryId: 11 }, { firstHour: 20, lastHour: 25, slot: 3, countryId: 13 }] },
      { tag: 'CCC', identities: [{ firstHour: 20, lastHour: 25, slot: 1, countryId: 11 }] },
    ] };
  const identities = countryIdentityIndex(reply.countryIdentities);
  const selections: CountrySelection[] = [{ kind: 'tags', tags: ['CCC'] }, { kind: 'selected' },
    ...(['hour', 'end', 'ever'] as const).map((membership): CountrySelection =>
      ({ kind: 'group', group: { type: 'humans' }, membership, aggregate: false }))];
  for (const selection of selections) {
    const client = countryMatch(selection, ['CCC'], reply, 25, identities);
    const filter = countryMatchFilter(selection, ['CCC'], reply, 25, identities, 0);
    for (const hour of [0, 9, 10, 12, 14, 15, 19, 20, 25]) for (const tag of ['AAA', 'BBB', 'CCC']) {
      const server = (filter.tags === null || filter.tags.includes(tag)) && (filter.membership === null ||
        filter.membership.some(run => run.tag === tag && run.firstHour <= hour && hour <= run.lastHour)) &&
        !filter.unobserved.some(span => span.firstHour <= hour && hour <= span.lastHour);
      assert.equal(server, client.matches(tag, hour), `${selection.kind}: ${tag}@${hour}`);
    }
  }
  const picked = countryMatch({ kind: 'tags', tags: ['CCC'] }, [], reply, 25, identities);
  assert.deepEqual([picked.matches('AAA', 9), picked.matches('BBB', 15), picked.matches('CCC', 25)], [true, true, true]);
  assert.deepEqual([picked.matches('AAA', 10), picked.matches('BBB', 25), picked.matches('BBB', 9)], [false, false, false]);
  const hourly = countryMatch(selections[2], [], reply, 25, identities);
  assert.deepEqual([hourly.matches('AAA', 9), hourly.matches('AAA', 10), hourly.matches('BBB', 12), hourly.matches('BBB', 14), hourly.matches('BBB', 15)],
    [true, false, false, true, false]);
  assert.equal(countryMatch({ kind: 'selected' }, ['CCC'], undefined, 25, null).ready, false);
  assert.equal(countryMatch({ kind: 'all' }, [], undefined, 25, null).matches('AAA', 10), true);
  assert.deepEqual(identities.aliasRuns(new Set([identities.keyAt('BBB', 0)!]), 0, 25), [{ tag: 'BBB', firstHour: 0, lastHour: 9 }]);
  assert.deepEqual(identities.aliasRuns(new Set([identities.keyAt('BBB', 25)!]), 0, 25), [{ tag: 'BBB', firstHour: 20, lastHour: 25 }]);
  assert.deepEqual(worldRequestTags({ kind: 'tags', tags: ['CCC'] }, [], reply, 25).tags, ['CCC']);
  assert.deepEqual(countryMatchFilter({ kind: 'tags', tags: ['CCC'] }, [], undefined, 25), { tags: null, membership: [], unobserved: [] });
});

test('manual selection includes every reused literal holder while smart groups retain their own dates', () => {
  const a = { slot: 4, countryId: 5 }, c = { slot: 4, countryId: 6 }, unrelated = { slot: 7, countryId: 8 };
  const reply = { ...REPLY, identityChanges: [], humans: [{ tag: 'X', firstHour: 1, lastHour: 2 }], countryIdentities: [
    { tag: 'earlier alias', identities: [{ firstHour: -3, lastHour: -1, ...a }] },
    { tag: 'X', identities: [{ firstHour: 0, lastHour: 4, ...a }, { firstHour: 10, lastHour: 14, ...c }] },
    { tag: 'Y', identities: [{ firstHour: 0, lastHour: 4, ...unrelated }, { firstHour: 5, lastHour: 9, ...a }] },
    { tag: 'later alias', identities: [{ firstHour: 15, lastHour: 19, ...c }] },
  ] };
  const manual: CountrySelection = { kind: 'tags', tags: ['X'] };
  const match = countryMatch(manual, [], reply, 19), filter = countryMatchFilter(manual, [], reply, 19, undefined, -3);
  const rows = [['earlier alias', -2, true], ['X', 2, true], ['Y', 6, true], ['X', 12, true], ['later alias', 18, true],
    ['Y', 2, false], ['X', 6, false], ['later alias', 20, false]] as const;
  for (const [tag, hour, expected] of rows) {
    assert.equal(match.matches(tag, hour), expected, `${tag}@${hour}`);
    assert.equal(filter.membership!.some(run => run.tag === tag && run.firstHour <= hour && hour <= run.lastHour), expected);
  }
  assert.deepEqual(worldFilter(manual, [], reply, 6, 19).countryIds, new Set([countryIdentityKey(a), countryIdentityKey(c)]));
  assert.deepEqual([...worldFilter(manual, [], reply, 6, 19).tags!], ['Y']);
  assert.deepEqual(worldRequestTags(manual, [], reply, 19).tags, ['X'], 'aliases do not become new literal selection roots');
  const resolved = resolveCountries(manual, [], reply, 19, Number.MAX_SAFE_INTEGER);
  const projected = projectResolvedCountries(resolved, filter);
  assert.deepEqual(resolved.tags, ['X'], 'drawing projection cannot mutate backend selection roots');
  assert.deepEqual(projected.membership?.get('Y'), [{ tag: 'Y', firstHour: 5, lastHour: 9 }]);
  assert.deepEqual(rankedCandidates(projected, -2, []), ['earlier alias']);
  assert.deepEqual(rankedCandidates(projected, 2, []), ['X']);
  assert.deepEqual(rankedCandidates(projected, 6, []), ['Y']);
  assert.deepEqual(rankedCandidates(projected, 12, []), ['X']);
  assert.deepEqual(rankedCandidates(projected, 18, []), ['later alias']);
  const ever = { kind: 'group', group: { type: 'humans' }, membership: 'ever', aggregate: false } as const;
  const smart = countryMatch(ever, [], reply, 19);
  assert.equal(smart.matches('Y', 6), true);
  assert.equal(smart.matches('X', 12), false);
  const endReply = { ...reply, humans: [{ tag: 'X', firstHour: 12, lastHour: 14 }] };
  const atEnd = countryMatch({ ...ever, membership: 'end' }, [], endReply, 14);
  assert.equal(atEnd.matches('X', 2), false);
  assert.equal(atEnd.matches('X', 12), true);
  assert.equal(atEnd.matches('later alias', 18), true);
  const endSelection = { ...ever, membership: 'end' } as const;
  const endDrawReply = { ...reply, humans: [{ tag: 'later alias', firstHour: 15, lastHour: 19 }] };
  const endProjection = projectResolvedCountries(resolveCountries(endSelection, [], endDrawReply, 19, Number.MAX_SAFE_INTEGER),
    countryMatchFilter(endSelection, [], endDrawReply, 19, undefined, -3));
  assert.deepEqual(rankedCandidates(endProjection, 2, []), []);
  assert.deepEqual(rankedCandidates(endProjection, 12, []), ['X']);
  assert.deepEqual(rankedCandidates(endProjection, 18, []), ['later alias']);
  const hourlySelection = { ...ever, membership: 'hour' } as const;
  const hourlyProjection = projectResolvedCountries(resolveCountries(hourlySelection, [], reply, 19, Number.MAX_SAFE_INTEGER),
    countryMatchFilter(hourlySelection, [], reply, 19));
  assert.deepEqual(rankedCandidates(hourlyProjection, 0, []), []);
  assert.deepEqual(rankedCandidates(hourlyProjection, 1, []), ['X']);
  assert.deepEqual(rankedCandidates(hourlyProjection, 6, []), []);
  assert.deepEqual(worldRequestTags({ ...ever, membership: 'hour' }, [], reply, 19).tags, ['X']);
  assert.equal(worldRequestTags({ ...ever, membership: 'hour' }, [], reply, 19).ready, true);
});

test('native drawing projection preserves ranking/errors and merges additional unknown coverage without changing roots', () => {
  const resolved: ResolvedCountries = { tags: ['literal root'], membership: null, dropped: 2, reason: null, known: () => true,
    unobserved: [{ firstHour: 2, lastHour: 3 }] };
  const filter = { tags: null, membership: [{ tag: 'dated alias', firstHour: 4, lastHour: 5 }],
    unobserved: [{ firstHour: 2, lastHour: 3 }, { firstHour: 7, lastHour: 8 }] };
  const projected = projectResolvedCountries(resolved, filter);
  assert.equal(projected.dropped, 2);
  assert.deepEqual(projected.tags, ['dated alias']);
  assert.deepEqual(projected.unobserved, [{ firstHour: 2, lastHour: 3 }, { firstHour: 7, lastHour: 8 }]);
  assert.deepEqual(resolved.tags, ['literal root']);
  const top = { ...resolved, top: 3 }, refused = { ...resolved, reason: 'Native membership is unavailable.' };
  assert.equal(projectResolvedCountries(top, filter), top);
  assert.equal(projectResolvedCountries(refused, filter), refused);
  assert.equal(projectResolvedCountries(resolved, { tags: null, membership: null, unobserved: [] }), resolved);
  assert.deepEqual(projectResolvedCountries(resolved, { tags: null, membership: [], unobserved: [] }).tags, []);
  const incomplete = projectResolvedCountries(resolved, { tags: null,
    membership: [{ tag: 'dated alias', firstHour: 1, lastHour: 10 }], unobserved: [{ firstHour: 4, lastHour: 6 }] });
  assert.deepEqual(incomplete.membership?.get('dated alias'), [
    { tag: 'dated alias', firstHour: 1, lastHour: 3 }, { tag: 'dated alias', firstHour: 7, lastHour: 10 },
  ]);
  assert.deepEqual(rankedCandidates(incomplete, 5, []), []);
});

test('hourly membership and server filters omit unrecorded identity spans even when membership itself is known', () => {
  const reply = { ...REPLY, humans: [{ tag: 'X', firstHour: 0, lastHour: 9 }],
    unobserved: { humans: [], factions: [], wars: [] }, countryIdentities: [{ tag: 'X', identities: [
      { firstHour: 0, lastHour: 3, slot: 4, countryId: 5 }, { firstHour: 6, lastHour: 9, slot: 4, countryId: 5 },
    ] }] };
  const selection = { kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false } as const;
  const match = countryMatch(selection, [], reply, 9), filter = countryMatchFilter(selection, [], reply, 9, undefined, 1);
  assert.deepEqual(filter.membership, [{ tag: 'X', firstHour: 1, lastHour: 3 }, { tag: 'X', firstHour: 6, lastHour: 9 }]);
  assert.equal(match.matches('X', 4), false);
  assert.equal(match.matches('X', 6), true);
});

test('a groups reply still being prepared is not proof that a choice has no countries', () => {
  // Answered before any holder or membership is read: every hour is pending.
  const partial = { ...REPLY, humans: [], countryIdentities: [], coverage: { firstHour: 0, lastHour: -1, complete: false },
    pending: [{ firstHour: 0, lastHour: 200 }] } as CountryGroupReply;
  assert.deepEqual([groupsSettled(partial), groupsSettled(REPLY), groupsPendingAt(partial, 150), groupsPendingAt(REPLY, 150)], [false, true, true, false]);
  const tags: CountrySelection = { kind: 'tags', tags: ['GER'] };
  assert.deepEqual([countryMatch(tags, [], partial, 200).ready, countryMatch(tags, [], partial, 200).reason], [false, null], 'still loading, not unavailable');
  assert.equal(worldFilter(tags, [], partial, 150, 200).reason, null);
  assert.deepEqual(worldRequestTags(tags, [], partial, 200), { tags: ['GER'], ready: true, reason: null }, 'a backend expands the literal pick itself, so it is not refused');
  const settled = { ...REPLY, countryIdentities: [] };
  assert.equal(countryMatch(tags, [], settled, 200).reason, 'Selected countries are unavailable in this campaign.', 'a final reply without the country proves it absent');
  const ever = { kind: 'group', group: { type: 'humans' }, membership: 'ever', aggregate: false } as const;
  assert.deepEqual([countryMatch(ever, [], partial, 200).ready, countryMatch(ever, [], partial, 200).reason], [false, null], 'a group with no members yet is loading');
  assert.equal(countryMatch({ kind: 'selected' }, [], partial, 200).reason, 'Choose a country in the widget settings.', 'an empty choice is still a choice to make');
  // At each hour, an hour still being prepared has no known members.
  const prepared = { ...REPLY, coverage: { firstHour: 0, lastHour: 160, complete: false }, pending: [{ firstHour: 161, lastHour: 200 }] } as CountryGroupReply;
  const hourly = countryMatch({ ...ever, membership: 'hour' }, [], prepared, 200);
  assert.deepEqual([hourly.matches('GER', 150), hourly.matches('GER', 170)], [true, false]);
});

test('a groups reply read through part of the branch keeps a recorded gap a gap and leaves only pending hours loading', () => {
  // Read through 160 with a recorded gap in the human players at 140-150; 161-200 still pending.
  const mixed = { ...REPLY, unobserved: { ...REPLY.unobserved, humans: [{ firstHour: 140, lastHour: 150 }] },
    coverage: { firstHour: 0, lastHour: 160, complete: false }, pending: [{ firstHour: 161, lastHour: 200 }] } as CountryGroupReply;
  assert.deepEqual([groupsKnownAt(mixed, 145), groupsKnownAt(mixed, 170), groupsKnownAt(REPLY, 170)], [true, false, true]);
  const humans = (membership: 'hour' | 'end' | 'ever') => ({ kind: 'group', group: { type: 'humans' }, membership, aggregate: false } as const);
  assert.equal(worldFilter(humans('hour'), [], mixed, 145, 200).reason, 'Group membership is unavailable at this time.', 'the recorded gap at 145 is not loading');
  const pending = worldFilter(humans('hour'), [], { ...mixed, humans: [] }, 170, 200);
  assert.deepEqual([pending.reason, pending.tags?.size], [null, 0], 'the same pick at 170, still pending, is loading');
  assert.equal(countryMatch(humans('hour'), [], mixed, 200).matches('GER', 145), false);
  // No humans at 50, read and recorded: a proven empty moment. The end, still pending, is not.
  assert.equal(worldFilter(humans('hour'), [], mixed, 50, 200).reason, 'This group has no members at this moment.');
  assert.equal(worldFilter(humans('end'), [], { ...mixed, humans: [] }, 50, 200).reason, null);
  assert.equal(worldFilter(humans('ever'), [], { ...mixed, humans: [] }, 50, 200).reason, null, 'ever waits for the whole branch');
  assert.equal(worldFilter({ kind: 'tags', tags: ['XXX'] }, [], mixed, 50, 200).reason, null, 'a pick not yet seen may still appear later');
  assert.equal(worldFilter({ kind: 'tags', tags: ['XXX'] }, [], REPLY, 50, 200).reason, 'Selected countries are unavailable in this campaign.');
});

test('membership still being prepared is dated: pending hours are unknown, never nonmembers, and a final reply proves absence', () => {
  // Read through 160, keyframes answer 161-170 approximately, 171-200 still pending.
  const partial = { ...REPLY, coverage: { firstHour: 0, lastHour: 160, complete: false }, approximate: [{ firstHour: 161, lastHour: 170 }],
    pending: [{ firstHour: 171, lastHour: 200 }] } as CountryGroupReply;
  assert.deepEqual([groupsKnownAt(partial, 100, 160), groupsKnownAt(partial, 100, 161), groupsKnownAt(partial, 165), groupsKnownAt(partial, 180), groupsKnownAt(REPLY, 0, 200)],
    [true, false, false, false, true], 'a range is known only inside the exact coverage');
  const humans = (membership: 'hour' | 'end' | 'ever') => ({ kind: 'group', group: { type: 'humans' }, membership, aggregate: false } as const);
  const hourly = countryMatch(humans('hour'), [], partial, 200);
  assert.deepEqual([hourly.known(150), hourly.known(100, 165), hourly.known(180)], [true, false, false]);
  // A keyframe still matches as sampled; only pending hours have no members.
  assert.deepEqual([hourly.matches('GER', 165), hourly.matches('GER', 180)], [true, false]);
  assert.equal(countryMatch(humans('hour'), [], REPLY, 200).known(0, 200), true);
  // Chosen tags, the end and ever need the hours beyond the rows' own.
  for (const selection of [{ kind: 'tags', tags: ['GER'] }, humans('end'), humans('ever')] as CountrySelection[]) {
    assert.equal(countryMatch(selection, [], partial, 200).known(150), false, JSON.stringify(selection));
    assert.equal(countryMatch(selection, [], REPLY, 200).known(150), true, JSON.stringify(selection));
  }
  assert.equal(countryMatch({ kind: 'all' }, [], partial, 200).known(180), true);
  assert.equal(countryMatch({ kind: 'selected' }, [], partial, 200).known(180), true, 'an empty choice is a choice to make, not loading');
  // A recorded gap is known: it says not recorded, it does not load.
  const gap = { ...REPLY, unobserved: { ...REPLY.unobserved, humans: [{ firstHour: 140, lastHour: 150 }] } };
  assert.deepEqual([countryMatch(humans('hour'), [], gap, 200).known(145), countryMatch(humans('hour'), [], gap, 200).matches('GER', 145)], [true, false]);
  assert.deepEqual([worldFilter(humans('hour'), [], partial, 150, 200).known, worldFilter(humans('hour'), [], partial, 180, 200).known,
    worldFilter(humans('end'), [], partial, 150, 200).known, worldFilter(humans('hour'), [], REPLY, 180, 200).known], [true, false, false, true]);
});

test('a group the prepared prefix does not name yet is loading, not unavailable', () => {
  // Faction 9 may still be founded in the hours being prepared.
  const partial = { ...REPLY, coverage: { firstHour: 0, lastHour: 160, complete: false }, pending: [{ firstHour: 161, lastHour: 200 }] } as CountryGroupReply;
  const later = (membership: 'hour' | 'end' | 'ever') => ({ kind: 'group', group: { type: 'faction', id: '9' }, membership, aggregate: false } as const);
  for (const membership of ['hour', 'end', 'ever'] as const) {
    const match = countryMatch(later(membership), [], partial, 200);
    assert.deepEqual([match.ready, match.reason, match.known(150)], [false, null, false], membership);
    assert.deepEqual([worldFilter(later(membership), [], partial, 150, 200).reason, worldFilter(later(membership), [], partial, 150, 200).known], [null, false], membership);
    const resolved = resolveCountries(later(membership), [], partial, 200, 10);
    assert.deepEqual([resolved.reason, resolved.known(150)], [null, false], membership);
    // The final reply proves it was never recorded.
    assert.equal(countryMatch(later(membership), [], REPLY, 200).reason, 'This group is unavailable in this campaign.', membership);
    assert.equal(worldFilter(later(membership), [], REPLY, 150, 200).reason, 'This group is unavailable in this campaign.', membership);
    assert.equal(resolveCountries(later(membership), [], REPLY, 200, 10).reason, 'This group is unavailable in this campaign.', membership);
  }
});

test('resolved countries are prefix-aware: no reason holds until the membership counted is read', () => {
  const partial = { ...REPLY, humans: [], coverage: { firstHour: 0, lastHour: 160, complete: false }, pending: [{ firstHour: 161, lastHour: 200 }] } as CountryGroupReply;
  const humans = (membership: 'hour' | 'end' | 'ever') => ({ kind: 'group', group: { type: 'humans' }, membership, aggregate: false } as const);
  // At the end, with the last hour still pending: no members yet is not an empty group.
  const end = resolveCountries(humans('end'), [], partial, 200, 10);
  assert.deepEqual([end.tags, end.reason, end.known(150)], [[], null, false]);
  assert.equal(resolveCountries(humans('end'), [], { ...REPLY, humans: [] }, 200, 10).reason, 'This group has no members in this campaign.');
  // Each hour: the union is the whole branch's, but each hour read is known on its own.
  const hourly = resolveCountries(humans('hour'), [], partial, 200, 10);
  assert.deepEqual([hourly.reason, hourly.known(150), hourly.known(150, 170), rankedCandidates(hourly, 150, [])], [null, true, false, []]);
  assert.equal(resolveCountries(humans('ever'), [], partial, 200, 10).reason, null);
  // A recorded gap at the end, read, is still unrecorded rather than loading.
  const gap = { ...REPLY, humans: [], unobserved: { ...REPLY.unobserved, humans: [{ firstHour: 150, lastHour: 160 }] },
    coverage: { firstHour: 0, lastHour: 160, complete: false }, pending: [{ firstHour: 161, lastHour: 200 }] } as CountryGroupReply;
  assert.equal(resolveCountries(humans('end'), [], gap, 160, 10).reason, 'Group membership is unavailable at this time.');
  // Chosen tags and top N need no group history; a group without its reply yet is not known.
  assert.equal(resolveCountries({ kind: 'tags', tags: ['GER'] }, [], partial, 200, 10).known(180), true);
  assert.equal(resolveCountries({ kind: 'group', group: { type: 'top', count: 3 }, membership: 'hour', aggregate: false }, [], undefined, 200, 10).known(180), true);
  assert.equal(resolveCountries(humans('hour'), [], undefined, 200, 10).known(0), false);
  assert.deepEqual([membersKnown(humans('end'), partial, 160)(0), membersKnown(humans('end'), partial, 200)(0), membersKnown(humans('ever'), REPLY, 200)(0)],
    [true, false, true]);
});
