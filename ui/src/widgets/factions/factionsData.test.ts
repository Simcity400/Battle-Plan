import test from 'node:test';
import assert from 'node:assert/strict';
import type { CountryGroupReply, FactionGroup, GroupCount } from '../../generated/dashboard';
import { rankedCounts } from '../../dashboard/groupCounts.ts';
import { LEADER_GLYPH, UNNAMED, factionChangeHours, factionGaps, factionLabel, factionLanes, factionViews, laneCountries, leaderAt, leadershipParts, membersAt, tagAt } from './factionsData.ts';
import type { LaneOptions } from './factionsData';
import { worldFilter } from '../../dashboard/groups.ts';
import { factionNameAt } from '../../dashboard/factionNames.ts';
import { laneRequests } from '../../charts/timeline/laneRequests.ts';
import { countryIdentityIndex } from '../../dashboard/countryIdentity.ts';

const run = (tag: string, firstHour: number, lastHour: number) => ({ tag, firstHour, lastHour });
const holder = (tag: string, slot: number, firstHour = 100, lastHour = 500, countryId = slot) => ({ tag, identities: [{ slot, countryId, firstHour, lastHour }] });

const factions: FactionGroup[] = [
  { id: '4294967384', name: null, recordedName: '', firstHour: 100, lastHour: 400, nameReason: 'Faction template not recorded', names: [{ firstHour: 100, lastHour: 400, name: null, reason: 'Faction template not recorded', token: null }],
    members: [run('D01', 100, 400), run('ITA', 150, 249), run('ITA', 300, 400), run('HUN', 120, 400)],
    leaders: [run('D01', 100, 299), run('HUN', 300, 400)], incomplete: [{ firstHour: 350, lastHour: 350 }] },
  { id: '25769803864', name: 'Pact of Steel Reforged', recordedName: 'Pact of Steel Reforged', firstHour: 200, lastHour: 299, nameReason: null, names: [{ firstHour: 200, lastHour: 299, name: 'Pact of Steel Reforged', reason: null, token: null }],
    members: [run('MY TAG', 200, 299), run('A:B', 220, 299)], leaders: [run('MY TAG', 200, 299)], incomplete: [] },
];
const reply: CountryGroupReply = {
  branchId: '1', humans: [], factions, wars: [], warStarts: [], capitulations: [], identityChanges: [],
  countryIdentities: ['D01', 'ITA', 'HUN', 'MY TAG', 'A:B'].map((tag, slot) => holder(tag, slot)),
  unobserved: { humans: [], factions: [{ firstHour: 260, lastHour: 269 }], wars: [], unattributed: [] }, warnings: [],
};
const names: Record<string, string> = { D01: 'Free Republic', ITA: 'Italy', HUN: 'Hungary', 'MY TAG': 'Modded Realm', 'A:B': 'Colon Land' };
const text = { country: (tag: string) => ({ name: names[tag] ?? tag, flag: `flag:${tag}` }), day: (hour: number) => `day ${hour}` };
const counted = (hour: number, groups: readonly [string, number | null][]) => rankedCounts({ branchId: '1', of: 'factions', hour, nextChange: null, missing: null, warnings: [],
  groups: groups.map(([key, count]): GroupCount => ({ key, count, missing: count === null ? 'Its roster was not read whole at this hour' : null })) });
const options: LaneOptions = { order: 'founded', members: 'joined', ended: true, countries: null, counts: null };
const summaryName = (faction: FactionGroup) => ({ name: faction.name, reason: faction.nameReason });
const views = factionViews(reply, summaryName, id => `colour-${id}`);


test('a faction that has ended keeps the name it ended with, and one never named stays unnamed', () => {
  const dated = factionViews(reply, (faction, hour) => factionNameAt(faction, hour), id => `colour-${id}`);
  const pact = dated.find(view => view.id === factions[1].id)!, template = dated.find(view => view.id === factions[0].id)!;
  assert.equal(factionLabel(pact, 250), 'Pact of Steel Reforged');
  assert.equal(factionLabel(pact, 400), 'Pact of Steel Reforged', 'dissolved at 299, never unnamed after');
  assert.equal(factionLabel(template, 400), UNNAMED);
});

test('a late recorded name outside the window moves its member from Zulu to Aardvark', () => {
  const tags = Array.from({ length: 50 }, (_, index) => `T${index}`);
  const metadata = new Map(tags.map((tag, index) => [tag, index === 49 ? 'Zulu' : `Member ${String(index).padStart(2, '0')}`]));
  const recorded = new Map<string, string>();
  const labels = { ...text, country: (tag: string) => ({ name: recorded.get(tag) ?? metadata.get(tag) ?? tag }) };
  const views = factionViews({ ...reply, identityChanges: [], unobserved: { humans: [], factions: [], wars: [], unattributed: [] },
    countryIdentities: tags.map((tag, slot) => holder(tag, slot, 0, 20)),
    factions: [{ id: '1', name: 'Pact', recordedName: 'Pact', firstHour: 0, lastHour: 20, nameReason: null, names: [{ firstHour: 0, lastHour: 20, name: 'Pact', reason: null, token: null }], incomplete: [], leaders: [],
      members: tags.map(tag => run(tag, 0, 20)) }],
  }, summaryName, id => id);
  for (const members of ['name', 'joined'] as const) {
    recorded.delete('T49');
    const layout = () => factionLanes(views, 20, { ...options, members }, labels, [], 0);
    const before = layout();
    assert.equal(before.lanes.at(-1)!.identity?.label, 'Zulu', members);
    const requests = laneRequests(before.lanes, [0, 24]);
    assert.ok(!laneCountries(requests.looks, before.info, 20).includes('T49'), members);
    const wanted = laneCountries(requests.names, before.info, 20);
    if (wanted.includes('T49')) recorded.set('T49', 'Aardvark');
    assert.ok(layout().lanes.findIndex(lane => lane.identity?.label === 'Aardvark') <= 2, members);
  }
});

test('end membership keeps the human member’s faction visible before its change of tag', () => {
  const humans = [run('AAA', 0, 9), run('BBB', 10, 20)];
  const renamed: CountryGroupReply = { ...reply, humans,
    countryIdentities: [holder('AAA', 1, 0, 9), holder('BBB', 1, 10, 20)],
    identityChanges: [{ hour: 10, from: 'AAA', to: 'BBB' }], unobserved: { humans: [], factions: [], wars: [], unattributed: [] },
    factions: [{ id: '1', name: 'Pact', recordedName: 'Pact', firstHour: 0, lastHour: 20, nameReason: null, names: [{ firstHour: 0, lastHour: 20, name: 'Pact', reason: null, token: null }], members: humans, leaders: [], incomplete: [] }],
  };
  const views = factionViews(renamed, summaryName, id => id);
  const filter = worldFilter({ kind: 'group', group: { type: 'humans' }, membership: 'hour', aggregate: false }, [], renamed, 5);
  const lanes = factionLanes(views, 5, { ...options, countries: filter.tags, countryIds: filter.countryIds }, text, [], 0).lanes;
  assert.deepEqual(lanes.map(lane => lane.identity?.label), ['Pact', 'AAA']);
});

test('after members swap tags and one leaves, the remaining member has the recorded tag', () => {
  const swapped: CountryGroupReply = { ...reply,
    countryIdentities: [{ tag: 'AAA', identities: [...holder('AAA', 1, 0, 9).identities, ...holder('AAA', 2, 10, 20).identities] },
      { tag: 'BBB', identities: [...holder('BBB', 2, 0, 9).identities, ...holder('BBB', 1, 10, 20).identities] }],
    identityChanges: [{ hour: 10, from: 'AAA', to: 'BBB' }, { hour: 10, from: 'BBB', to: 'AAA' }],
    unobserved: { humans: [], factions: [], wars: [], unattributed: [] },
    factions: [{ id: '1', name: 'Pact', recordedName: 'Pact', firstHour: 0, lastHour: 20, nameReason: null, names: [{ firstHour: 0, lastHour: 20, name: 'Pact', reason: null, token: null }], incomplete: [], leaders: [],
      members: [run('AAA', 0, 20), run('BBB', 0, 14)] }],
  };
  const [view] = factionViews(swapped, summaryName, id => id);
  const held = (hour: number) => view.members.flatMap(member => member.runs.some(item => item.firstHour <= hour && hour <= item.lastHour) ? [tagAt(view, member, hour)] : []);
  assert.deepEqual(held(5).sort(), ['AAA', 'BBB']);
  assert.deepEqual(held(16), ['AAA']);
});

test('factions keep recorded ids, colours and names, and count members over known hours only', () => {
  assert.deepEqual(views.map(view => [view.id, view.name, view.color]), [['4294967384', null, 'colour-4294967384'], ['25769803864', 'Pact of Steel Reforged', 'colour-25769803864']]);
  const [first, second] = views;
  assert.deepEqual(second.existence, [{ firstHour: 200, lastHour: 259 }, { firstHour: 270, lastHour: 299 }]);
  assert.equal(membersAt(first, 130), 2);
  assert.equal(membersAt(first, 200), 3);
  assert.equal(membersAt(first, 350), null);
  assert.equal(membersAt(first, 265), null);
  assert.equal(leaderAt(first.leaders, 310), 'HUN');
  assert.deepEqual(leadershipParts(run('D01', 100, 400), first.leaders), [{ firstHour: 100, lastHour: 299, leader: true }, { firstHour: 300, lastHour: 400, leader: false }]);
});

test('lanes list each faction founded by the hour, then its members, the leader marked', () => {
  const early = factionLanes(views, 180, { ...options, counts: counted(180, [[factions[0].id, 3]]) }, text, reply.unobserved.factions, 100);
  assert.deepEqual(early.lanes.map(lane => lane.identity?.label), [UNNAMED, 'Free Republic', 'Hungary', 'Italy']);
  assert.equal(early.lanes[0].heading, true);
  assert.equal(early.lanes[0].label, '3 members', 'the heading counts members; the leader is marked in its row');
  assert.equal(early.lanes[1].label, `${LEADER_GLYPH} Leader · Member before day 100`);
  assert.equal(early.lanes[3].label, 'Joined day 150');
  assert.equal(early.lanes[1].spans[0].glyph, LEADER_GLYPH);
  assert.deepEqual(early.lanes[0].spans[0].band?.slice(0, 3).map(step => step.value), [1, 2, 3]);
  const later = factionLanes(views, 320, options, text, reply.unobserved.factions, 100);
  const labels = later.lanes.map(lane => `${lane.identity?.label}: ${lane.label}`);
  assert.ok(labels.includes('Pact of Steel Reforged: Dissolved day 300'), labels.join(' | '));
  assert.ok(labels.includes(`Hungary: ${LEADER_GLYPH} Leader · Joined day 120`), labels.join(' | '));
  assert.ok(labels.includes('Colon Land: Left day 300'), labels.join(' | '));
  const ids = later.lanes.map(lane => lane.id);
  assert.equal(new Set(ids).size, ids.length);
  assert.ok(later.lanes.some(lane => later.info.get(lane.id)?.member?.runs[0].tag === 'A:B'));
  assert.deepEqual(laneCountries(later.lanes, later.info, 320).sort(), ['A:B', 'D01', 'HUN', 'ITA', 'MY TAG']);
});

test('dissolved factions and former members can be hidden, and chosen countries narrow the factions', () => {
  const hidden = factionLanes(views, 320, { ...options, ended: false }, text, reply.unobserved.factions, 100);
  assert.deepEqual(hidden.lanes.map(lane => lane.identity?.label), [UNNAMED, 'Free Republic', 'Hungary', 'Italy']);
  const chosen = factionLanes(views, 320, { ...options, countries: new Set(['MY TAG']), countryIds: countryIdentityIndex(reply.countryIdentities).selected(['MY TAG']) }, text, reply.unobserved.factions, 100);
  assert.deepEqual(chosen.lanes.map(lane => lane.identity?.label), ['Pact of Steel Reforged', 'Modded Realm', 'Colon Land']);
  const byMembers = factionLanes(views, 280, { ...options, order: 'members', counts: counted(280, [[factions[1].id, 2], [factions[0].id, null]]) }, text, reply.unobserved.factions, 100);
  assert.deepEqual(byMembers.lanes.filter(lane => lane.heading).map(lane => [lane.identity?.label, lane.label]), [['Pact of Steel Reforged', '2 members'], [UNNAMED, 'Members not recorded']],
    'the product ranks: a faction whose count it does not know follows, unranked, never counted here');
  const byName = factionLanes(views, 280, { ...options, order: 'name', members: 'name' }, text, reply.unobserved.factions, 100);
  assert.deepEqual(byName.lanes.map(lane => lane.identity?.label),
    ['Pact of Steel Reforged', 'Colon Land', 'Modded Realm', UNNAMED, 'Free Republic', 'Hungary', 'Italy']);
});

test('a member whose tag changes stays one member, neither leaving nor joining', () => {
  const renamed: CountryGroupReply = {
    ...reply, identityChanges: [{ hour: 200, from: 'AAA', to: 'BBB' }], unobserved: { humans: [], factions: [], wars: [], unattributed: [] },
    countryIdentities: [holder('AAA', 1, 100, 199), holder('BBB', 1, 200, 300), holder('CCC', 2, 100, 300)],
    factions: [{ id: '1', name: 'Pact', recordedName: 'Pact', firstHour: 100, lastHour: 300, nameReason: null, names: [{ firstHour: 100, lastHour: 300, name: 'Pact', reason: null, token: null }], incomplete: [],
      members: [run('AAA', 100, 199), run('BBB', 200, 300), run('CCC', 100, 300)], leaders: [run('AAA', 100, 199), run('BBB', 200, 300)] }],
  };
  const [view] = factionViews(renamed, summaryName, id => id);
  assert.equal(view.members.length, 2);
  const lanes = factionLanes([view], 250, { ...options, counts: counted(250, [['1', 2]]) }, text, [], 0).lanes.map(lane => `${lane.identity?.label}: ${lane.label}`);
  assert.deepEqual(lanes, ['Pact: 2 members', `BBB: ${LEADER_GLYPH} Leader · Joined day 100`, 'CCC: Joined day 100']);
  assert.equal(factionViews({ ...renamed, identityChanges: [] }, summaryName, id => id)[0].members.length, 2);
  const [unjoined] = factionViews({ ...renamed, identityChanges: [],
    countryIdentities: [holder('AAA', 1, 100, 199), holder('BBB', 3, 200, 300), holder('CCC', 2, 100, 300)],
  }, summaryName, id => id);
  assert.equal(unjoined.members.length, 3);
});

test('two countries that held one tag at different times are two members with their own lanes', () => {
  const reused: CountryGroupReply = {
    ...reply, identityChanges: [{ hour: 200, from: 'AAA', to: 'BBB' }], unobserved: { humans: [], factions: [], wars: [], unattributed: [] },
    countryIdentities: [{ tag: 'AAA', identities: [...holder('AAA', 1, 100, 199).identities, ...holder('AAA', 2, 400, 450).identities] }, holder('BBB', 1, 200, 450)],
    factions: [{ id: '1', name: 'Pact', recordedName: 'Pact', firstHour: 100, lastHour: 450, nameReason: null, names: [{ firstHour: 100, lastHour: 450, name: 'Pact', reason: null, token: null }], incomplete: [], leaders: [],
      members: [run('AAA', 100, 199), run('BBB', 200, 450), run('AAA', 400, 450)] }],
  };
  const [view] = factionViews(reused, summaryName, id => id);
  assert.deepEqual(view.members.map(member => member.runs.map(item => `${item.tag}@${item.firstHour}`)), [['AAA@100', 'BBB@200'], ['AAA@400']]);
  const { lanes, info } = factionLanes([view], 420, options, text, [], 0);
  const ids = lanes.map(lane => lane.id);
  assert.equal(new Set(ids).size, ids.length);
  assert.deepEqual(lanes.slice(1).map(lane => info.get(lane.id)!.member!.runs[0].firstHour), [100, 400]);
  assert.deepEqual(lanes.slice(1).map(lane => lane.identity?.label), ['BBB', 'AAA']);
});

test('a former member is named by its current alias and manual tags keep every holder', () => {
  const renamed: CountryGroupReply = {
    ...reply, identityChanges: [{ hour: 200, from: 'AAA', to: 'BBB' }], unobserved: { humans: [], factions: [], wars: [], unattributed: [] },
    countryIdentities: [{ tag: 'AAA', identities: [...holder('AAA', 1, 100, 199).identities, ...holder('AAA', 2, 400, 500).identities] }, holder('BBB', 1, 200, 500), holder('CCC', 3)],
    factions: [{ id: '1', name: 'Pact', recordedName: 'Pact', firstHour: 100, lastHour: 500, nameReason: null, names: [{ firstHour: 100, lastHour: 500, name: 'Pact', reason: null, token: null }], incomplete: [], leaders: [],
      members: [run('AAA', 100, 150), run('CCC', 100, 500)] }],
  };
  const [view] = factionViews(renamed, summaryName, id => id);
  const { lanes } = factionLanes([view], 450, options, text, [], 0);
  assert.deepEqual(lanes.map(lane => lane.identity?.label), ['Pact', 'BBB', 'CCC']);
  const identities = countryIdentityIndex(renamed.countryIdentities);
  for (const tag of ['AAA', 'BBB']) assert.equal(factionLanes([view], 450,
    { ...options, countries: new Set([tag]), countryIds: identities.selected([tag]) }, text, [], 0).lanes.length, 3);
});

test('a country chosen before it joins admits no faction yet', () => {
  const chosen: CountryGroupReply = { ...reply, countryIdentities: [holder('A', 1, 0), holder('C', 2, 0)], unobserved: { humans: [], factions: [], wars: [], unattributed: [] },
    factions: [{ ...factions[0], id: '1', firstHour: 0, lastHour: 300, members: [run('A', 0, 300), run('C', 200, 300)], leaders: [], incomplete: [] }] };
  const identities = countryIdentityIndex(chosen.countryIdentities);
  const views = factionViews(chosen, summaryName, id => id);
  const selected = { ...options, countries: new Set(['C']), countryIds: identities.selected(['C']) };
  assert.equal(factionLanes(views, 100, selected, text, [], 0).lanes.length, 0);
  assert.ok(factionLanes(views, 250, selected, text, [], 0).lanes.some(lane => lane.heading));
});

test('manual X includes every native holder and its aliases, while unrelated Y and unknown spans stay excluded', () => {
  const countries = [{ tag: 'X', identities: [...holder('X', 1, 0, 9, 10).identities, ...holder('X', 1, 20, 30, 20).identities] },
    { tag: 'Y', identities: [...holder('Y', 1, 10, 19, 10).identities, ...holder('Y', 2, 20, 30, 30).identities] }];
  const chosen: CountryGroupReply = { ...reply, countryIdentities: countries, unobserved: { humans: [], factions: [], wars: [], unattributed: [] },
    factions: [run('Y', 10, 19), run('X', 20, 30), run('Y', 20, 30), run('X', 15, 16)].map((member, index) => ({
      ...factions[0], id: String(index + 1), firstHour: 0, lastHour: 30, members: [member], leaders: [], incomplete: [],
    })) };
  const filter = worldFilter({ kind: 'tags', tags: ['X'] }, [], chosen, 30);
  const prepared = factionViews(chosen, summaryName, id => id);
  const lanes = factionLanes(prepared, 30, { ...options, countries: filter.tags, countryIds: filter.countryIds }, text, [], 0);
  assert.deepEqual(lanes.lanes.filter(lane => lane.heading).map(lane => lanes.info.get(lane.id)!.faction.id), ['1', '2']);
});

test('a membership continued across unrecorded hours is not said to have ended', () => {
  const gap: CountryGroupReply = {
    ...reply, identityChanges: [], unobserved: { humans: [], factions: [{ firstHour: 200, lastHour: 209 }], wars: [], unattributed: [] },
    countryIdentities: [holder('AAA', 1, 100, 300), holder('CCC', 2, 100, 300)],
    factions: [{ id: '1', name: 'Pact', recordedName: 'Pact', firstHour: 100, lastHour: 300, nameReason: null, names: [{ firstHour: 100, lastHour: 300, name: 'Pact', reason: null, token: null }], incomplete: [],
      members: [run('AAA', 100, 199), run('AAA', 210, 300), run('CCC', 100, 150), run('CCC', 250, 300)], leaders: [] }],
  };
  const [view] = factionViews(gap, summaryName, id => id);
  const member = (tag: string) => view.members.find(item => item.runs[0].tag === tag)!;
  assert.deepEqual(member('AAA').stretches, [{ firstHour: 100, lastHour: 300 }]);
  assert.deepEqual(member('CCC').stretches, [{ firstHour: 100, lastHour: 150 }, { firstHour: 250, lastHour: 300 }]);
  const lanes = factionLanes([view], 300, options, text, gap.unobserved.factions, 0).lanes.map(lane => `${lane.identity?.label}: ${lane.label}`);
  assert.deepEqual(lanes.slice(1), ['AAA: Joined day 100', 'CCC: Joined day 250']);
  assert.equal(membersAt(view, 205), null);
  const during = factionLanes([view], 205, options, text, gap.unobserved.factions, 0).lanes.map(lane => `${lane.identity?.label}: ${lane.label}`);
  assert.deepEqual(during.slice(1), ['AAA: Last recorded day 199', 'CCC: Left day 151'], 'inside the gap, a later reading does not make the membership continuous yet');
});

test('hours the rosters were not recorded after a faction was last seen are not a dissolution, a departure or an empty roster', () => {
  const unrecorded = { ...reply, identityChanges: [], unobserved: { humans: [], factions: [{ firstHour: 10, lastHour: 19 }], wars: [], unattributed: [] },
    countryIdentities: [holder('AAA', 1, 0, 19)],
    factions: [{ id: '1', name: 'Pact', recordedName: 'Pact', firstHour: 0, lastHour: 9, nameReason: null, names: [{ firstHour: 0, lastHour: 9, name: 'Pact', reason: null, token: null }],
      incomplete: [], members: [run('AAA', 0, 9)], leaders: [] }] } as CountryGroupReply;
  assert.deepEqual(factionGaps(unrecorded), [{ firstHour: 10, lastHour: 19 }]);
  const [view] = factionViews(unrecorded, summaryName, id => id);
  assert.equal(membersAt(view, 15), null, 'an unrecorded roster is not known to be empty');
  const lanes = factionLanes([view], 15, options, text, factionGaps(unrecorded), 0).lanes.map(lane => lane.label);
  assert.deepEqual(lanes, ['Last recorded day 9', 'Last recorded day 9'], 'neither the faction dissolved nor its member left at 10');
});

test('the widget renders only when the head passes a faction, name, roster, leader or gap edge', () => {
  const hours = factionChangeHours(reply, views);
  assert.deepEqual(hours, [...hours].sort((a, b) => a - b), 'sorted');
  assert.equal(new Set(hours).size, hours.length, 'each once');
  for (const hour of [100, 120, 150, 250, 300, 350, 351, 401, 200, 220, 260, 270]) assert.ok(hours.includes(hour), `${hour} is a change`);
  for (const hour of [101, 175, 330]) assert.ok(!hours.includes(hour), `${hour} changes nothing`);
  const between = (hour: number) => factionLanes(views, hour, options, text, factionGaps(reply), 0).lanes.map(lane => `${lane.id}|${lane.label}`);
  assert.deepEqual(between(160), between(175), 'between two changes the lanes are the same');
});

test('a same-tag native replacement starts its own membership without a rename event', () => {
  const replaced: CountryGroupReply = { ...reply, identityChanges: [], unobserved: { humans: [], factions: [], wars: [], unattributed: [] },
    countryIdentities: [{ tag: 'X', identities: [...holder('X', 1, 0, 9, 10).identities, ...holder('X', 1, 10, 19, 20).identities] }],
    factions: [{ ...factions[0], id: '1', firstHour: 0, lastHour: 19, members: [run('X', 0, 19)], leaders: [], incomplete: [] }],
  };
  const [view] = factionViews(replaced, summaryName, id => id);
  assert.deepEqual(view.members.map(member => [member.key, member.stretches]), [
    ['[1,10]', [{ firstHour: 0, lastHour: 9 }]], ['[1,20]', [{ firstHour: 10, lastHour: 19 }]],
  ]);
  const layout = factionLanes([view], 19, { ...options, ended: false }, text, [], 0);
  assert.equal(layout.lanes.length, 2);
  assert.equal(layout.lanes[1].label, 'Joined day 10');
  assert.equal(layout.info.get(layout.lanes[1].id)?.member?.key, '[1,20]');
  assert.equal(membersAt(view, 19), 1);
});

test('a native alias across a missing identity hour retains one membership and an unknown look', () => {
  const renamed: CountryGroupReply = { ...reply, identityChanges: [], unobserved: { humans: [], factions: [], wars: [], unattributed: [] },
    countryIdentities: [holder('X', 1, 0, 9, 10), holder('Y', 1, 11, 19, 10)],
    factions: [{ ...factions[0], id: '1', firstHour: 0, lastHour: 19, members: [run('X', 0, 10), run('Y', 11, 19)], leaders: [run('X', 0, 10), run('Y', 11, 19)], incomplete: [] }],
  };
  const [view] = factionViews(renamed, summaryName, id => id);
  assert.equal(view.members.length, 1);
  assert.deepEqual(view.members[0].stretches, [{ firstHour: 0, lastHour: 19 }]);
  assert.deepEqual(view.members[0].runs, [run('X', 0, 9), run('Y', 11, 19)]);
  assert.equal(tagAt(view, view.members[0], 10), null);
  assert.equal(tagAt(view, view.members[0], 15), 'Y');
  assert.equal(membersAt(view, 10), null);
  assert.equal(leaderAt(view.leaders, 10), null);
  const asked: string[] = [];
  const layout = factionLanes([view], 10, options, { ...text, country: tag => { asked.push(tag); return text.country(tag); } }, [], 0);
  assert.deepEqual(asked, []);
  assert.deepEqual(laneCountries(layout.lanes, layout.info, 10), []);
  assert.equal(layout.lanes[1].identity?.label, 'Unknown country');
  assert.equal(layout.lanes[1].identity?.countryFlag?.flagReason, 'Unknown country');
  const later = factionLanes([view], 15, options, text, [], 0);
  assert.equal(later.lanes[1].identity?.label, 'Y');
  assert.equal(later.lanes[1].label, `${LEADER_GLYPH} Leader · Member before day 0`);
  assert.deepEqual(laneCountries(later.lanes, later.info, 15), ['Y']);
});

test('a member with a missing native suffix stays an unknown roster rather than a zero count', () => {
  const [view] = factionViews({ ...reply, countryIdentities: [], unobserved: { humans: [], factions: [], wars: [], unattributed: [] },
    factions: [{ ...factions[0], firstHour: 0, lastHour: 19, members: [run('X', 0, 19)], leaders: [run('X', 0, 19)], incomplete: [] }],
  }, summaryName, id => id);
  assert.equal(view.members.length, 0);
  assert.equal(membersAt(view, 15), null);
  const layout = factionLanes([view], 15, options, text, [], 0);
  assert.equal(layout.lanes[0].label, 'Members not recorded');
  assert.deepEqual(laneCountries(layout.lanes, layout.info, 15), []);
});

test('a world of 1,500 countries in factions lays out a lane per member and faction', () => {
  const world = (size: number): CountryGroupReply => {
    const count = Math.max(1, Math.floor(size / 30));
    const factions: FactionGroup[] = Array.from({ length: count }, (_, faction) => {
      const members = Array.from({ length: Math.ceil((size - faction) / count) }, (_, index) => `F${faction}M${index}`);
      return { id: String(faction + 1), name: null, recordedName: '', firstHour: 0, lastHour: 2000, incomplete: [],
        nameReason: 'Faction template not recorded', names: [{ firstHour: 0, lastHour: 2000, name: null, reason: 'Faction template not recorded', token: null }],
        members: members.flatMap((tag, index) => [run(tag, index, 900), run(tag, 1000 + index, 2000)]),
        leaders: [run(members[0], 0, 900), run(members[1], 1001, 2000)] };
    });
    const tags = [...new Set(factions.flatMap(faction => faction.members.map(member => member.tag)))];
    return { ...reply, factions, countryIdentities: tags.map((tag, slot) => holder(tag, slot, 0, 2000)), unobserved: { humans: [], factions: [], wars: [], unattributed: [] } };
  };
  const big = factionViews(world(1500), summaryName, id => id);
  assert.equal(factionLanes(big, 1500, options, text, [], 0).lanes.length, 1500 + 50);
  const larger = factionViews(world(5000), summaryName, id => id);
  assert.equal(factionLanes(larger, 1500, options, text, [], 0).lanes.length, 5000 + Math.floor(5000 / 30));
});

test('heading, band and hover labels use their own hour after a template rename', () => {
  const faction = { ...factions[0], name: 'Later pact', nameReason: null, names: [
    { firstHour: 100, lastHour: 199, name: 'Opening pact', reason: null, token: null },
    { firstHour: 210, lastHour: 299, name: null, reason: 'Template missing from content', token: null },
    { firstHour: 300, lastHour: 400, name: 'Later pact', reason: null, token: null },
  ] };
  const data = { ...reply, factions: [faction], unobserved: { ...reply.unobserved, factions: [] } };
  const [view] = factionViews(data, factionNameAt, id => id);
  const heading = (hour: number) => factionLanes([view], hour, options, text, [], 100).lanes[0];
  assert.equal(heading(150).identity?.label, 'Opening pact');
  assert.equal(heading(150).spans[0].label, 'Opening pact');
  assert.equal(heading(205).identity?.label, UNNAMED);
  assert.equal(heading(320).identity?.label, 'Later pact');
  assert.equal(factionLabel(view, 150), 'Opening pact');
  assert.equal(factionLabel(view, 205), UNNAMED);
  assert.equal(view.nameAt(205).reason, 'Not recorded');
  assert.equal(view.nameAt(250).reason, 'Template missing from content');
  const [renamed] = factionViews(data, (faction, hour) => factionNameAt(faction, hour, 'My pact'), id => id);
  assert.equal(factionLabel(renamed, 205), 'My pact');
  assert.equal(renamed.nameAt(205).reason, null);
});



test('member lanes retain native flag refusals while faction headings stay flagless', () => {
  const labels = { ...text, country: (tag: string) => ({ name: tag, flagPending: false, flagReason: `Recorded refusal for ${tag}` }) };
  const lanes = factionLanes(views, 200, options, labels, reply.unobserved.factions, 100).lanes;
  assert.ok(lanes.some(lane => lane.depth === 1));
  for (const lane of lanes) {
    if (lane.depth === 1) assert.equal(lane.identity?.countryFlag?.flagReason, `Recorded refusal for ${lane.identity?.label}`);
    else assert.equal(lane.identity?.countryFlag, undefined);
  }
});
