import assert from 'node:assert/strict';
import test from 'node:test';
import { engineHour } from '../../dashboard/calendar.ts';
import type { CategoryInfo, IdeaHistoryReply, IdeaHolding, IdeaInfo, IdeaLook } from '../../generated/dashboard';
import { ideaTimelineProjection } from '../../widgets/idea-timeline/ideaTimelineProjection.ts';
import { createBoundedReadView } from '../boundedView.ts';
import { timelineHostAdapter } from './hostAdapter.ts';
import { frameTimeline, placeTimeline } from './layout.ts';
import { resolutionFor } from './resolution.ts';
import { familyTimelineLaneView, nextDayStart, type FamilyTimelineBinding } from './laneView.ts';
import { registerFamilyProjection } from '../registry.ts';
import { timelineModel } from './helpers.ts';
import type { PaintInput } from '../../draw/lane.ts';
import type { FamilyTheme } from '../contract.ts';
import { assertFreshPaint, assertQuietFrames, LANE_THEME } from '../laneRuntime.fixture.ts';
import { visibleLanes, type TimelineLane, type TimelineModel, type TimelineProjection } from './model.ts';
import { decodeTimelineHit, encodeTimelineHit, hitTimeline } from './painter.ts';

const H = (year: number, month = 1, day = 1) => engineHour({ year, month, day, hour: 1 })!;
const FIRST = H(1936);

const idea = (token: string, slot: string): IdeaInfo => ({ token, category: 'government', slot, law: true, designer: false,
  effects: [], targeted: [], equipment: [], research: [], traits: [], tooltips: [], effectsKnown: true, unknown: null, nameMissing: null });
const GOVERNMENT: CategoryInfo = { token: 'government', name: 'Laws', kind: null, hidden: false, politicsTab: true, slots: [
  { token: 'economy', name: 'Economy law', character: false, seats: 1, laws: ['civilian', 'war'] },
  { token: 'trade', name: 'Trade law', character: false, seats: 1, laws: ['free', 'autarky'] },
] };
const GAP = { firstHour: H(1938, 6), lastHour: H(1938, 10) - 1 };
const LOOKS: IdeaLook[] = [
  { info: idea('civilian', 'economy'), artwork: { token: 'civilian', name: null, description: null, baseDurationDays: null, icon: null } },
  { info: idea('war', 'economy'), artwork: { token: 'war', name: 'War Economy', description: null, baseDurationDays: null, icon: 'asset://icons/war.png' } },
  { info: idea('free', 'trade'), artwork: { token: 'free', name: null, description: null, baseDurationDays: null, icon: null } },
  { info: idea('autarky', 'trade'), artwork: { token: 'autarky', name: null, description: null, baseDurationDays: null, icon: null } },
];
const holding = (look: number, firstHour: number, lastHour: number, start: IdeaHolding['start'], end: IdeaHolding['end']): IdeaHolding =>
  ({ token: LOOKS[look]!.info.token, firstHour, lastHour, start, end, slot: null, character: null, look });
const replyAt = (hour: number, holdings: IdeaHolding[], extra: Partial<IdeaHistoryReply> = {}): IdeaHistoryReply => ({
  branchId: '1', tag: 'GER', asked: 'GER', hour, nextChange: null, unrecorded: null, holdings,
  held: holdings.flatMap((item, at) => item.end.kind === 'held' ? [at] : []), looks: LOOKS, categories: [GOVERNMENT],
  unobserved: [], modifiers: [], texts: [], warnings: [], ...extra,
});
const REPLIES = new Map<number, IdeaHistoryReply>([
  [H(1937, 3), replyAt(H(1937, 3), [holding(0, FIRST, H(1937, 3), 'heldBefore', { kind: 'held' }), holding(2, FIRST, H(1937, 3), 'heldBefore', { kind: 'held' })], { nextChange: H(1937, 6) })],
  [H(1938, 7), replyAt(H(1938, 7), [
    holding(0, FIRST, H(1937, 6) - 1, 'heldBefore', { kind: 'ended' }), holding(2, FIRST, H(1938, 6) - 1, 'heldBefore', { kind: 'lastSeen' }),
    holding(1, H(1937, 6), H(1938, 6) - 1, 'seen', { kind: 'lastSeen' }),
  ], { unrecorded: 'Ideas not read at this hour', unobserved: [{ firstHour: GAP.firstHour, lastHour: H(1938, 7) }], nextChange: H(1938, 10) })],
  [H(1940), replyAt(H(1940), [
    holding(0, FIRST, H(1937, 6) - 1, 'heldBefore', { kind: 'ended' }), holding(2, FIRST, H(1938, 6) - 1, 'heldBefore', { kind: 'endedBy', hour: H(1938, 10) }),
    holding(1, H(1937, 6), H(1940), 'seen', { kind: 'held' }), holding(3, H(1938, 10), H(1940), 'firstObserved', { kind: 'held' }),
  ], { unobserved: [GAP], nextChange: H(1941) + 1 })],
]);
const OPTIONS = { category: '', iconsOnBars: true, firstHour: FIRST, surface: '#161616' };

function project(hour: number, displayed = hour): TimelineModel {
  const readView = createBoundedReadView({ hour: displayed, panels: new Map([['main', REPLIES.get(hour)!]]) });
  return ideaTimelineProjection.project(readView, OPTIONS);
}

const lane = (model: TimelineModel, key: string): TimelineLane => model.lanes.find(item => item.key === key)!;

test('at the displayed hour nothing later is drawn, and a held item runs square into the playhead', () => {
  const at = H(1937, 3);
  const model = project(at);
  assert.deepEqual([model.firstHour, model.hour], [FIRST, at]);
  const economy = lane(model, 'economy');
  assert.deepEqual(economy.spans.map(span => [span.label, span.lastHour, span.open]), [['Idea name unavailable', at, true]], 'War Economy starts later and is not drawn');
  assert.equal(economy.subtitle, 'Idea name unavailable');
  assert.equal(economy.spans[0]!.heldBefore, true, 'held when the recording began');
  assert.equal(model.nextChangeAt, H(1937, 6));
  const later = project(H(1937, 3), H(1937, 5));
  assert.deepEqual(lane(later, 'economy').spans.map(span => [span.lastHour, span.open]), [[H(1937, 5), true]], 'inside the proven stretch a held item runs on to the displayed hour');
});

test('an item that ended is not open, an item with no art is a named missing icon, and art comes from the recorded path', () => {
  const model = project(H(1940));
  const [civilian, war] = lane(model, 'economy').spans;
  assert.deepEqual([civilian!.open, civilian!.lastHour], [false, H(1937, 6) - 1]);
  assert.equal(civilian!.iconMissing, true);
  assert.deepEqual([war!.open, war!.lastHour, war!.icon], [true, H(1940), { kind: 'url', url: 'asset://icons/war.png' }]);
});

test('a slot read during an unrecorded stretch says so, and an end inside the stretch is not a guessed edge', () => {
  const trade = lane(project(H(1938, 7)), 'trade');
  assert.equal(trade.subtitle, 'Not recorded');
  assert.deepEqual(project(H(1938, 7), H(1938, 8)).unobserved, [{ firstHour: GAP.firstHour, lastHour: H(1938, 8) }], 'an open unread stretch runs on to the displayed hour');
  const later = lane(project(H(1940)), 'trade');
  assert.equal(later.spans[0]!.endUnseen, true, 'last seen just before the gap: its end is somewhere in the gap');
  assert.deepEqual([later.spans[1]!.heldBefore, later.spans[1]!.startBy], [undefined, true], 'first seen just after the gap: a "by" start, not held before the recording');
  assert.equal(later.subtitle, later.spans[1]!.label);
});

test('expanded state hides children of closed lanes, and hits encode lane, span and hour', () => {
  const lanes: TimelineLane[] = [
    { key: 'g', kind: 'group', label: 'Axis', expandable: true, defaultOpen: true, spans: [], auditValue: 'Axis' },
    { key: 'm', kind: 'item', label: 'Italy', parentKey: 'g', depth: 1, spans: [{ id: 's', firstHour: FIRST, lastHour: H(1940), color: '#fff', open: true }], auditValue: 'Italy' },
  ];
  assert.equal(visibleLanes(lanes, {}).length, 2);
  assert.equal(visibleLanes(lanes, { g: false }).length, 1);
  const model: TimelineModel = { hour: H(1940) as TimelineModel['hour'], firstHour: FIRST, lanes, unobserved: [], iconsOnBars: false, labelWidth: 200, warnings: [], nextChangeAt: Infinity };
  const layout = frameTimeline(placeTimeline(model, 600, 200, {}), H(1941));
  assert.equal(layout.endHour, H(1941), 'the axis runs to the painted hour, not the projection hour');
  const item = layout.lanes[1]!;
  const resolution = resolutionFor(null, model, layout);
  const hit = hitTimeline(layout, resolution, layout.plotRight - 4, item.y + item.height / 2, 0);
  assert.equal(hit?.kind, 'span');
  const token = encodeTimelineHit(hit);
  assert.deepEqual(decodeTimelineHit(token), hit);
  assert.deepEqual(timelineHostAdapter.parseHit!(token), { key: 'm', subject: 'm', hour: hit!.hour, kind: 'span', toggle: false, inspectToken: token });
  const toggle = hitTimeline(layout, resolution, 20, layout.lanes[0]!.y + 5, 0);
  assert.equal(timelineHostAdapter.parseHit!(encodeTimelineHit(toggle))?.toggle, true);
});

test('the picture repaints at each new day so the playhead and open bars follow the clock between content changes', () => {
  assert.equal(nextDayStart(H(1940, 3, 4)), H(1940, 3, 5));
  assert.equal(nextDayStart(H(1940, 3, 4) + 13), H(1940, 3, 5));
  assert.equal(nextDayStart(H(1940, 12, 31) + 23), H(1941));
});

test('a pointer over a painted bar hits that bar, even when an earlier bar ends within the slack, and keeps its own hour', () => {
  const lanes: TimelineLane[] = [{ key: 'l', kind: 'item', label: 'Lane', auditValue: 'Lane', spans: [
    { id: 'a', firstHour: FIRST, lastHour: H(1938) - 1, color: '#fff', open: false },
    { id: 'b', firstHour: H(1938), lastHour: H(1940), color: '#fff', open: true },
  ] }];
  const model: TimelineModel = { hour: H(1940) as TimelineModel['hour'], firstHour: FIRST, lanes, unobserved: [], iconsOnBars: false, labelWidth: 200, warnings: [], nextChangeAt: Infinity };
  const layout = frameTimeline(placeTimeline(model, 600, 200, {}), H(1940));
  const resolution = resolutionFor(null, model, layout);
  const y = layout.lanes[0]!.y + 10;
  const x = layout.plotLeft + ((H(1938) - FIRST) / (H(1940) - FIRST)) * (layout.plotRight - layout.plotLeft) + 1;
  const hit = hitTimeline(layout, resolution, x, y, 0)!;
  assert.equal(hit.spanId, 'b');
  assert.ok(hit.hour! >= H(1938), 'the hour is the pointer hour, not clamped to the earlier bar');
});

test('consecutive holders with the same name and different recorded ids stay two bars', () => {
  const advisors: CategoryInfo = { token: 'government', name: 'Laws', kind: null, hidden: false, politicsTab: true, slots: [
    { token: 'advisor', name: 'Advisor', character: true, seats: 1, laws: [] },
  ] };
  const looks: IdeaLook[] = [1, 2].map(() => ({ info: idea('adv', 'advisor'), artwork: { token: 'adv', name: 'Same Name', description: null, baseDurationDays: null, icon: null } }));
  const holder = (id: number, look: number, firstHour: number, lastHour: number, end: IdeaHolding['end']): IdeaHolding =>
    ({ token: 'adv', firstHour, lastHour, start: 'seen', end, slot: 'advisor', character: { id, token: String(id), name: 'Same Name' }, look });
  const reply = replyAt(FIRST + 200, [holder(1, 0, FIRST, FIRST + 99, { kind: 'ended' }), holder(2, 1, FIRST + 100, FIRST + 200, { kind: 'held' })], { categories: [advisors], looks });
  const model = ideaTimelineProjection.project(createBoundedReadView({ hour: FIRST + 200, panels: new Map([['main', reply]]) }), OPTIONS);
  assert.equal(model.lanes[0]!.spans.length, 2);
});

test('a hover over an unchanged model neither re-projects nor re-places, and paint work follows the visible rows, not the lane count', () => {
  let projections = 0;
  const lanesOf = (count: number): TimelineLane[] => Array.from({ length: count }, (_, i) => ({
    key: `l${i}`, kind: 'item', label: `Lane ${i}`, auditValue: `Lane ${i}`,
    spans: Array.from({ length: 50 }, (_, j) => ({ id: `s${j}`, firstHour: FIRST + j * 100, lastHour: FIRST + j * 100 + 50, color: '#4f86f7', open: false })),
  }));
  for (const count of [100, 1500]) {
    registerFamilyProjection<TimelineProjection<{ readonly n: number }>>({
  family: 'timeline',
      kind: `test-timeline-${count}`,
      project(view, opt) {
        projections++;
        return { hour: view.hour, firstHour: FIRST, lanes: lanesOf(opt.n), unobserved: [], iconsOnBars: false, labelWidth: 200, warnings: [], nextChangeAt: Infinity };
      },
    });
  }
  const theme = { surface: '#161616', raised: '#1e1e1e', text: '#eeeeee', muted: '#999999', line: '#333333', lineSoft: '#222222', accent: '#4f86f7', blue: '#4f86f7', red: '#e5534b',
    amber: '#d29922', green: '#3fb950', grey: '#999999', hatch: '#666666', hatchEdge: '#444444', font: 'sans-serif', fontSize: { small: 11, body: 12 }, lineWidth: 1, hatchSpacing: 6 } as unknown as FamilyTheme;
  const original = globalThis.OffscreenCanvas;
  let ops = 0;
  const ctx = new Proxy({}, { get: (_target, prop) => {
    if (prop === 'measureText') return (text: string) => ({ width: text.length * 6 });
    if (prop === 'createLinearGradient') return () => ({ addColorStop() {} });
    return () => { ops++; };
  }, set: () => true });
  class FakeCanvas { width: number; height: number; constructor(w: number, h: number) { this.width = w; this.height = h; } getContext() { return ctx; } }
  (globalThis as { OffscreenCanvas: unknown }).OffscreenCanvas = FakeCanvas;
  try {
    const opsFor = (count: number) => {
      const view = familyTimelineLaneView(() => {});
      const state = new Map<string, unknown>();
      const binding: FamilyTimelineBinding<{ readonly n: number }> = { family: 'timeline', kind: `test-timeline-${count}`, options: { n: count }, sources: {},
        width: 600, height: 240, scrollTop: 0, theme, identitySnapshot: null, selectedKey: null, focusedKey: null, expanded: {}, pauseFlush: 0 };
      const input = (inspect: string | null): PaintInput => ({ widget: 1, subs: [], pictures: [], hour: FIRST + 6000, seek: 0, speed: 0, now: 0, inspect,
        binding: { surfaces: [{ width: 600, height: 240, dpr: 1 }] } as unknown as PaintInput['binding'], model: binding });
      let seq = 0;
      const first = view.paint(input(null), state, () => `c${++seq}`);
      const before = projections;
      ops = 0;
      const hovered = view.paint(input(encodeTimelineHit({ kind: 'lane', laneKey: 'l1' })), state, () => `c${++seq}`);
      assert.equal(projections, before, 'a hover does not re-project');
      assert.equal(hovered.fields.nav, first.fields.nav, 'navigation is published once per placement');
      return ops;
    };
    assert.equal(opsFor(1500), opsFor(100), 'paint work does not grow with lanes that are not visible');
  } finally {
    (globalThis as { OffscreenCanvas: unknown }).OffscreenCanvas = original;
  }
});

test('a timeline lane equals a fresh paint after a seek back, a correction and an opened lane, and draws once per day while quiet', () => {
  registerFamilyProjection<TimelineProjection<Record<string, never>>>({
  family: 'timeline',
    kind: 'test-timeline-lane',
    project(view) {
      const lanes = view.panel<{ lanes: TimelineLane[] }>('main')?.lanes ?? [];
      return { hour: view.hour, firstHour: FIRST, lanes, unobserved: [], iconsOnBars: false, labelWidth: 120, warnings: [], nextChangeAt: Infinity };
    },
  });
  const lane = (label: string, open: boolean): TimelineLane => ({ key: 'war', kind: 'item', label, auditValue: label, expandable: true, spans: [{ id: 'a', firstHour: FIRST + 100, lastHour: FIRST + 900, color: '#4f86f7', open }] });
  const child: TimelineLane = { key: 'front', kind: 'item', label: 'Front', auditValue: 'Front', parentKey: 'war', depth: 1, spans: [{ id: 'b', firstHour: FIRST + 200, lastHour: FIRST + 400, color: '#e5534b', open: false }] };
  const binding = (lanes: TimelineLane[], expanded: Record<string, boolean> = {}): FamilyTimelineBinding<Record<string, never>> => ({
    family: 'timeline', kind: 'test-timeline-lane', options: {}, sources: {}, staticPanels: { main: { lanes } }, width: 600, height: 240, scrollTop: 0,
    theme: LANE_THEME, identitySnapshot: null, selectedKey: null, focusedKey: null, expanded, pauseFlush: 0,
  });
  const held = binding([lane('Winter War', true), child]);
  const surface = { width: 600, height: 240, dpr: 1 };
  const at = FIRST + 1000 + 5;
  assert.equal(assertQuietFrames(() => familyTimelineLaneView(() => {}), { label: 'quiet', binding: held, hour: at }, 30, surface), nextDayStart(at));
  assertFreshPaint(() => familyTimelineLaneView(() => {}), [
    { label: 'held', binding: held, hour: at, check: true },
    { label: 'a day later', binding: held, hour: at + 30, speed: 24 },
    { label: 'seek back', binding: held, hour: at, check: true },
    { label: 'later in the same day', binding: held, hour: at + 7, check: true },
    { label: 'correction', binding: binding([lane('Continuation War', true), child]), hour: at, check: true },
    { label: 'opened', binding: binding([lane('Continuation War', true), child], { war: true }), hour: at, check: true },
  ], surface);
});

test('timelineModel forwards footer notes through to lane meta', () => {
  const model = timelineModel({
    hour: H(1940) as TimelineModel['hour'],
    firstHour: FIRST,
    labelWidth: 96,
    summaryText: '4 completed',
    notes: ['Head: 2 in progress · Radio 50%, 10 days left'],
  });
  assert.deepEqual(model.notes, ['Head: 2 in progress · Radio 50%, 10 days left']);
  assert.equal(model.summaryText, '4 completed');
});
