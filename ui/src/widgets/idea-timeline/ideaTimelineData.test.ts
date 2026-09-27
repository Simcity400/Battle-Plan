import test from 'node:test';
import assert from 'node:assert/strict';
import type { CategoryInfo, IdeaHistoryReply, IdeaInfo } from '../../generated/dashboard';
import { AXIS_HEIGHT, ROW_HEIGHT, barHours, inSpans, laneAt, leftColumn, spanAt, spansExtent } from '../../charts/timeline/layout.ts';
import { validOption } from '../../dashboard/options.ts';
import type { OptionField } from '../../dashboard/options';
import { validateIdeaHistoryReply } from '../../dashboard/ideaHistoryReply.ts';
import { chosenCategory, lanesFor, lawLevel, timelineCategories, UNIDENTIFIED_HOLDER } from './ideaTimelineData.ts';

const idea = (token: string, slot: string | null, category = 'government'): IdeaInfo => ({ token, category, slot, law: false, designer: false,
  effects: [], targeted: [], equipment: [], research: [], traits: [], tooltips: [] });
const GOVERNMENT: CategoryInfo = { token: 'government', name: 'Laws & Government', kind: null, hidden: false, politicsTab: true, slots: [
  { token: 'economy', name: 'Economy Law', character: false, seats: 1, laws: ['civilian_economy', 'partial_mobilisation', 'war_economy'] },
  { token: 'political_advisor', name: 'Political Advisor', character: true, seats: 2, laws: [] },
] };
const OTHERS: CategoryInfo[] = [
  { token: 'country', name: 'Country', kind: 'national_spirit', hidden: false, politicsTab: true, slots: [] },
  { token: 'army_spirit', name: null, kind: 'army_spirit', hidden: false, politicsTab: false, slots: [{ token: 'army_spirit', name: null, character: false, seats: 1, laws: [] }] },
];
const REPLY: IdeaHistoryReply = {
  branchId: '1', tag: 'GER', unobserved: [], warnings: [], categories: [GOVERNMENT, ...OTHERS],
  spans: [
    { token: 'civilian_economy', firstHour: 0, lastHour: 99, slot: null, character: null },
    { token: 'war_economy', firstHour: 100, lastHour: 500, slot: null, character: null },
    { token: 'schacht', firstHour: 10, lastHour: 300, slot: null, character: null },
    { token: 'todt', firstHour: 50, lastHour: 400, slot: null, character: null },
    { token: 'speer', firstHour: 320, lastHour: 600, slot: null, character: null },
  ],
  ideas: [idea('civilian_economy', 'economy'), idea('war_economy', 'economy'), idea('schacht', 'political_advisor'), idea('todt', 'political_advisor'), idea('speer', 'political_advisor')],
  artwork: [{ token: 'war_economy', name: 'War Economy', description: null, baseDurationDays: null, icon: null }],
  holders: [],
};

test('only political-screen categories with slots are offered, the first one by default', () => {
  assert.deepEqual(timelineCategories(REPLY.categories).map(category => category.token), ['government']);
  assert.equal(chosenCategory(REPLY.categories, '')?.token, 'government');
  assert.equal(chosenCategory(REPLY.categories, 'army_spirit'), null);
  assert.equal(chosenCategory(REPLY.categories, 'missing'), null);
});

test('law slots shade by escalation and advisor seats take the first free seat', () => {
  const lanes = lanesFor(REPLY, GOVERNMENT, { laws: 'both', holders: 'both' });
  assert.deepEqual(lanes.map(lane => [lane.id, lane.label, lane.spans.length]), [
    ['economy', 'Economy Law', 2], ['political_advisor#0', 'Political Advisor', 2], ['political_advisor#1', 'Political Advisor', 1],
  ]);
  assert.deepEqual(lanes[0].spans.map(span => [span.label, span.shade]), [['civilian_economy', 0], ['War Economy', 1]]);
  // Schacht leaves at 300, so Speer (from 320) takes his seat while Todt keeps the second.
  assert.deepEqual(lanes[1].spans.map(span => span.id), ['schacht:10', 'speer:320']);
  assert.deepEqual(lanes[2].spans.map(span => span.id), ['todt:50']);
  const icons = lanesFor(REPLY, GOVERNMENT, { laws: 'icons', holders: 'name' });
  assert.equal(icons[0].spans[1].shade, undefined);
  assert.deepEqual([icons[0].icons, icons[1].icons, icons[1].names], [true, false, true]);
  assert.equal(lawLevel(GOVERNMENT, REPLY.ideas[1]), '3 of 3');
  assert.equal(lawLevel(GOVERNMENT, REPLY.ideas[2]), null);
});

test('holders beyond the seats get their own lane instead of overlapping', () => {
  const crowded = { ...REPLY, spans: [...REPLY.spans, { token: 'extra', firstHour: 60, lastHour: 70, slot: null, character: null }], ideas: [...REPLY.ideas, idea('extra', 'political_advisor')] };
  assert.equal(lanesFor(crowded, GOVERNMENT, { laws: 'both', holders: 'both' }).length, 4);
});

test('a recorded seat on a span places it, ahead of content', () => {
  const moved = { ...REPLY, spans: [...REPLY.spans, { token: 'schacht', firstHour: 700, lastHour: 800, slot: 'economy', character: null }] };
  assert.equal(lanesFor(moved, GOVERNMENT, { laws: 'both', holders: 'both' })[0].spans.length, 3);
});

test('the left column never takes more than 40% of the chart, and unobserved hours are known', () => {
  assert.equal(leftColumn(280, 1000), 280);
  assert.equal(leftColumn(280, 300), 120);
  assert.equal(leftColumn(280, 200), 96);
  assert.equal(inSpans([{ firstHour: 140, lastHour: 159 }], 150), true);
  assert.equal(inSpans(undefined, 150), false);
});

test('timeline layout: lanes by row, spans cover their last observed day, bars stop at the playhead', () => {
  assert.equal(laneAt(AXIS_HEIGHT - 1, 3), null);
  assert.equal(laneAt(AXIS_HEIGHT + ROW_HEIGHT * 1.5, 3), 1);
  assert.equal(laneAt(AXIS_HEIGHT + ROW_HEIGHT * 3, 3), null);
  const spans = [{ id: 'a', firstHour: 10, lastHour: 20, label: 'A' }];
  assert.equal(spanAt(spans, 20.5)?.id, 'a');
  assert.equal(spanAt(spans, 21), null);
  assert.deepEqual(barHours(spans[0], null), [10, 21]);
  assert.deepEqual(barHours(spans[0], 15), [10, 15]);
  assert.equal(barHours(spans[0], 5), null);
  assert.deepEqual(spansExtent([{ id: 'l', label: 'L', spans }]), [10, 21]);
});

test('content choices keep any short token, and malformed replies are refused', () => {
  const field: OptionField = { key: 'category', section: 'options', label: 'Category', kind: 'content-choice', automatic: 'First', default: '' };
  assert.equal(validOption(field, ''), true);
  assert.equal(validOption(field, 'research_production'), true);
  assert.equal(validOption(field, 'bad token!'), false);
  assert.equal(validOption(field, 3), false);
  assert.throws(() => validateIdeaHistoryReply({ spans: [{ token: 'x', firstHour: 5, lastHour: 1, slot: null, character: null }] }));
  assert.equal(validateIdeaHistoryReply(REPLY), REPLY);
});

test('an advisor span shows its own recorded holder, not the idea’s latest one', () => {
  const art = (name: string) => ({ token: 'schacht', name, description: null, baseDurationDays: null, icon: null });
  const reply = { ...REPLY,
    spans: [
      { token: 'schacht', firstHour: 10, lastHour: 100, slot: 'political_advisor', character: { id: 1, token: 'A' } },
      { token: 'schacht', firstHour: 101, lastHour: 300, slot: 'political_advisor', character: { id: 2, token: 'B' } },
    ],
    ideas: [idea('schacht', 'political_advisor')],
    artwork: [art('Holder B')],
    holders: [{ token: 'schacht', character: 1, artwork: art('Holder A') }, { token: 'schacht', character: 2, artwork: art('Holder B') }] };
  const seat = lanesFor(reply, GOVERNMENT, { laws: 'both', holders: 'both' }).find(lane => lane.id === 'political_advisor#0')!;
  assert.deepEqual(seat.spans.map(span => span.label), ['Holder A', 'Holder B']);
});

test('a holder the recording could not identify stays unnamed, not the latest holder', () => {
  const art = (name: string) => ({ token: 'schacht', name, description: null, baseDurationDays: null, icon: null });
  const reply = { ...REPLY,
    spans: [
      { token: 'schacht', firstHour: 10, lastHour: 100, slot: 'political_advisor', character: { id: 1, token: '' } },
      { token: 'schacht', firstHour: 101, lastHour: 300, slot: 'political_advisor', character: { id: 2, token: 'B' } },
    ],
    ideas: [idea('schacht', 'political_advisor')],
    artwork: [art('Holder B')],
    holders: [{ token: 'schacht', character: 2, artwork: art('Holder B') }] };
  const seat = lanesFor(reply, GOVERNMENT, { laws: 'both', holders: 'both' }).find(lane => lane.id === 'political_advisor#0')!;
  assert.deepEqual(seat.spans.map(span => span.label), [UNIDENTIFIED_HOLDER, 'Holder B']);
});
