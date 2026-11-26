import assert from 'node:assert/strict';
import test from 'node:test';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { datedIdentity } from '../../families/identity/dated.fixture.ts';
import type { WarObservation, WarParticipantRun, WarTimeline, WarTimelineReply } from '../../generated/dashboard';
import { sideOf, validateWarTimeline, warSides, warsProjection, warTimelineProof, type WarsOptions } from './warsProjection.ts';

const run = (tag: string, side: WarParticipantRun['side'], firstHour: number, lastHour: number): WarParticipantRun => ({ tag, side, firstHour, lastHour, joinedBy: false, leftBy: false });
const seen = (attackersAreViewpoint: boolean | null, representative: [string, string] | null): WarObservation => ({
  firstHour: 100, lastHour: 400, viewpointTag: 'AAA', viewpointSide: [], opposingSide: [], attackersAreViewpoint, representative, name: { text: 'Not recorded', known: false, reason: 'none', key: null },
});
const war = (id: string, firstHour: number, lastHour: number, extra: Partial<WarTimeline> = {}): WarTimeline => ({
  id, firstHour, lastHour, startedBy: false, endedBy: false, observations: [seen(true, ['AAA', 'BBB'])],
  participants: [run('AAA', 'attacker', firstHour, lastHour), run('CCC', 'attacker', firstHour + 50, lastHour), run('BBB', 'defender', firstHour, lastHour)], capitulations: [], ...extra,
});
const reply = (hour: number, wars: WarTimeline[]): WarTimelineReply => ({ branchId: '1', hour, nextChange: null, available: true, viewpoints: [], wars, unobserved: [], capitulationsUnobserved: [], warnings: [] });
const OPTIONS: WarsOptions = { view: 'gantt', colourBy: 'war', firstHour: 0, surface: '#161616' };

test('sides come from the recorded orientation: the viewpoint attacks only where the war says so', () => {
  assert.equal(sideOf(run('X', 'viewpoint', 1, 2), seen(false, null)), 'defenders');
  assert.equal(sideOf(run('X', 'opponent', 1, 2), seen(true, null)), 'defenders');
  const flipped = war('1', 100, 200, { observations: [seen(false, ['BBB', 'AAA'])], participants: [run('AAA', 'viewpoint', 100, 200), run('BBB', 'opponent', 100, 200)] });
  assert.deepEqual(warSides(flipped).leads, ['BBB', 'AAA'], 'each side is led by its country in the representative pair');
});

test('each war is a lane with its two leads, an outcome, and its participants beneath, attackers first, then "Against"', () => {
  const capitulated = war('2', 100, 199, { capitulations: [{ tag: 'BBB', hour: 190 }] });
  const identity = datedIdentity({ AAA: [{ from: 0, name: 'Alpha' }], BBB: [{ from: 0, name: 'Bravo' }], CCC: [{ from: 0, name: 'Charlie' }] });
  const model = warsProjection.project(createBoundedReadView({ hour: 400, identity, panels: new Map([['main', reply(400, [war('1', 300, 400), capitulated])]]) }), OPTIONS);
  const heads = model.lanes.filter(lane => lane.sides);
  assert.deepEqual(heads.map(lane => lane.outcome?.text), ['Ongoing', 'Bravo capitulated']);
  assert.equal(heads[1]!.sides![1].dim, true, 'the side that capitulated dims');
  assert.equal(heads[0]!.defaultOpen, true, 'ongoing wars are open by default');
  const rows = model.lanes.filter(lane => lane.parentKey === heads[0]!.key);
  assert.deepEqual(rows.map(lane => lane.label), ['Alpha', 'Charlie', 'Against', 'Bravo']);
  assert.equal(rows[1]!.aside?.startsWith('Joined '), true, 'a late joiner says when it joined');
  assert.deepEqual(heads[1]!.spans[0]!.marks?.map(mark => mark.hour), [190]);
  assert.equal(model.firstHour, 100, 'the window starts at the first war shown');
  assert.equal(model.summaryText, '2 wars · 1 ongoing');
});

test('a war timeline reply holds nothing after its hour and proves up to its next change', () => {
  const ok = { ...reply(200, [war('1', 100, 200)]), nextChange: 250 };
  assert.equal(validateWarTimeline(ok), ok);
  assert.throws(() => validateWarTimeline(reply(150, [war('1', 100, 200)])), /malformed/, 'a war cut after the hour asked is refused');
  assert.deepEqual(warTimelineProof(ok, [0, 200], 220), [200, 249]);
  assert.equal(warTimelineProof(ok, [0, 200], 250), null);
});

test('every bar in the countries view identifies its country and resolves its war tooltip from the hovered span', () => {
  const ended = war('2', 100, 199);
  const identity = datedIdentity({ AAA: [{ from: 0, name: 'Alpha' }], BBB: [{ from: 0, name: 'Bravo' }], CCC: [{ from: 0, name: 'Charlie' }] });
  const view = createBoundedReadView({ hour: 400, identity, panels: new Map([['main', reply(400, [war('1', 300, 400), ended])]]) });
  const model = warsProjection.project(view, { ...OPTIONS, view: 'countries' });
  assert.deepEqual(model.lanes.map(lane => [lane.flagTag, lane.label, lane.subtitle, lane.flag?.status]), [
    ['AAA', 'Alpha', '1 war ongoing', 'pending'],
    ['CCC', 'Charlie', '1 war ongoing', 'pending'],
    ['BBB', 'Bravo', '1 war ongoing', 'pending'],
  ]);
  const alpha = model.lanes[0]!;
  assert.equal(alpha.spans.length, 2);
  const tip = warsProjection.tooltip!(model, { kind: 'span', laneKey: alpha.key, spanId: alpha.spans[0]!.id, hour: 350 }, view, { ...OPTIONS, view: 'countries' });
  assert.equal(tip?.title, 'Alpha');
  assert.deepEqual(tip?.items?.map(item => [item.label, item.value]), [
    ['Attackers (2)', 'Alpha, Charlie (joined Hour 350)'],
    ['Defenders (1)', 'Bravo'],
  ]);
});
