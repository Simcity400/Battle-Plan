import test from 'node:test';
import assert from 'node:assert/strict';
import type { CountryGroupReply, SubjectLevel, SubjectReply } from '../../generated/dashboard';
import { changeHours, laneCountries, levelColours, levelName, overlordTagAt, overlordViews, subjectLanes, subjectTagAt, validateSubjectReply } from './subjectsData.ts';
import { Lineage } from '../../dashboard/identity.ts';
import type { LaneOptions } from './subjectsData';
import { worldFilter } from '../../dashboard/groups.ts';
import { laneRequests } from '../../charts/timeline/laneRequests.ts';
import { changesBy } from '../../charts/timeline/runState.ts';
import { countryIdentityIndex } from '../../dashboard/countryIdentity.ts';

const levels: SubjectLevel[] = [
  { token: 'autonomy_mod_march', name: 'March', ordered: true },
  { token: 'autonomy_puppet', name: 'Puppet', ordered: true },
  { token: 'autonomy_mod_unnamed', name: null, ordered: true },
  { token: 'autonomy_mod_unordered', name: 'Unordered', ordered: false },
];
const step = (firstHour: number, lastHour: number, level: string | null) => ({ firstHour, lastHour, level });
const reply: SubjectReply = {
  branchId: '1', levels, warnings: [],
  unobserved: [{ firstHour: 300, lastHour: 309 }],
  runs: [
    { overlord: 'D01', subject: 'MY TAG', startHour: 50, firstHour: 100, lastHour: 199,
      steps: [step(100, 149, 'autonomy_mod_march'), step(150, 199, 'autonomy_puppet')] },
    { overlord: 'D01', subject: 'MY TAG', startHour: 250, firstHour: 250, lastHour: 299, steps: [step(250, 299, 'autonomy_mod_unnamed')] },
    { overlord: 'D01', subject: 'A:B', startHour: 120, firstHour: 120, lastHour: 299, steps: [step(120, 299, null)] },
    { overlord: 'ENG', subject: 'CAN', startHour: 10, firstHour: 100, lastHour: 400, steps: [step(100, 400, 'autonomy_puppet')] },
    { overlord: 'D01', subject: 'A:B', startHour: 120, firstHour: 310, lastHour: 400, steps: [step(310, 400, 'autonomy_mod_unordered')] },
  ],
};

const names: Record<string, string> = { D01: 'Free Republic', 'MY TAG': 'Modded Realm', 'A:B': 'Colon Land', ENG: 'United Kingdom', CAN: 'Canada' };
const text = { country: (tag: string) => ({ name: names[tag] ?? tag }), day: (hour: number) => `day ${hour}` };
const options: LaneOptions = { order: 'first', subjects: 'since', ended: true, countries: null };
const byToken = new Map(levels.map(level => [level.token, level]));
const holder = (tag: string, slot: number, firstHour = 0, lastHour = 500, countryId = slot) => ({ tag, identities: [{ slot, countryId, firstHour, lastHour }] });
const identities = countryIdentityIndex(['D01', 'MY TAG', 'A:B', 'ENG', 'CAN'].map((tag, slot) => holder(tag, slot)));

test('a late recorded name outside the window reorders both overlords and subjects', () => {
  for (const field of ['subject', 'overlord'] as const) {
    const tags = Array.from({ length: 50 }, (_, index) => `T${index}`);
    const metadata = new Map(tags.map((tag, index) => [tag, index === 49 ? 'Zulu' : `Country ${String(index).padStart(2, '0')}`]));
    const recorded = new Map<string, string>();
    const labels = { ...text, country: (tag: string) => ({ name: recorded.get(tag) ?? metadata.get(tag) ?? tag }) };
    const views = overlordViews({ ...reply, unobserved: [], runs: tags.map((tag, index) => ({
      overlord: field === 'overlord' ? tag : 'OVR', subject: field === 'subject' ? tag : `S${index}`,
      startHour: 0, firstHour: 0, lastHour: 20, steps: [step(0, 20, null)],
    })) });
    const layout = () => subjectLanes(views, byToken, new Map(), 20,
      { ...options, order: field === 'overlord' ? 'name' : 'first', subjects: field === 'subject' ? 'name' : 'since' }, labels, []);
    const before = layout();
    const requests = laneRequests(before.lanes, [0, 24]);
    assert.ok(!laneCountries(requests.looks, before.info, 20).includes('T49'), field);
    const wanted = laneCountries(requests.names, before.info, 20);
    if (wanted.includes('T49')) recorded.set('T49', 'Aardvark');
    assert.equal(layout().lanes[field === 'overlord' ? 0 : 1].identity?.label, 'Aardvark', field);
  }
});

test('end membership keeps the human subject’s overlord visible before its change of tag', () => {
  const groups: CountryGroupReply = { branchId: '1', warnings: [], factions: [], wars: [], warStarts: [], capitulations: [],
    countryIdentities: [holder('AAA', 1, 0, 9), holder('BBB', 1, 10, 20), holder('OVR', 2)],
    identityChanges: [{ hour: 10, from: 'AAA', to: 'BBB' }], unobserved: { humans: [], factions: [], wars: [] },
    humans: [{ tag: 'AAA', firstHour: 0, lastHour: 9 }, { tag: 'BBB', firstHour: 10, lastHour: 20 }],
  };
  const views = overlordViews({ ...reply, unobserved: [], runs: [
    { overlord: 'OVR', subject: 'AAA', startHour: 0, firstHour: 0, lastHour: 9, steps: [step(0, 9, null)] },
    { overlord: 'OVR', subject: 'BBB', startHour: 0, firstHour: 10, lastHour: 20, steps: [step(10, 20, null)] },
  ] }, new Lineage(groups.identityChanges), countryIdentityIndex(groups.countryIdentities));
  const filter = worldFilter({ kind: 'group', group: { type: 'humans' }, membership: 'end', aggregate: false }, [], groups, 5, 20);
  const lanes = subjectLanes(views, byToken, new Map(), 5, { ...options, countries: filter.tags, countryIds: filter.countryIds }, text, []).lanes;
  assert.deepEqual(lanes.map(lane => lane.identity?.label), ['OVR', 'AAA']);
});

test('a relation spanning a swap follows each country and clips its level steps', () => {
  const lineage = new Lineage([{ hour: 10, from: 'AAA', to: 'BBB' }, { hour: 10, from: 'BBB', to: 'AAA' }]);
  for (const field of ['subject', 'overlord'] as const) {
    const swapped: SubjectReply = { ...reply, unobserved: [], runs: ['AAA', 'BBB'].map((tag, index) => ({
      overlord: field === 'overlord' ? tag : 'OVR', subject: field === 'subject' ? tag : 'SUB',
      startHour: 0, firstHour: 0, lastHour: index ? 14 : 20,
      steps: [step(0, 11, 'autonomy_puppet'), step(12, index ? 14 : 20, 'autonomy_mod_march')],
    })) };
    const views = overlordViews(swapped, lineage);
    const held = views.flatMap(overlord => overlord.subjects.flatMap(subject => subject.runs
      .filter(run => run.firstHour <= 16 && 16 <= run.lastHour)
      .map(() => lineage.tagOf(field === 'overlord' ? overlord.key : subject.subject, 16))));
    assert.deepEqual(held, ['AAA'], field);
    for (const overlord of views) for (const subject of overlord.subjects) for (const run of subject.runs) {
      assert.equal(run.steps[0].firstHour, run.firstHour);
      assert.equal(run.steps.at(-1)!.lastHour, run.lastHour);
    }
  }
});

test('a subjects reply is validated, down to steps covering their run and naming listed levels', () => {
  assert.equal(validateSubjectReply(reply).runs.length, 5);
  assert.throws(() => validateSubjectReply({ ...reply, unobserved: [{ firstHour: 0, lastHour: Number.MAX_SAFE_INTEGER + 1 }] }), /malformed/);
  const broken = structuredClone(reply);
  broken.runs[0].steps[1].firstHour = 151;
  assert.throws(() => validateSubjectReply(broken), /malformed/);
  const unlisted = structuredClone(reply);
  unlisted.runs[0].steps[0].level = 'autonomy_not_listed';
  assert.throws(() => validateSubjectReply(unlisted), /malformed/);
  assert.throws(() => validateSubjectReply({ ...reply, levels: undefined }), /malformed/);
});

test('level colours follow the content order, never a level name, and fold past the palette', () => {
  const colours = levelColours(levels, ['c0', 'c1', 'c2'], 'other');
  assert.deepEqual([...colours], [['autonomy_mod_march', 'c0'], ['autonomy_puppet', 'c1'], ['autonomy_mod_unnamed', 'c2'], ['autonomy_mod_unordered', 'other']]);
  assert.equal(levelName(byToken, 'autonomy_mod_unnamed'), 'Autonomy level name unavailable');
  assert.equal(levelName(byToken, null), 'Level not recorded');
});

test('each overlord heads its subjects, dated as the game dates them, coloured by level', () => {
  const views = overlordViews(reply);
  const colours = levelColours(levels, ['c0', 'c1', 'c2', 'c3'], 'other');
  const { lanes, info } = subjectLanes(views, byToken, colours, 160, options, text, reply.unobserved);
  assert.deepEqual(lanes.map(lane => [lane.identity?.label, lane.label]), [
    ['United Kingdom', '1 subject'], ['Canada', 'Puppet · Since day 10'],
    ['Free Republic', '2 subjects'], ['Modded Realm', 'Puppet · Since day 50'], ['Colon Land', 'Level not recorded · Since day 120'],
  ]);
  assert.equal(lanes[0].heading, true);
  assert.equal(lanes[1].depth, 1);
  assert.deepEqual(lanes[2].spans[0].band?.map(item => [item.firstHour, item.value]).slice(0, 2), [[100, 1], [120, 2]]);
  const realm = lanes[3].spans;
  assert.deepEqual(realm.map(span => [span.firstHour, span.label, span.color]), [[100, 'March', 'c0'], [150, 'Puppet', 'c1'], [250, 'Autonomy level name unavailable', 'c2']]);
  assert.equal(lanes[4].spans[0].color, undefined);
  assert.ok(lanes.some(lane => info.get(lane.id)?.subject?.runs[0].subject === 'A:B'));
  assert.deepEqual(laneCountries(lanes, info, 160).sort(), ['A:B', 'CAN', 'D01', 'ENG', 'MY TAG']);
});

test('ended and unrecorded stretches are said honestly, and can be hidden or narrowed', () => {
  const views = overlordViews(reply, new Lineage([]), identities);
  const colours = levelColours(levels, ['c0', 'c1', 'c2', 'c3'], 'other');
  const at = (hour: number, extra: Partial<LaneOptions> = {}) => subjectLanes(views, byToken, colours, hour, { ...options, ...extra }, text, reply.unobserved)
    .lanes.map(lane => `${lane.identity?.label}: ${lane.label}`);
  assert.ok(at(220).includes('Modded Realm: Ended day 200'));
  assert.ok(at(260).includes('Modded Realm: Autonomy level name unavailable · Since day 250'));
  assert.ok(at(305).includes('Modded Realm: Last recorded day 299'));
  assert.ok(at(320).includes('Modded Realm: Ended by day 310'));
  assert.ok(at(305).includes('Free Republic: Subjects not recorded'));
  assert.ok(!at(320, { ended: false }).some(label => label.startsWith('Modded Realm')));
  assert.deepEqual(at(320, { countries: new Set(['CAN']), countryIds: identities.selected(['CAN']) }), ['United Kingdom: 1 subject', 'Canada: Puppet · Since day 10']);
  assert.deepEqual(at(320, { order: 'name', subjects: 'name' }).filter(label => !label.includes('·')).slice(0, 1), ['Free Republic: 1 subject']);
});

test('an unrecorded hour is a gap in the overlord band, not a drop in its subjects', () => {
  const partial: SubjectReply = { ...reply, unobserved: [{ firstHour: 101, lastHour: 101 }], runs: [
    { overlord: 'D01', subject: 'AAA', startHour: 50, firstHour: 100, lastHour: 200, steps: [step(100, 200, 'autonomy_puppet')] },
    { overlord: 'D01', subject: 'BBB', startHour: 50, firstHour: 100, lastHour: 100, steps: [step(100, 100, 'autonomy_puppet')] },
    { overlord: 'D01', subject: 'BBB', startHour: 50, firstHour: 102, lastHour: 200, steps: [step(102, 200, 'autonomy_puppet')] },
  ] };
  const [overlord] = overlordViews(partial);
  assert.deepEqual(overlord.counts.map(item => [item.firstHour, item.lastHour, item.value]), [[100, 100, 2], [102, 200, 2]]);
  assert.equal(overlord.subjects.find(view => view.runs[0].subject === 'BBB')!.stretches.length, 1);
});

test('a relation continued across an unrecorded stretch is still held; a new start is a new relation', () => {
  const gap: SubjectReply = { ...reply, unobserved: [{ firstHour: 200, lastHour: 209 }], runs: [
    { overlord: 'D01', subject: 'AAA', startHour: 50, firstHour: 100, lastHour: 199, steps: [step(100, 199, 'autonomy_puppet')] },
    { overlord: 'D01', subject: 'AAA', startHour: 50, firstHour: 210, lastHour: 300, steps: [step(210, 300, 'autonomy_puppet')] },
    { overlord: 'D01', subject: 'BBB', startHour: 60, firstHour: 100, lastHour: 199, steps: [step(100, 199, 'autonomy_puppet')] },
    { overlord: 'D01', subject: 'BBB', startHour: 205, firstHour: 210, lastHour: 300, steps: [step(210, 300, 'autonomy_puppet')] },
  ] };
  const [overlord] = overlordViews(gap);
  const view = (tag: string) => overlord.subjects.find(item => item.runs[0].subject === tag)!;
  assert.deepEqual(view('AAA').stretches.map(item => [item.firstHour, item.lastHour]), [[100, 300]]);
  assert.deepEqual(view('BBB').stretches.map(item => [item.firstHour, item.lastHour]), [[100, 199], [210, 300]]);
  const colours = levelColours(levels, ['c0', 'c1'], 'other');
  const labels = subjectLanes([overlord], byToken, colours, 300, options, text, gap.unobserved).lanes.map(lane => `${lane.identity?.label}: ${lane.label}`);
  assert.deepEqual(labels.slice(1), ['AAA: Puppet · Since day 50', 'BBB: Puppet · Since day 205']);
});

test('a change of tag of a subject or its overlord keeps one lane and one overlord', () => {
  const renamed: SubjectReply = { ...reply, unobserved: [], runs: [
    { overlord: 'D01', subject: 'AAA', startHour: 50, firstHour: 100, lastHour: 199, steps: [step(100, 199, 'autonomy_puppet')] },
    { overlord: 'D01', subject: 'BBB', startHour: 50, firstHour: 200, lastHour: 249, steps: [step(200, 249, 'autonomy_puppet')] },
    { overlord: 'D01', subject: 'CCC', startHour: 70, firstHour: 100, lastHour: 249, steps: [step(100, 249, 'autonomy_colony')] },
    { overlord: 'D02', subject: 'BBB', startHour: 50, firstHour: 250, lastHour: 300, steps: [step(250, 300, 'autonomy_puppet')] },
    { overlord: 'D02', subject: 'CCC', startHour: 70, firstHour: 250, lastHour: 300, steps: [step(250, 300, 'autonomy_colony')] },
  ] };
  const changes = new Lineage([{ hour: 200, from: 'AAA', to: 'BBB' }, { hour: 250, from: 'D01', to: 'D02' }]);
  const views = overlordViews(renamed, changes);
  assert.equal(views.length, 1);
  assert.equal(views[0].subjects.length, 2);
  for (const subject of views[0].subjects) assert.equal(subject.stretches.length, 1);
  assert.equal(overlordTagAt(views[0], 120), 'D01');
  assert.equal(overlordTagAt(views[0], 280), 'D02');
  const colours = levelColours(levels, ['c0', 'c1'], 'other');
  const labels = subjectLanes(views, byToken, colours, 280, options, text, []).lanes.map(lane => `${lane.identity?.label}: ${lane.label}`);
  assert.deepEqual(labels, ['D02: 2 subjects', 'BBB: Puppet · Since day 50', 'CCC: Autonomy level name unavailable · Since day 70']);
  assert.equal(overlordViews(renamed).length, 2);
});

test('a tag an overlord gave up and another country later took is another overlord', () => {
  const reused: SubjectReply = { ...reply, unobserved: [], runs: [
    { overlord: 'AAA', subject: 'S1', startHour: 50, firstHour: 100, lastHour: 199, steps: [step(100, 199, 'autonomy_puppet')] },
    { overlord: 'BBB', subject: 'S1', startHour: 50, firstHour: 200, lastHour: 500, steps: [step(200, 500, 'autonomy_puppet')] },
    { overlord: 'AAA', subject: 'S2', startHour: 400, firstHour: 400, lastHour: 500, steps: [step(400, 500, 'autonomy_puppet')] },
  ] };
  const views = overlordViews(reused, new Lineage([{ hour: 200, from: 'AAA', to: 'BBB' }]));
  assert.equal(views.length, 2);
  const byTag = (tag: string) => views.find(view => overlordTagAt(view, 450) === tag)!;
  assert.deepEqual(byTag('BBB').subjects.map(view => view.runs.map(run => run.subject)), [['S1', 'S1']]);
  assert.deepEqual(byTag('AAA').subjects.map(view => view.runs.map(run => run.subject)), [['S2']]);
  const colours = levelColours(levels, ['c0'], 'other');
  const ids = subjectLanes(views, byToken, colours, 450, options, text, []).lanes.map(lane => lane.id);
  assert.equal(new Set(ids).size, ids.length);
});

test('a subject released by a country later renamed is not merged into whoever takes the old tag', () => {
  const released: SubjectReply = { ...reply, unobserved: [], runs: [
    { overlord: 'AAA', subject: 'S1', startHour: 50, firstHour: 100, lastHour: 199, steps: [step(100, 199, 'autonomy_puppet')] },
    { overlord: 'AAA', subject: 'S2', startHour: 60, firstHour: 100, lastHour: 150, steps: [step(100, 150, 'autonomy_puppet')] },
    { overlord: 'BBB', subject: 'S1', startHour: 50, firstHour: 200, lastHour: 500, steps: [step(200, 500, 'autonomy_puppet')] },
    { overlord: 'AAA', subject: 'S2', startHour: 400, firstHour: 400, lastHour: 500, steps: [step(400, 500, 'autonomy_puppet')] },
  ] };
  const views = overlordViews(released, new Lineage([{ hour: 200, from: 'AAA', to: 'BBB' }]));
  const byTag = (tag: string) => views.find(view => overlordTagAt(view, 450) === tag)!;
  assert.equal(views.length, 2);
  assert.deepEqual(byTag('BBB').subjects.map(view => view.runs.map(run => `${run.overlord}/${run.subject}@${run.firstHour}`)).sort(),
    [['AAA/S1@100', 'BBB/S1@200'], ['AAA/S2@100']]);
  assert.deepEqual(byTag('AAA').subjects.map(view => view.runs.map(run => `${run.overlord}/${run.subject}@${run.firstHour}`)), [['AAA/S2@400']]);
});

test('a former subject is named by its current alias and manual tags keep every holder', () => {
  const released: SubjectReply = { ...reply, unobserved: [], runs: [
    { overlord: 'OVR', subject: 'S1', startHour: 50, firstHour: 100, lastHour: 150, steps: [step(100, 150, 'autonomy_puppet')] },
    { overlord: 'OVR', subject: 'KEPT', startHour: 50, firstHour: 100, lastHour: 500, steps: [step(100, 500, 'autonomy_puppet')] },
  ] };
  const identities = countryIdentityIndex([{ tag: 'S1', identities: [...holder('S1', 1, 100, 199).identities, ...holder('S1', 2, 400, 500).identities] },
    holder('S2', 1, 200, 500), holder('OVR', 3), holder('KEPT', 4)]);
  const views = overlordViews(released, new Lineage([{ hour: 200, from: 'S1', to: 'S2' }]), identities);
  const colours = levelColours(levels, ['c0'], 'other');
  const labels = (extra: Partial<LaneOptions>) => subjectLanes(views, byToken, colours, 450, { ...options, ...extra }, text, []).lanes.map(lane => lane.identity?.label);
  assert.deepEqual(labels({}), ['OVR', 'KEPT', 'S2']);
  for (const tag of ['S1', 'S2']) assert.deepEqual(labels({ countries: new Set([tag]), countryIds: identities.selected([tag]) }), ['OVR', 'KEPT', 'S2']);
});

test('native relation endpoints include all X holders and exclude an unrelated alias holder and gaps', () => {
  const identities = countryIdentityIndex([{ tag: 'X', identities: [...holder('X', 1, 0, 9, 10).identities, ...holder('X', 1, 20, 30, 20).identities] },
    { tag: 'Y', identities: [...holder('Y', 1, 10, 19, 10).identities, ...holder('Y', 2, 20, 30, 30).identities] },
    ...['A', 'B', 'C', 'D'].map((tag, index) => holder(tag, index + 3))]);
  const runs = [['A', 'Y', 10, 19], ['B', 'X', 20, 30], ['C', 'Y', 20, 30], ['D', 'X', 15, 16]].map(([overlord, subject, first, last]) => ({
    overlord: String(overlord), subject: String(subject), firstHour: Number(first), lastHour: Number(last), startHour: 0,
    steps: [step(Number(first), Number(last), null)],
  }));
  const views = overlordViews({ ...reply, runs, unobserved: [] }, new Lineage([]), identities);
  const lanes = subjectLanes(views, byToken, new Map(), 30, { ...options, countries: new Set(['X']), countryIds: identities.selected(['X']) }, text, []);
  assert.deepEqual(lanes.lanes.filter(lane => lane.heading).map(lane => lane.identity?.label), ['A', 'B']);
});

test('a country chosen before its relation begins admits no overlord tree yet', () => {
  const identities = countryIdentityIndex([holder('B', 1), holder('A', 2), holder('C', 3)]);
  const views = overlordViews({ ...reply, unobserved: [], runs: [
    { overlord: 'B', subject: 'A', startHour: 0, firstHour: 0, lastHour: 300, steps: [step(0, 300, null)] },
    { overlord: 'B', subject: 'C', startHour: 200, firstHour: 200, lastHour: 300, steps: [step(200, 300, null)] },
  ] }, new Lineage([]), identities);
  const chosen = { ...options, countries: new Set(['C']), countryIds: identities.selected(['C']) };
  assert.deepEqual(subjectLanes(views, byToken, new Map(), 100, chosen, text, []).lanes, []);
  assert.deepEqual(subjectLanes(views, byToken, new Map(), 250, chosen, text, []).lanes.filter(lane => lane.heading).map(lane => lane.identity?.label), ['B']);
});

test('manual OLD never admits a replacement O tree when no tag-change event was recorded', () => {
  const identities = countryIdentityIndex([holder('OLD', 1, 0, 4, 10),
    { tag: 'O', identities: [...holder('O', 1, 5, 9, 10).identities, ...holder('O', 1, 10, 19, 20).identities] }, holder('S', 2, 5, 19, 30)]);
  const data: SubjectReply = { ...reply, unobserved: [], runs: [
    { overlord: 'O', subject: 'S', startHour: 5, firstHour: 5, lastHour: 9, steps: [step(5, 9, 'autonomy_puppet')] },
    { overlord: 'O', subject: 'S', startHour: 10, firstHour: 10, lastHour: 19, steps: [step(10, 19, 'autonomy_mod_march')] },
  ] };
  const views = overlordViews(data, new Lineage([]), identities);
  assert.deepEqual(views.map(view => view.key), ['[1,10]', '[1,20]']);
  const selected = { ...options, countries: new Set(['OLD']), countryIds: identities.selected(['OLD']) };
  assert.deepEqual(subjectLanes(views, byToken, new Map(), 19, { ...selected, ended: false }, text, []).lanes, []);
  const former = subjectLanes(views, byToken, new Map(), 19, selected, text, []);
  assert.equal(former.lanes.length, 2);
  assert.equal(former.info.get(former.lanes[0].id)?.overlord.key, '[1,10]');
  assert.equal(former.lanes[0].label, 'No subjects');
  assert.equal(former.lanes[1].label, 'Ended day 10');
  assert.equal(former.lanes[0].identity?.label, 'Unknown country');
  assert.deepEqual(laneCountries(former.lanes, former.info, 19), ['S']);
});

test('native replacement of either relation endpoint splits the tree or lane and clips levels', () => {
  for (const field of ['overlord', 'subject'] as const) {
    const identities = countryIdentityIndex([{ tag: 'X', identities: [...holder('X', 1, 0, 9, 10).identities, ...holder('X', 1, 10, 19, 20).identities] }, holder('KEPT', 2, 0, 19, 30)]);
    const views = overlordViews({ ...reply, unobserved: [], runs: [{
      overlord: field === 'overlord' ? 'X' : 'KEPT', subject: field === 'subject' ? 'X' : 'KEPT', startHour: 0, firstHour: 0, lastHour: 19,
      steps: [step(0, 11, 'autonomy_puppet'), step(12, 19, 'autonomy_mod_march')],
    }] }, new Lineage([]), identities);
    assert.equal(views.length, field === 'overlord' ? 2 : 1, field);
    const lanes = views.flatMap(view => view.subjects);
    assert.equal(lanes.length, 2, field);
    assert.deepEqual(lanes.map(view => view.runs.map(run => [run.firstHour, run.lastHour])), [[[0, 9]], [[10, 19]]], field);
    assert.deepEqual(lanes[1].runs[0].steps, [step(10, 11, 'autonomy_puppet'), step(12, 19, 'autonomy_mod_march')], field);
    const layout = subjectLanes(views, byToken, new Map(), 19, { ...options, ended: false }, text, []);
    assert.equal(layout.lanes.length, 2, field);
    assert.equal(layout.lanes[0].label, '1 subject', field);
  }
});

test('proved subject aliases across a native gap retain one relation but never supply a gap look or count', () => {
  const identities = countryIdentityIndex([holder('O', 2, 0, 19, 30), holder('X', 1, 0, 9, 10), holder('Y', 1, 11, 19, 10)]);
  const views = overlordViews({ ...reply, unobserved: [], runs: [
    { overlord: 'O', subject: 'X', startHour: 0, firstHour: 0, lastHour: 10, steps: [step(0, 10, 'autonomy_puppet')] },
    { overlord: 'O', subject: 'Y', startHour: 0, firstHour: 11, lastHour: 19, steps: [step(11, 19, 'autonomy_puppet')] },
  ] }, new Lineage([]), identities);
  assert.equal(views.length, 1);
  const [view] = views;
  assert.equal(view.subjects.length, 1);
  assert.deepEqual(view.subjects[0].stretches.map(run => [run.firstHour, run.lastHour]), [[0, 19]]);
  assert.deepEqual(view.counts.map(run => [run.firstHour, run.lastHour, run.value]), [[0, 9, 1], [11, 19, 1]]);
  assert.deepEqual(view.identityGaps, [{ firstHour: 10, lastHour: 10 }]);
  assert.equal(subjectTagAt(view, view.subjects[0], 10), null);
  assert.equal(subjectTagAt(view, view.subjects[0], 15), 'Y');
  const asked: string[] = [];
  const layout = subjectLanes(views, byToken, new Map(), 10, options, { ...text, country: tag => { asked.push(tag); return text.country(tag); } }, []);
  assert.equal(layout.lanes[0].label, 'Subjects not recorded');
  assert.equal(layout.lanes[1].identity?.label, 'Unknown country');
  assert.equal(layout.lanes[1].label, 'Level not recorded · Since day 0');
  assert.deepEqual([...new Set(asked)], ['O']);
  assert.deepEqual(laneCountries(layout.lanes, layout.info, 10), ['O']);
  const later = subjectLanes(views, byToken, new Map(), 15, options, text, []);
  assert.equal(later.lanes[1].identity?.label, 'Y');
  assert.deepEqual(laneCountries(later.lanes, later.info, 15), ['O', 'Y']);
});

test('a world of 1,500 countries lays out a lane per subject and overlord', () => {
  const world = (size: number): SubjectReply => ({
    ...reply, unobserved: [],
    runs: Array.from({ length: size }, (_, index) => [
      { overlord: `O${index % Math.max(1, size / 10)}`, subject: `S${index}`, startHour: index, firstHour: index, lastHour: index + 500,
        steps: [step(index, index + 250, 'autonomy_puppet'), step(index + 251, index + 500, 'autonomy_mod_march')] },
      { overlord: `O${index % Math.max(1, size / 10)}`, subject: `S${index}`, startHour: index + 600, firstHour: index + 600, lastHour: index + 900,
        steps: [step(index + 600, index + 900, null)] },
    ]).flat(),
  });
  const colours = levelColours(levels, ['c0', 'c1'], 'other');
  const lanes = subjectLanes(overlordViews(world(1500)), byToken, colours, 2500, options, text, []).lanes;
  assert.equal(lanes.length, 1500 + 150);
});

test('overlord and subject lanes reserve native flags without an available URL', () => {
  const labels = { ...text, country: (tag: string) => ({ name: tag, flagPending: true }) };
  const lanes = subjectLanes(overlordViews(reply), byToken, new Map(), 200, options, labels, reply.unobserved).lanes;
  assert.ok(lanes.some(lane => lane.depth === 1) && lanes.some(lane => lane.heading));
  for (const lane of lanes) {
    assert.equal(lane.identity?.countryFlag?.flagPending, true);
    assert.equal(lane.identity?.countryFlag?.flag, undefined);
  }
});

test('the timeline renders only where a relation, a level, a stretch not recorded or an identity changes', () => {
  const hours = changeHours(reply, [{ hour: 120 }]);
  assert.ok(hours.every((hour, index) => index === 0 || hours[index - 1] < hour), 'ascending and distinct');
  for (const hour of [100, 120, 150, 200, 300, 310]) assert.ok(hours.includes(hour), `${hour} is a change`);
  assert.equal(changesBy(hours, 101), changesBy(hours, 119));
  assert.notEqual(changesBy(hours, 119), changesBy(hours, 120));
  assert.equal(changesBy(hours, 0), 0);
  assert.equal(changesBy(hours, 1_000_000), hours.length);
});
