import test from 'node:test';
import assert from 'node:assert/strict';
import type { AutonomyReply, AutonomyRun } from '../../generated/dashboard';
import { laneStops } from '../../charts/timeline/layout.ts';
import { Lineage, countryMembership } from '../../dashboard/identity.ts';
import { absentAt, autonomyViews, chosenViews, clipSubject, formatProgress, formatScore, laneLabels, levelName, membershipUnknownFor, noLanesMessage, partlyIdentified, partlyRecorded, progressOf, runOf,
  subjectLane, validateAutonomyReply } from './autonomyData.ts';

const run = (firstHour: number, lastHour: number, level: number, scoreRaw: string, lowerRaw: string, upperRaw: string, extra: Partial<AutonomyRun> = {}): AutonomyRun => ({
  firstHour, lastHour, startsSegment: false, overlord: 'GER', level, scoreRaw, lowerRaw, upperRaw, lowest: false, highest: false, ratioRaw: '20000', ...extra,
});

test('a release at hour 150 keeps the historical lane and names recorded absence in its pill and description', () => {
  const subject = { tag: 'AAA', runs: [run(100, 149, 0, '15000000', '10000000', '25000000', { startsSegment: true })] };
  const reply: AutonomyReply = { ...REPLY, subjects: [subject], recorded: [{ firstHour: 100, lastHour: 300 }], unidentified: [] };
  const lane = subjectLane(subject, reply.levels);
  assert.equal(lane.spans.length, 1);
  assert.equal(laneLabels(reply, lane, 149, []).pill, 'Serf State');
  for (const hour of [150, 300]) {
    assert.deepEqual(laneLabels(reply, lane, hour, []), { label: 'Not a subject shown here', pill: 'Not a subject here', description: 'Not a subject shown here' });
  }
  const missing = { ...reply, recorded: [{ firstHour: 100, lastHour: 149 }] };
  assert.deepEqual(laneLabels(missing, lane, 300, []), { label: 'Not recorded', pill: 'Not recorded', description: 'Not recorded' });
  const unidentified = { ...reply, unidentified: [{ firstHour: 300, lastHour: 300 }] };
  assert.equal(laneLabels(unidentified, lane, 300, []).pill, 'Not known');
  assert.equal(laneLabels(unidentified, lane, 300, []).description, absentAt(unidentified, 300, []).text);
  const unobserved = [{ firstHour: 300, lastHour: 300 }];
  assert.equal(laneLabels(reply, lane, 300, unobserved).pill, 'Membership not recorded');
  assert.equal(laneLabels(reply, lane, 300, unobserved).description, absentAt(reply, 300, unobserved).text);
});

// A mod's own levels, in its own order and number: one the content names, one
// only recorded (named by its id), one the game left unlocalised.
const REPLY: AutonomyReply = {
  branchId: '1', scoreScale: 100_000,
  levels: [
    { id: 'mod_serf', name: 'Serf State', defined: true },
    { id: 'mod_vassal', name: null, defined: true },
    { id: 'mod_ally', name: 'Ally', defined: false },
  ],
  subjects: [
    { tag: 'D01 mod', runs: [
      run(100, 110, 0, '15000000', '10000000', '25000000', { startsSegment: true, lowest: true }),
      run(111, 120, 0, '20000000', '10000000', '25000000', { lowest: true }),
      // Moved up a level at 121.
      run(121, 130, 1, '25000000', '25000000', '50000000'),
      // A gap, then back at the lowest level: no change is marked across a gap.
      run(140, 150, 0, '12000000', '10000000', '25000000', { startsSegment: true, lowest: true }),
    ] },
    { tag: 'ITA', runs: [
      run(100, 150, 2, '51250000', '40000000', '50000000', { startsSegment: true, highest: true, overlord: 'ENG' }),
      run(151, 160, 2, '51250000', '40000000', '40000000', { highest: true, overlord: null, ratioRaw: null }),
    ] },
  ],
  recorded: [{ firstHour: 100, lastHour: 130 }, { firstHour: 140, lastHour: 160 }],
  unidentified: [],
  warnings: [],
};

test('renamed subjects and overlords keep one country history, while reused tags stay separate', () => {
  const lineage = new Lineage([{ hour: 120, from: 'AAA', to: 'BBB' }, { hour: 130, from: 'GER', to: 'NEW OWNER' }]);
  const reply = { ...REPLY, subjects: [
    { tag: 'AAA', runs: [run(100, 119, 0, '1', '0', '10', { startsSegment: true }), run(140, 150, 0, '3', '0', '10', { startsSegment: true })] },
    { tag: 'BBB', runs: [run(120, 129, 0, '2', '0', '10', { startsSegment: true }), run(130, 150, 0, '2', '0', '10', { overlord: 'NEW OWNER' })] },
  ] };
  const views = autonomyViews(reply, lineage);
  assert.equal(views.length, 2);
  const original = lineage.countryAt('AAA', 100);
  const selected = chosenViews(views, new Set([original]), true, lineage);
  assert.equal(selected.length, 1);
  const lane = subjectLane(selected[0], reply.levels);
  assert.equal(lane.id, original);
  assert.deepEqual(lane.stretches, [{ firstHour: 100, lastHour: 150 }]);
  assert.deepEqual(chosenViews(views, new Set([lineage.countryAt('NEW OWNER', 150)]), false, lineage).map(view => view.countryId), [original]);
  assert.deepEqual(chosenViews(views, new Set([lineage.countryAt('GER', 150)]), false, lineage).map(view => view.countryId), [lineage.countryAt('AAA', 140)]);
  const membership = countryMembership([{ tag: 'GER', firstHour: 100, lastHour: 129 }, { tag: 'NEW OWNER', firstHour: 130, lastHour: 145 }], lineage);
  assert.equal(clipSubject(selected[0], false, membership, [], lineage).runs.at(-1)?.lastHour, 145);
  const many = { ...REPLY, subjects: Array.from({ length: 5000 }, (_, index) => ({ tag: `mod ${index}`, runs: [run(100, 150, 0, '1', '0', '10')] })) };
  assert.equal(autonomyViews(many, lineage).length, 5000);
});

test('the reply is checked, and runs naming no level are refused', () => {
  assert.equal(validateAutonomyReply(REPLY), REPLY);
  assert.throws(() => validateAutonomyReply({ ...REPLY, subjects: [{ tag: 'X', runs: [run(1, 2, 3, '1', '0', '2')] }] }));
  assert.throws(() => validateAutonomyReply({ ...REPLY, levels: [...REPLY.levels, REPLY.levels[0]] }));
  assert.throws(() => validateAutonomyReply({ ...REPLY, scoreScale: 3 }));
});

test('splitting at an overlord tag change keeps a gap break only on the original first piece', () => {
  const lineage = new Lineage([{ hour: 120, from: 'GER', to: 'NEW OWNER' }]);
  const reply = { ...REPLY, subjects: [{ tag: 'AAA', runs: [run(100, 150, 0, '1', '0', '10', { startsSegment: true })] }] };
  const [view] = autonomyViews(reply, lineage);
  assert.deepEqual(view.runs.map(run => [run.firstHour, run.lastHour, run.startsSegment]), [[100, 119, true], [120, 150, false]]);
  const lane = subjectLane(view, reply.levels);
  assert.deepEqual(lane.stretches, [{ firstHour: 100, lastHour: 150 }]);
  assert.deepEqual(lane.progress.map(run => run.breakBefore), [true, false]);
});

test('levels are named by the game, else by their recorded id', () => {
  assert.deepEqual(REPLY.levels.map(levelName), ['Serf State', 'mod_vassal', 'Ally']);
  assert.equal(levelName(undefined), 'Unknown level');
});

test('a lane bands each level for the whole span it applied, marks each change and never joins across a gap', () => {
  const lane = subjectLane(REPLY.subjects[0], REPLY.levels);
  assert.deepEqual(lane.spans.map(span => [span.firstHour, span.lastHour, span.label]), [[100, 120, 'Serf State'], [121, 130, 'mod_vassal'], [140, 150, 'Serf State']]);
  assert.deepEqual([...lane.spanLevels.values()], [0, 1, 0]);
  assert.deepEqual(lane.changes, [121]);
  assert.deepEqual(lane.stretches, [{ firstHour: 100, lastHour: 130 }, { firstHour: 140, lastHour: 150 }]);
  assert.deepEqual(lane.progress.map(item => [item.firstHour, item.value, item.breakBefore]),
    // Exact to a billionth, rounded toward zero.
    [[100, 1 / 3, true], [111, 2 / 3, false], [121, 0, false], [140, 2 / 15, true]].map(([hour, value, fresh]) => [hour, Math.floor(Number(value) * 1e9) / 1e9, fresh]));
  assert.equal(runOf(lane, 135), null);
  assert.equal(runOf(lane, 125)?.level, 1);
});

test('progress sits between the thresholds, may pass the top one at the top level, and is undefined without a span', () => {
  const [top, flat] = REPLY.subjects[1].runs;
  assert.equal(progressOf(top), 1.125);
  assert.equal(formatProgress(top), '112.5%');
  assert.equal(progressOf(flat), null);
  assert.equal(formatProgress(flat), 'Not defined (both scores are equal)');
  assert.equal(formatScore(REPLY, top.scoreRaw), '512.5');
  // A lane without defined progress there draws no line (a gap), but keeps its band.
  const lane = subjectLane(REPLY.subjects[1], REPLY.levels);
  assert.deepEqual(lane.progress.map(item => item.lastHour), [150]);
  assert.deepEqual(lane.spans.map(span => [span.firstHour, span.lastHour]), [[100, 160]]);
});

test('the keyboard stops at every change inside a level, not only where the level starts', () => {
  // The level holds 100–120, but the score changes at 111.
  const lane = subjectLane(REPLY.subjects[0], REPLY.levels);
  assert.deepEqual(lane.spans[0].stops, [111]);
  assert.deepEqual(laneStops(lane.spans, 100, 150, null).map(stop => stop.hour), [100, 111, 121, 140]);
});

test('a group counted at each hour shows a subject only while it, or its overlord, belongs; unknown membership is cut out', () => {
  const subject = REPLY.subjects[0];
  // As a chosen subject: a member from 110 (as the human players group can become), so nothing shows before.
  const member = new Map([['D01 mod', [{ firstHour: 110, lastHour: 150 }]]]);
  const own = clipSubject(subject, true, member, []);
  assert.deepEqual(own.runs.map(item => [item.firstHour, item.lastHour]), [[110, 110], [111, 120], [121, 130], [140, 150]]);
  // As a subject of chosen overlords: only while its overlord (GER) belongs.
  const overlords = new Map([['GER', [{ firstHour: 100, lastHour: 115 }]]]);
  assert.deepEqual(clipSubject(subject, false, overlords, []).runs.map(item => [item.firstHour, item.lastHour]), [[100, 110], [111, 115]]);
  // Hours whose membership was not recorded are cut out, and break the line.
  const always = new Map([['D01 mod', [{ firstHour: 0, lastHour: 1000 }]]]);
  const cut = clipSubject(subject, true, always, [{ firstHour: 105, lastHour: 112 }]);
  assert.deepEqual(cut.runs.map(item => [item.firstHour, item.lastHour]), [[100, 104], [113, 120], [121, 130], [140, 150]]);
  assert.deepEqual(subjectLane(cut, REPLY.levels).stretches, [{ firstHour: 100, lastHour: 104 }, { firstHour: 113, lastHour: 130 }, { firstHour: 140, lastHour: 150 }]);
  // Without a group nothing changes.
  assert.equal(clipSubject(subject, true, null, []), subject);
});

test('a group counted at one moment keeps its countries’ whole history, however its membership went unrecorded elsewhere', () => {
  // AAA is a member at the branch end (110); its subject BBB was recorded at 102–104, when membership was not.
  const bbb = { tag: 'BBB', runs: [run(102, 104, 0, '15000000', '10000000', '25000000', { startsSegment: true, overlord: 'AAA' })] };
  const unobserved = [{ firstHour: 100, lastHour: 105 }];
  assert.equal(clipSubject(bbb, false, null, unobserved), bbb);
  assert.equal(clipSubject(bbb, true, null, unobserved), bbb);
  // Counted at each hour, the same hours are unknown and cut out.
  assert.deepEqual(clipSubject(bbb, false, new Map([['AAA', [{ firstHour: 100, lastHour: 110 }]]]), unobserved).runs, []);
});

test('a branch recorded only in part says so, so a missing lane is not called "no subjects"', () => {
  assert.equal(partlyRecorded(REPLY, { firstHour: 100, lastHour: 160 }), true);
  assert.equal(partlyRecorded({ ...REPLY, recorded: [{ firstHour: 90, lastHour: 170 }] }, { firstHour: 100, lastHour: 160 }), false);
  assert.throws(() => validateAutonomyReply({ ...REPLY, recorded: [{ firstHour: 5, lastHour: 1 }] }));
});

test('a subject whose overlord changes is cut by each overlord’s own membership, in hour order', () => {
  const subject = { tag: 'D01 mod', runs: [
    run(100, 109, 0, '15000000', '10000000', '25000000', { startsSegment: true }),
    run(110, 119, 0, '15000000', '10000000', '25000000', { overlord: 'ENG' }),
    run(120, 129, 0, '15000000', '10000000', '25000000'),
    run(130, 139, 0, '15000000', '10000000', '25000000', { overlord: null, ratioRaw: null }),
  ] };
  const membership = new Map([['GER', [{ firstHour: 105, lastHour: 125 }]], ['ENG', [{ firstHour: 100, lastHour: 200 }]]]);
  assert.deepEqual(clipSubject(subject, false, membership, []).runs.map(item => [item.firstHour, item.lastHour, item.overlord, item.startsSegment]),
    [[105, 109, 'GER', false], [110, 119, 'ENG', false], [120, 125, 'GER', false]]);
});

test('cutting a subject by its overlords’ membership counted at each hour keeps their member turns', () => {
  // Two overlords taking turns every other hour, each a member every other turn: as
  // many membership spans as runs, the shape that took seconds when cut run by run.
  const history = (count: number) => {
    const runs = Array.from({ length: count }, (_, index) => run(index * 2, index * 2 + 1, 0, '15000000', '10000000', '25000000',
      { overlord: index % 2 ? 'ENG' : 'GER' }));
    const spans = (parity: number) => Array.from({ length: count / 2 }, (_, index) => index * 4 + parity * 2)
      .filter((_, index) => index % 2 === 0).map(hour => ({ firstHour: hour, lastHour: hour + 1 }));
    return { subject: { tag: 'D01 mod', runs }, membership: new Map([['GER', spans(0)], ['ENG', spans(1)]]) };
  };
  const { subject, membership } = history(32_000);
  assert.equal(clipSubject(subject, false, membership, []).runs.length, 16_000);
});

test('an hour with a subject or overlord the recording does not name is never called one without subjects', () => {
  const branch = { firstHour: 100, lastHour: 160 };
  const partial: AutonomyReply = { ...REPLY, recorded: [{ firstHour: 100, lastHour: 160 }], unidentified: [{ firstHour: 135, lastHour: 136 }] };
  assert.equal(validateAutonomyReply(partial), partial);
  assert.throws(() => validateAutonomyReply({ ...REPLY, unidentified: [{ firstHour: 5, lastHour: 1 }] }));
  assert.throws(() => validateAutonomyReply({ ...REPLY, unidentified: undefined }));
  assert.equal(partlyIdentified(partial, branch), true);
  assert.equal(partlyIdentified(partial, { firstHour: 100, lastHour: 134 }), false);
  // A lane without a run at such an hour is not known, where elsewhere it is not a subject.
  assert.match(absentAt(partial, 135, []).text, /^Not known/);
  assert.equal(absentAt(partial, 137, []).text, 'Not a subject shown here');
  assert.equal(absentAt(REPLY, 135, []).text, 'Not recorded');
  assert.equal(absentAt(partial, 135, [{ firstHour: 130, lastHour: 140 }]).text, 'Group membership not recorded');
  // The empty state never claims absence then, by overlord or by subject.
  const byOverlord = noLanesMessage(partial, branch, false, ['Germany']);
  const bySubject = noLanesMessage(partial, branch, true, []);
  assert.match(byOverlord, /^Germany had no subjects the recording names in this campaign branch; .* may have had one\.$/);
  assert.match(bySubject, /may have been\.$/);
  assert.doesNotMatch(bySubject, /^None of these countries was a subject/);
  // Fully identified, it says so plainly, and a partly recorded branch still says which hours.
  assert.equal(noLanesMessage({ ...partial, unidentified: [] }, branch, false, ['Germany']), 'Germany had no subjects in this campaign branch.');
  assert.equal(noLanesMessage(REPLY, branch, true, []), 'None of these countries was a subject in the hours this campaign branch recorded.');
});

test('a lane cut away where the group’s membership was unknown is said, never called "no subjects"', () => {
  // A belongs to the group 100–105 and its membership is unknown 106–110, when B is its subject.
  const b = { tag: 'B', runs: [run(106, 110, 0, '15000000', '10000000', '25000000', { startsSegment: true, overlord: 'A' })] };
  const reply: AutonomyReply = { ...REPLY, subjects: [b], recorded: [{ firstHour: 100, lastHour: 160 }] };
  const membership = new Map([['A', [{ firstHour: 100, lastHour: 105 }]]]);
  const unobserved = [{ firstHour: 106, lastHour: 110 }];
  assert.deepEqual(clipSubject(b, false, membership, unobserved).runs, []);
  assert.equal(membershipUnknownFor([b], false, unobserved), true);
  const branch = { firstHour: 100, lastHour: 160 };
  const message = noLanesMessage(reply, branch, false, ['A'], membershipUnknownFor([b], false, unobserved));
  assert.equal(message, 'No subjects of A are shown in this campaign branch: the group’s membership was not recorded at some hours they may have had subjects.');
  assert.doesNotMatch(message, /had no subjects/);
  assert.match(noLanesMessage(reply, branch, true, [], true), /^None of these countries is shown as a subject .*membership was not recorded/);
  // Membership unknown only where no lane had a run, or only for a run without an overlord, cuts nothing that could show.
  assert.equal(membershipUnknownFor([b], false, [{ firstHour: 111, lastHour: 120 }]), false);
  const released = { tag: 'B', runs: [run(106, 110, 0, '1', '0', '2', { overlord: null })] };
  assert.equal(membershipUnknownFor([released], false, unobserved), false);
  assert.equal(membershipUnknownFor([released], true, unobserved), true);
  assert.equal(noLanesMessage(reply, branch, false, ['A'], false), 'A had no subjects in this campaign branch.');
});

test('subjects are those of the chosen overlords at any hour, or the chosen subjects, in the reply’s order', () => {
  const lineage = new Lineage([]);
  const views = autonomyViews(REPLY, lineage);
  const ids = (...tags: string[]) => new Set(tags.map(tag => lineage.countryAt(tag, 160)));
  assert.deepEqual(chosenViews(views, ids('GER'), false, lineage).map(subject => subject.tag), ['D01 mod']);
  assert.deepEqual(chosenViews(views, ids('ENG'), false, lineage).map(subject => subject.tag), ['ITA']);
  assert.deepEqual(chosenViews(views, ids('ITA', 'D01 mod'), true, lineage).map(subject => subject.tag), ['D01 mod', 'ITA']);
  assert.deepEqual(chosenViews(views, ids('FRA'), false, lineage), []);
});
