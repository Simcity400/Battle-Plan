import assert from 'node:assert/strict';
import test from 'node:test';
import { liveKey } from '../../charts/live.ts';
import { engineHour, firstHourOfMonth, gameDate, monthIndex } from '../../dashboard/calendar.ts';
import { createBoundedReadView } from '../../families/boundedView.ts';
import { datedIdentity } from '../../families/identity/dated.fixture.ts';
import { assertQuietFrames, LANE_THEME } from '../../families/laneRuntime.fixture.ts';
import { createLaneView } from '../../families/laneRuntime.ts';
import { nextDayStart, timelineLane, type FamilyTimelineBinding } from '../../families/timeline/laneView.ts';
import type { LeaderCareerReply, LeaderHourReply, LeaderRole } from '../../generated/dashboard';
import {
  careerTimelineDeclaration,
  careerTimelineProjection,
  leaderCareerProof,
  TIMELINE_VIEW,
  validateLeaderCareer,
  type CareerTimelineOptions,
} from './careerTimelineProjection.ts';

const H = (year: number, month = 1, day = 1) => engineHour({ year, month, day, hour: 1 })!;
const FIRST = H(1936, 1, 1);
const AT = H(1941, 9, 15);

const role: LeaderRole = {
  owner: 'JAP',
  kind: 1,
  category: 0,
  rank: 'General',
  attributeNames: ['Attack', 'Defense', 'Planning', 'Logistics'],
  maximumSkill: 9,
};

const hourReply = (): LeaderHourReply => ({
  reference: '42',
  missing: null,
  name: 'Tomoyuki Yamashita',
  role,
  skill: 7,
  components: [6, 5, 5, 4],
  traits: [0],
  traitDates: [[FIRST, 0]],
  traitCatalogue: [{ token: 'infantry_officer', name: 'Infantry Officer', icon: null, missing: null }],
  fought: 1584,
  won: 841,
  lost: 737,
  battlesPartialFrom: null,
  firstSeenHour: FIRST,
  progress: {
    nextRequiredRaw: '100000000',
    training: [
      { key: 't1', token: 'infantry_leader', name: 'Infantry Leader', requiredRaw: '100000000' },
      { key: 't2', token: 'skilled_staffer', name: 'Skilled Staffer', requiredRaw: '200000000' },
    ],
    factors: [],
  },
  warnings: [],
});

const careerReply = (hour = AT, nextChange: number | null = AT + 500): LeaderCareerReply => ({
  reference: '42',
  hour,
  nextChange,
  name: 'Tomoyuki Yamashita',
  portraitHour: hour,
  runs: [{ firstHour: FIRST, lastHour: hour, role, skill: 7, components: [6, 5, 5, 4] }],
  traitGains: [[FIRST, 0, 0], [H(1938, 4, 1), 1, 1]],
  traits: [
    { token: 'infantry_officer', name: 'Infantry Officer', icon: null, missing: null },
    { token: 'infantry_leader', name: 'Infantry Leader', icon: null, missing: null },
  ],
  commands: [
    { firstHour: FIRST, lastHour: H(1937, 8, 1), formation: '1', kind: 'army', name: 'Home army' },
    { firstHour: H(1937, 8, 2), lastHour: hour, formation: '2', kind: 'army', name: 'Expeditionary army' },
  ],
  battles: [
    { start: H(1938, 1, 5), read: H(1938, 1, 5), unknown: false, kind: 0, won: true, resultHour: H(1938, 1, 8), resultRead: H(1938, 1, 8) },
    { start: H(1938, 1, 10), read: H(1938, 1, 10), unknown: false, kind: 0, won: true, resultHour: H(1938, 1, 12), resultRead: H(1938, 1, 12) },
    { start: H(1938, 1, 15), read: H(1938, 1, 15), unknown: false, kind: 0, won: false, resultHour: H(1938, 1, 16), resultRead: H(1938, 1, 16) },
  ],
  observed: [[FIRST, hour]],
  battlesPartialFrom: null,
  resultsMissing: '',
  missing: null,
  warnings: [],
});

const OPTIONS: CareerTimelineOptions = { reference: '42', firstHour: FIRST, surface: '#161616' };

test('general career timeline projects leader header KPIs, command bars, battle histogram, trait diamonds, and training meters', () => {
  const identity = datedIdentity({ JAP: [{ from: FIRST, name: 'Imperial Japan' }] });
  const live = new Map<string, number | bigint | string>([
    [liveKey('t1', 'experience'), '64000000'],
    [liveKey('t2', 'experience'), '82000000'],
  ]);
  const view = createBoundedReadView({
    hour: AT,
    identity,
    panels: new Map<string, unknown>([['hour', hourReply()], ['career', validateLeaderCareer(careerReply())]]),
    live: new Map([['hour', { get: k => live.get(k), missing: () => '' }]]),
  });
  const model = careerTimelineProjection.project(view, OPTIONS);
  assert.equal(model.axisPosition, 'top');
  assert.equal(model.header?.title, 'Tomoyuki Yamashita');
  assert.equal(model.header?.subtitle, 'General · ★ 7 / 9');
  assert.deepEqual(model.header?.kpis?.map(k => [k.value, k.label]), [
    ['841', 'won'], ['737', 'lost'], ['6', 'unresolved'], ['1,584', 'fought'], ['53%', 'won'],
  ]);
  assert.deepEqual(model.lanes.map(l => l.key), ['command', 'battles', 'traits']);
  assert.deepEqual(model.lanes[0]!.spans.map(s => s.label), ['Home army', 'Expeditionary army']);
  assert.deepEqual(model.lanes[1]!.spans[0]!.histogram, { positive: 2, negative: 1, max: 2 });
  assert.equal(model.lanes[2]!.spans[0]!.diamond?.text, undefined, 'held-before trait has blue diamond without clutter text');
  assert.equal(model.lanes[2]!.spans[1]!.diamond?.text, 'Infantry Leader');
  assert.equal(model.meters?.title, 'Traits in training');
  assert.deepEqual(model.meters?.items.map(i => [i.label, i.figure]), [
    ['Infantry Leader', '640 / 1,000'],
    ['Skilled Staffer', '820 / 2,000'],
  ]);
});

test('leader career reply validates causality, proves quiet stretches up to nextChange, and paints once per day while quiet', () => {
  const data = validateLeaderCareer(careerReply(AT, AT + 500));
  assert.deepEqual(leaderCareerProof(data, [FIRST, AT], AT + 100), [AT, AT + 499]);
  assert.equal(leaderCareerProof(data, [FIRST, AT], AT + 500), null);
  assert.throws(() => validateLeaderCareer({ ...data, nextChange: AT }), /malformed/);

  const binding: FamilyTimelineBinding<CareerTimelineOptions> = {
    family: 'timeline',
    kind: 'general-career',
    view: TIMELINE_VIEW,
    options: OPTIONS,
    sources: {},
    staticPanels: { hour: hourReply(), career: data },
    width: 680,
    height: 360,
    scrollTop: 0,
    theme: LANE_THEME,
    identitySnapshot: null,
    selectedKey: null,
    focusedKey: null,
    expanded: {},
    pauseFlush: 0,
  };
  const surface = { width: 680, height: 360, dpr: 1 };
  const nextWake = assertQuietFrames(() => createLaneView(timelineLane, () => {}), { label: 'career-timeline quiet', binding, hour: AT }, 30, surface);
  assert.equal(nextWake, nextDayStart(AT));
  for (let h = AT; h <= AT + 30; h++) {
    assert.deepEqual(leaderCareerProof(data, [FIRST, h], h), [AT, AT + 499]);
  }
  assert.equal(careerTimelineDeclaration.kind, 'general-career');
});

const projectAt = (hour: LeaderHourReply | undefined, career: LeaderCareerReply) => careerTimelineProjection.project(createBoundedReadView({
  hour: career.hour,
  identity: datedIdentity({ JAP: [{ from: FIRST, name: 'Imperial Japan' }] }),
  panels: new Map<string, unknown>([...(hour ? [['hour', hour] as const] : []), ['career', validateLeaderCareer(career)]]),
}), OPTIONS);

test('a general no longer serving keeps the battles the career recorded, not the hour reply’s empty counters', () => {
  const retired = H(1939, 1, 1);
  const career = { ...careerReply(), runs: [{ firstHour: FIRST, lastHour: retired, role, skill: 6, components: [5, 5, 5, 4] as [number, number, number, number] }] };
  const notServing: LeaderHourReply = { ...hourReply(), missing: 'This leader is not serving at this hour', role: null, skill: null, fought: 0, won: 0, lost: 0, progress: null, traits: null };
  const model = projectAt(notServing, career);
  assert.deepEqual(model.header?.kpis?.map(k => [k.value, k.label]), [['2', 'won'], ['1', 'lost'], ['0', 'unresolved'], ['3', 'fought'], ['67%', 'won']]);
  assert.equal(model.header?.subtitle, 'General · ★ 6 / 9', 'rank and skill as last recorded in service');
  assert.ok(model.notes?.includes('This leader is not serving at this hour'), 'why the hour reply holds no counters is said');
  assert.equal(model.meters, undefined, 'no training is claimed for a leader not serving');
});

test('a month of battles is drawn only over recorded hours in service, and its count is qualified when part of the month was not recorded', () => {
  const left = H(1938, 1, 20);
  const career: LeaderCareerReply = {
    ...careerReply(),
    runs: [{ firstHour: FIRST, lastHour: left, role, skill: 7, components: [6, 5, 5, 4] }],
    observed: [[FIRST, H(1938, 1, 12)], [H(1938, 1, 18), AT]],
  };
  const model = projectAt(undefined, career);
  const month = model.lanes.find(lane => lane.key === 'battles')!.spans;
  const january = firstHourOfMonth(monthIndex(gameDate(H(1938, 1, 5))!));
  assert.deepEqual(month.map(span => [span.firstHour, span.lastHour]), [[january, H(1938, 1, 12)], [H(1938, 1, 18), left]],
    'the bar stops at the gap and where service ended');
  assert.deepEqual(model.unobserved, [{ firstHour: H(1938, 1, 12) + 1, lastHour: H(1938, 1, 18) - 1 }], 'the unrecorded stretch is hatched');
  const tip = careerTimelineProjection.tooltip!(model, { kind: 'span', laneKey: 'battles', spanId: month[0]!.id, hour: H(1938, 1, 6) }, createBoundedReadView({ hour: AT }), OPTIONS);
  assert.ok(JSON.stringify(tip).includes('not recorded'), 'the month count says part of the month was not recorded');
});

test('traits gained at the same hour are one marker that names each of them', () => {
  const both = H(1939, 5, 1);
  const career: LeaderCareerReply = {
    ...careerReply(),
    traitGains: [[FIRST, 0, 0], [both, 1, 1], [both, 2, 1]],
    traits: [...careerReply().traits, { token: 'skilled_staffer', name: 'Skilled Staffer', icon: null, missing: null }],
  };
  const model = projectAt(undefined, career);
  const traits = model.lanes.find(lane => lane.key === 'traits')!.spans;
  assert.equal(traits.length, 2);
  const tip = careerTimelineProjection.tooltip!(model, { kind: 'span', laneKey: 'traits', spanId: traits[1]!.id, hour: both }, createBoundedReadView({ hour: AT }), OPTIONS);
  const text = JSON.stringify(tip);
  assert.ok(text.includes('Infantry Leader') && text.includes('Skilled Staffer'), text);
});

test('a career reply shown after its read hour runs its service and coverage on to the shown hour', () => {
  const read = H(1938, 1, 10);
  const career: LeaderCareerReply = { ...careerReply(read, H(1938, 3, 1)), observed: [[FIRST, read]],
    runs: [{ firstHour: FIRST, lastHour: read, role, skill: 7, components: [6, 5, 5, 4] }],
    battles: careerReply().battles.slice(0, 2) };
  const shown = H(1938, 1, 20);
  const model = careerTimelineProjection.project(createBoundedReadView({ hour: shown, panels: new Map<string, unknown>([['career', validateLeaderCareer(career)]]) }), OPTIONS);
  assert.deepEqual(model.unobserved, [], 'nothing is hatched inside the proven interval');
  const bar = model.lanes.find(lane => lane.key === 'battles')!.spans[0]!;
  assert.equal(bar.lastHour, shown, 'the month bar advances to the shown hour');
});

test('a battle dated in a month with no recorded hour in service draws no bar but still counts', () => {
  const career: LeaderCareerReply = { ...careerReply(), observed: [[FIRST, H(1937, 12, 31)], [H(1938, 2, 1), AT]] };
  const model = projectAt(undefined, career);
  assert.deepEqual(model.lanes.find(lane => lane.key === 'battles')!.spans, [], 'January 1938 was not recorded');
  assert.equal(model.header?.kpis?.find(kpi => kpi.label === 'fought')?.value, '3');
});

test('with a Range, trait gains before it are not piled onto its first hour', () => {
  const model = careerTimelineProjection.project(createBoundedReadView({ hour: AT, panels: new Map<string, unknown>([['career', validateLeaderCareer(careerReply())]]) }),
    { ...OPTIONS, firstHour: H(1938, 1, 1) });
  const traits = model.lanes.find(lane => lane.key === 'traits')!.spans;
  assert.deepEqual(traits.map(span => span.firstHour), [H(1938, 4, 1)], 'the gain held before 1936 lies outside the Range');
});
