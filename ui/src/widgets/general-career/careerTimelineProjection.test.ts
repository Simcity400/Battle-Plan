import assert from 'node:assert/strict';
import test from 'node:test';
import { liveKey } from '../../charts/live.ts';
import { engineHour } from '../../dashboard/calendar.ts';
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
