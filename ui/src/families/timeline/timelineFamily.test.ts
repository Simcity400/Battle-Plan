import assert from 'node:assert/strict';
import test from 'node:test';
import { engineHour } from '../../dashboard/calendar.ts';
import type { CategoryInfo, IdeaHistoryReply, IdeaInfo } from '../../generated/dashboard';
import { ideaTimelineProjection } from '../../widgets/idea-timeline/ideaTimelineProjection.ts';
import { createBoundedReadView } from '../boundedView.ts';
import { timelineHostAdapter } from './hostAdapter.ts';
import { layoutTimeline } from './layout.ts';
import { visibleLanes, type TimelineLane, type TimelineModel } from './model.ts';
import { decodeTimelineHit, encodeTimelineHit, hitTimeline } from './painter.ts';

const H = (year: number, month = 1, day = 1) => engineHour({ year, month, day, hour: 1 })!;
const FIRST = H(1936);

const idea = (token: string, slot: string): IdeaInfo => ({ token, category: 'government', slot, law: true, designer: false,
  effects: [], targeted: [], equipment: [], research: [], traits: [], tooltips: [], effectsKnown: true, unknown: null, nameMissing: null });
const GOVERNMENT: CategoryInfo = { token: 'government', name: 'Laws', kind: null, hidden: false, politicsTab: true, slots: [
  { token: 'economy', name: 'Economy law', character: false, seats: 1, laws: ['civilian', 'war'] },
  { token: 'trade', name: 'Trade law', character: false, seats: 1, laws: ['free', 'autarky'] },
] };
const REPLY: IdeaHistoryReply = {
  branchId: '1', tag: 'GER', asked: 'GER', identityStarts: [], modifiers: [], texts: [], warnings: [], categories: [GOVERNMENT],
  unobserved: [{ firstHour: H(1938, 6), lastHour: H(1938, 10) - 1 }],
  spans: [
    { token: 'civilian', firstHour: FIRST, lastHour: H(1937, 6) - 1, slot: null, character: null },
    { token: 'war', firstHour: H(1937, 6), lastHour: H(1941), slot: null, character: null },
    { token: 'free', firstHour: FIRST, lastHour: H(1938, 6) - 1, slot: null, character: null },
    { token: 'autarky', firstHour: H(1938, 10), lastHour: H(1941), slot: null, character: null },
  ],
  ideas: [idea('civilian', 'economy'), idea('war', 'economy'), idea('free', 'trade'), idea('autarky', 'trade')],
  artwork: [{ token: 'war', name: 'War Economy', description: null, baseDurationDays: null, icon: 'asset://icons/war.png' }],
  holders: [],
};
const OPTIONS = { category: '', iconsOnBars: true, firstHour: FIRST, surface: '#161616' };

function project(hour: number): TimelineModel {
  const readView = createBoundedReadView({ hour, panels: new Map([['main', REPLY]]) });
  return ideaTimelineProjection.project(readView, OPTIONS);
}

const lane = (model: TimelineModel, key: string): TimelineLane => model.lanes.find(item => item.key === key)!;

test('at the displayed hour nothing later is drawn, and a held item runs square into the playhead', () => {
  const at = H(1937, 3);
  const model = project(at);
  assert.deepEqual(model.extent, [FIRST, at]);
  const economy = lane(model, 'economy');
  assert.deepEqual(economy.spans.map(span => [span.label, span.lastHour, span.open]), [['Idea name unavailable', at, true]], 'War Economy starts later and is not drawn');
  assert.equal(economy.subtitle, 'Idea name unavailable');
  assert.equal(economy.spans[0]!.heldBefore, true, 'held when the recording began');
  assert.equal(model.nextChangeAt, H(1937, 6));
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
  const model: TimelineModel = { hour: H(1940) as TimelineModel['hour'], extent: [FIRST, H(1940)], lanes, unobserved: [], iconsOnBars: false, labelWidth: 200, warnings: [], nextChangeAt: Infinity };
  const layout = layoutTimeline(model, 600, 200, {});
  const item = layout.lanes[1]!;
  const hit = hitTimeline(layout, layout.plotRight - 4, item.y + item.height / 2, 0);
  assert.equal(hit?.kind, 'span');
  const token = encodeTimelineHit(hit);
  assert.deepEqual(decodeTimelineHit(token), hit);
  assert.deepEqual(timelineHostAdapter.parseHit(token), { key: 'm', subject: 'm', hour: hit!.hour, kind: 'span', toggle: false, inspectToken: token });
  const toggle = hitTimeline(layout, 20, layout.lanes[0]!.y + 5, 0);
  assert.equal(timelineHostAdapter.parseHit(encodeTimelineHit(toggle))?.toggle, true);
});
