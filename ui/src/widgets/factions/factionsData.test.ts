import test from 'node:test';
import assert from 'node:assert/strict';
import type { CountryGroupReply, FactionGroup } from '../../generated/dashboard';
import { LEADER_GLYPH, UNNAMED, factionLanes, factionViews, laneCountries, leaderAt, leadershipParts, membersAt, membersHeld } from './factionsData.ts';
import type { LaneOptions } from './factionsData';
import { assertLinear } from '../../charts/assertLinear.ts';
import { worldFilter } from '../../dashboard/groups.ts';
import { laneRequests } from '../../charts/timeline/laneRequests.ts';

const run = (tag: string, firstHour: number, lastHour: number) => ({ tag, firstHour, lastHour });

/** A modded campaign: a template faction with no recorded name led by a dynamic
 * country, and a faction a mod names, founded and dissolved partway through, whose
 * members' tags hold a space and a colon. */
const factions: FactionGroup[] = [
  { id: '4294967384', name: null, recordedName: '', firstHour: 100, lastHour: 400,
    members: [run('D01', 100, 400), run('ITA', 150, 249), run('ITA', 300, 400), run('HUN', 120, 400)],
    leaders: [run('D01', 100, 299), run('HUN', 300, 400)], incomplete: [{ firstHour: 350, lastHour: 350 }] },
  { id: '25769803864', name: 'Pact of Steel Reforged', recordedName: 'Pact of Steel Reforged', firstHour: 200, lastHour: 299,
    members: [run('MY TAG', 200, 299), run('A:B', 220, 299)], leaders: [run('MY TAG', 200, 299)], incomplete: [] },
];
const reply: CountryGroupReply = {
  branchId: '1', humans: [], factions, wars: [], warStarts: [], capitulations: [], identityChanges: [],
  unobserved: { humans: [], factions: [{ firstHour: 260, lastHour: 269 }], wars: [] }, warnings: [],
};
const names: Record<string, string> = { D01: 'Free Republic', ITA: 'Italy', HUN: 'Hungary', 'MY TAG': 'Modded Realm', 'A:B': 'Colon Land' };
const text = { country: (tag: string) => ({ name: names[tag] ?? tag, flag: `flag:${tag}` }), day: (hour: number) => `day ${hour}` };
const options: LaneOptions = { order: 'founded', members: 'joined', ended: true, countries: null };
const views = factionViews(reply, faction => faction.name, id => `colour-${id}`);

test('a late recorded name outside the window moves its member from Zulu to Aardvark', () => {
  const tags = Array.from({ length: 50 }, (_, index) => `T${index}`);
  const metadata = new Map(tags.map((tag, index) => [tag, index === 49 ? 'Zulu' : `Member ${String(index).padStart(2, '0')}`]));
  const recorded = new Map<string, string>();
  const labels = { ...text, country: (tag: string) => ({ name: recorded.get(tag) ?? metadata.get(tag) ?? tag }) };
  const views = factionViews({ ...reply, identityChanges: [], unobserved: { humans: [], factions: [], wars: [] },
    factions: [{ id: '1', name: 'Pact', recordedName: 'Pact', firstHour: 0, lastHour: 20, incomplete: [], leaders: [],
      members: tags.map(tag => run(tag, 0, 20)) }],
  }, faction => faction.name, id => id);
  // By name, and joined order, whose founding members tie on the hour and sort by name.
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
    identityChanges: [{ hour: 10, from: 'AAA', to: 'BBB' }], unobserved: { humans: [], factions: [], wars: [] },
    factions: [{ id: '1', name: 'Pact', recordedName: 'Pact', firstHour: 0, lastHour: 20, members: humans, leaders: [], incomplete: [] }],
  };
  const views = factionViews(renamed, faction => faction.name, id => id);
  const filter = worldFilter({ kind: 'group', group: { type: 'humans' }, membership: 'end', aggregate: false }, [], renamed, 5, 20, views[0].lineage);
  const lanes = factionLanes(views, 5, { ...options, countries: filter.tags, countryIds: filter.countryIds }, text, [], 0).lanes;
  assert.deepEqual(lanes.map(lane => lane.identity?.label), ['Pact', 'AAA']);
});

test('after members swap tags and one leaves, the remaining member has the recorded tag', () => {
  const swapped: CountryGroupReply = { ...reply,
    identityChanges: [{ hour: 10, from: 'AAA', to: 'BBB' }, { hour: 10, from: 'BBB', to: 'AAA' }],
    unobserved: { humans: [], factions: [], wars: [] },
    factions: [{ id: '1', name: 'Pact', recordedName: 'Pact', firstHour: 0, lastHour: 20, incomplete: [], leaders: [],
      members: [run('AAA', 0, 20), run('BBB', 0, 14)] }],
  };
  const [view] = factionViews(swapped, faction => faction.name, id => id);
  assert.deepEqual(membersHeld(view, 5).sort(), ['AAA', 'BBB']);
  assert.deepEqual(membersHeld(view, 16), ['AAA']);
});

test('factions keep recorded ids, colours and names, and count members over known hours only', () => {
  assert.deepEqual(views.map(view => [view.id, view.name, view.color]), [['4294967384', null, 'colour-4294967384'], ['25769803864', 'Pact of Steel Reforged', 'colour-25769803864']]);
  const [first, second] = views;
  // Existence stops for the hours no faction was recorded.
  assert.deepEqual(second.existence, [{ firstHour: 200, lastHour: 259 }, { firstHour: 270, lastHour: 299 }]);
  assert.equal(membersAt(first, 130), 2);
  assert.equal(membersAt(first, 200), 3);
  // The hour its roster named an unknown country, and hours not recorded, are unknown, not empty.
  assert.equal(membersAt(first, 350), null);
  assert.equal(membersAt(first, 265), null);
  assert.equal(leaderAt(first.leaders, 310), 'HUN');
  assert.deepEqual(leadershipParts(run('D01', 100, 400), first.leaders), [{ firstHour: 100, lastHour: 299, leader: true }, { firstHour: 300, lastHour: 400, leader: false }]);
});

test('lanes list each faction founded by the hour, then its members, the leader marked', () => {
  const early = factionLanes(views, 180, options, text, reply.unobserved.factions, 100);
  // The second faction is founded at 200: not yet listed.
  assert.deepEqual(early.lanes.map(lane => lane.identity?.label), [UNNAMED, 'Free Republic', 'Hungary', 'Italy']);
  assert.equal(early.lanes[0].heading, true);
  assert.equal(early.lanes[0].label, 'Led by Free Republic · 3 members');
  assert.equal(early.lanes[1].label, `${LEADER_GLYPH} Leader · Member before day 100`);
  assert.equal(early.lanes[3].label, 'Joined day 150');
  assert.equal(early.lanes[1].spans[0].glyph, LEADER_GLYPH);
  // The heading's band follows the member count.
  assert.deepEqual(early.lanes[0].spans[0].band?.slice(0, 3).map(step => step.value), [1, 2, 3]);
  const later = factionLanes(views, 320, options, text, reply.unobserved.factions, 100);
  const labels = later.lanes.map(lane => `${lane.identity?.label}: ${lane.label}`);
  assert.ok(labels.includes('Pact of Steel Reforged: Dissolved day 300'), labels.join(' | '));
  assert.ok(labels.includes(`Hungary: ${LEADER_GLYPH} Leader · Joined day 120`), labels.join(' | '));
  assert.ok(labels.includes('Colon Land: Left day 300'), labels.join(' | '));
  // Lane ids never blur tags holding a space or a colon.
  const ids = later.lanes.map(lane => lane.id);
  assert.equal(new Set(ids).size, ids.length);
  assert.ok(later.lanes.some(lane => later.info.get(lane.id)?.member?.runs[0].tag === 'A:B'));
  assert.deepEqual(laneCountries(later.lanes, later.info, 320).sort(), ['A:B', 'D01', 'HUN', 'ITA', 'MY TAG']);
});

test('dissolved factions and former members can be hidden, and chosen countries narrow the factions', () => {
  const hidden = factionLanes(views, 320, { ...options, ended: false }, text, reply.unobserved.factions, 100);
  assert.deepEqual(hidden.lanes.map(lane => lane.identity?.label), [UNNAMED, 'Free Republic', 'Hungary', 'Italy']);
  const chosen = factionLanes(views, 320, { ...options, countries: new Set(['MY TAG']) }, text, reply.unobserved.factions, 100);
  assert.deepEqual(chosen.lanes.map(lane => lane.identity?.label), ['Pact of Steel Reforged', 'Modded Realm', 'Colon Land']);
  const byMembers = factionLanes(views, 280, { ...options, order: 'members' }, text, reply.unobserved.factions, 100);
  assert.deepEqual(byMembers.lanes.filter(lane => lane.heading).map(lane => lane.identity?.label), [UNNAMED, 'Pact of Steel Reforged']);
  const byName = factionLanes(views, 280, { ...options, order: 'name', members: 'name' }, text, reply.unobserved.factions, 100);
  assert.deepEqual(byName.lanes.map(lane => lane.identity?.label),
    ['Pact of Steel Reforged', 'Colon Land', 'Modded Realm', UNNAMED, 'Free Republic', 'Hungary', 'Italy']);
});

test('a member whose tag changes stays one member, neither leaving nor joining', () => {
  const renamed: CountryGroupReply = {
    ...reply, identityChanges: [{ hour: 200, from: 'AAA', to: 'BBB' }], unobserved: { humans: [], factions: [], wars: [] },
    factions: [{ id: '1', name: 'Pact', recordedName: 'Pact', firstHour: 100, lastHour: 300, incomplete: [],
      members: [run('AAA', 100, 199), run('BBB', 200, 300), run('CCC', 100, 300)], leaders: [run('AAA', 100, 199), run('BBB', 200, 300)] }],
  };
  const [view] = factionViews(renamed, faction => faction.name, id => id);
  assert.equal(view.members.length, 2);
  const lanes = factionLanes([view], 250, options, text, [], 0).lanes.map(lane => `${lane.identity?.label}: ${lane.label}`);
  // Named by its tag at the hour, a member since its first run, still leading.
  assert.deepEqual(lanes, ['Pact: Led by BBB · 2 members', `BBB: ${LEADER_GLYPH} Leader · Joined day 100`, 'CCC: Joined day 100']);
  // Without the change, AAA would have left at 200 and BBB joined then.
  const [unjoined] = factionViews({ ...renamed, identityChanges: [] }, faction => faction.name, id => id);
  assert.equal(unjoined.members.length, 3);
});

test('two countries that held one tag at different times are two members with their own lanes', () => {
  // AAA is a member 100-199, becomes BBB at 200 and stays to 450; another AAA joins at 400.
  const reused: CountryGroupReply = {
    ...reply, identityChanges: [{ hour: 200, from: 'AAA', to: 'BBB' }], unobserved: { humans: [], factions: [], wars: [] },
    factions: [{ id: '1', name: 'Pact', recordedName: 'Pact', firstHour: 100, lastHour: 450, incomplete: [], leaders: [],
      members: [run('AAA', 100, 199), run('BBB', 200, 450), run('AAA', 400, 450)] }],
  };
  const [view] = factionViews(reused, faction => faction.name, id => id);
  assert.deepEqual(view.members.map(member => member.runs.map(item => `${item.tag}@${item.firstHour}`)), [['AAA@100', 'BBB@200'], ['AAA@400']]);
  const { lanes, info } = factionLanes([view], 420, options, text, [], 0);
  const ids = lanes.map(lane => lane.id);
  assert.equal(new Set(ids).size, ids.length);
  // Neither overwrites the other: each lane is about its own member.
  assert.deepEqual(lanes.slice(1).map(lane => info.get(lane.id)!.member!.runs[0].firstHour), [100, 400]);
  assert.deepEqual(lanes.slice(1).map(lane => lane.identity?.label), ['BBB', 'AAA']);
});

test('a former member renamed after it left is named, and chosen, by its tags then', () => {
  // AAA leaves at 150 and becomes BBB at 200 without rejoining; a new AAA appears at 400 (not a member).
  const renamed: CountryGroupReply = {
    ...reply, identityChanges: [{ hour: 200, from: 'AAA', to: 'BBB' }], unobserved: { humans: [], factions: [], wars: [] },
    factions: [{ id: '1', name: 'Pact', recordedName: 'Pact', firstHour: 100, lastHour: 500, incomplete: [], leaders: [],
      members: [run('AAA', 100, 150), run('CCC', 100, 500)] }],
  };
  const [view] = factionViews(renamed, faction => faction.name, id => id);
  const { lanes } = factionLanes([view], 450, options, text, [], 0);
  // Its lane shows BBB at 450, never the new AAA's name and flag.
  assert.deepEqual(lanes.map(lane => lane.identity?.label), ['Pact', 'BBB', 'CCC']);
  // Choosing BBB finds the faction it belonged to as AAA; choosing the new AAA does not.
  assert.equal(factionLanes([view], 450, { ...options, countries: new Set(['BBB']) }, text, [], 0).lanes.length, 3);
  assert.equal(factionLanes([view], 450, { ...options, countries: new Set(['AAA']) }, text, [], 0).lanes.length, 0);
});

test('a membership continued across unrecorded hours is not said to have ended', () => {
  const gap: CountryGroupReply = {
    ...reply, identityChanges: [], unobserved: { humans: [], factions: [{ firstHour: 200, lastHour: 209 }], wars: [] },
    factions: [{ id: '1', name: 'Pact', recordedName: 'Pact', firstHour: 100, lastHour: 300, incomplete: [],
      members: [run('AAA', 100, 199), run('AAA', 210, 300), run('CCC', 100, 150), run('CCC', 250, 300)], leaders: [] }],
  };
  const [view] = factionViews(gap, faction => faction.name, id => id);
  const member = (tag: string) => view.members.find(item => item.runs[0].tag === tag)!;
  assert.deepEqual(member('AAA').stretches, [{ firstHour: 100, lastHour: 300 }]);
  // CCC was recorded out of the faction between 151 and 249: it left and joined again.
  assert.deepEqual(member('CCC').stretches, [{ firstHour: 100, lastHour: 150 }, { firstHour: 250, lastHour: 300 }]);
  const lanes = factionLanes([view], 300, options, text, gap.unobserved.factions, 0).lanes.map(lane => `${lane.identity?.label}: ${lane.label}`);
  assert.deepEqual(lanes.slice(1), ['AAA: Joined day 100', 'CCC: Joined day 250']);
  // The faction's count has a gap at the unrecorded hours rather than a drop.
  assert.equal(membersAt(view, 205), null);
});

test('a world of 1,500 countries in factions lays out in about linear time', () => {
  // `size` members over size / 30 factions, each joining, leaving and joining again, the leader changing once.
  const world = (size: number): CountryGroupReply => {
    const count = Math.max(1, size / 30);
    const factions: FactionGroup[] = Array.from({ length: count }, (_, faction) => {
      const members = Array.from({ length: size / count }, (_, index) => `F${faction}M${index}`);
      return { id: String(faction + 1), name: null, recordedName: '', firstHour: 0, lastHour: 2000, incomplete: [],
        members: members.flatMap((tag, index) => [run(tag, index, 900), run(tag, 1000 + index, 2000)]),
        leaders: [run(members[0], 0, 900), run(members[1], 1001, 2000)] };
    });
    return { ...reply, factions, unobserved: { humans: [], factions: [], wars: [] } };
  };
  const big = factionViews(world(1500), faction => faction.name, id => id);
  assert.equal(factionLanes(big, 1500, options, text, [], 0).lanes.length, 1500 + 50);
  assertLinear('members', size => {
    const views = factionViews(world(size), faction => faction.name, id => id);
    return () => factionLanes(views, 1500, options, text, [], 0);
  }, 1500);
});
