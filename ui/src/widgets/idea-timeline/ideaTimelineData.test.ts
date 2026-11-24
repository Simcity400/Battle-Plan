import test from 'node:test';
import assert from 'node:assert/strict';
import { validOption } from '../../dashboard/options.ts';
import type { OptionField } from '../../dashboard/options';
import { chosenCategory, lanesFor, lawLevel, timelineCategories } from './ideaTimelineData.ts';
import type { CategoryInfo, IdeaCharacter, IdeaHistoryReply, IdeaHolding, IdeaInfo, IdeaLook } from '../../generated/dashboard';

const idea = (token: string, slot: string | null, category: string | null = 'government'): IdeaInfo => ({ token, category, slot, law: false, designer: false,
  effects: [], targeted: [], equipment: [], research: [], traits: [], tooltips: [], effectsKnown: true, unknown: null, nameMissing: null });
const GOVERNMENT: CategoryInfo = { token: 'government', name: 'Laws & Government', kind: null, hidden: false, politicsTab: true, slots: [
  { token: 'economy', name: 'Economy Law', character: false, seats: 1, laws: ['low_levy', 'mid_levy', 'high_levy'] },
  { token: 'political_advisor', name: 'Political Advisor', character: true, seats: 2, laws: [] },
] };
const OTHERS: CategoryInfo[] = [
  { token: 'country', name: 'Country', kind: 'national_spirit', hidden: false, politicsTab: true, slots: [] },
  { token: 'army_spirit', name: null, kind: 'army_spirit', hidden: false, politicsTab: false, slots: [{ token: 'army_spirit', name: null, character: false, seats: 1, laws: [] }] },
];

interface Held {
  readonly token: string;
  readonly first: number;
  readonly last: number;
  readonly slot?: string | null;
  readonly character?: IdeaCharacter;
  readonly info?: IdeaInfo;
  readonly name?: string | null;
}

function replyOf(hour: number, items: readonly Held[], categories: readonly CategoryInfo[] = [GOVERNMENT, ...OTHERS]): IdeaHistoryReply {
  const looks: IdeaLook[] = [];
  const holdings: IdeaHolding[] = items.map(item => {
    looks.push({ info: item.info ?? idea(item.token, 'political_advisor'), artwork: { token: item.token, name: item.name ?? null, description: null, baseDurationDays: null, icon: null } });
    const last = Math.min(item.last, hour);
    return { token: item.token, firstHour: item.first, lastHour: last, start: 'seen', end: item.last >= hour ? { kind: 'held' } : { kind: 'ended' },
      slot: item.slot ?? null, character: item.character ?? null, look: looks.length - 1 };
  });
  const held = holdings.flatMap((holding, at) => holding.end.kind === 'held' ? [at] : []);
  return { branchId: '1', tag: 'GER', asked: 'GER', hour, nextChange: null, unrecorded: null, holdings, held, looks, categories: [...categories],
    unobserved: [], modifiers: [], texts: [], warnings: [] };
}

const REPLY = replyOf(1000, [
  { token: 'low_levy', first: 0, last: 99, info: idea('low_levy', 'economy') },
  { token: 'high_levy', first: 100, last: 500, info: idea('high_levy', 'economy'), name: 'High Levy' },
  { token: 'adv_one', first: 10, last: 300 },
  { token: 'adv_two', first: 50, last: 400 },
  { token: 'adv_three', first: 320, last: 600 },
]);

test('a mod category is offered with its own name, and one the mod removed is gone', () => {
  const modded: CategoryInfo[] = [
    { token: 'mod_council', name: 'Council', kind: null, hidden: false, politicsTab: true, slots: [{ token: 'mod_seat', name: 'Seat', character: false, seats: 1, laws: ['mod_open'] }] },
    { token: 'mod_hidden', name: null, kind: null, hidden: true, politicsTab: true, slots: [{ token: 'x', name: null, character: false, seats: 1, laws: [] }] },
    ...OTHERS,
  ];
  assert.deepEqual(timelineCategories(modded).map(category => category.token), ['mod_council']);
  assert.equal(chosenCategory(modded, '')?.token, 'mod_council');
  assert.equal(chosenCategory(modded, 'government'), null);
  const reply = replyOf(100, [{ token: 'mod_open', first: 0, last: 10, info: idea('mod_open', 'mod_seat', 'mod_council') }], modded);
  assert.deepEqual(lanesFor(reply, modded[0]!).map(lane => [lane.label, lane.spans.map(span => span.label)]), [['Seat', ['Idea name unavailable']]]);
});

test('only political-screen categories with slots are offered, the first one by default', () => {
  assert.deepEqual(timelineCategories(REPLY.categories).map(category => category.token), ['government']);
  assert.equal(chosenCategory(REPLY.categories, '')?.token, 'government');
  assert.equal(chosenCategory(REPLY.categories, 'army_spirit'), null);
  assert.equal(chosenCategory(REPLY.categories, 'missing'), null);
});

test('law slots keep one lane and advisor seats take the first free seat', () => {
  const lanes = lanesFor(REPLY, GOVERNMENT);
  assert.deepEqual(lanes.map(lane => [lane.id, lane.label, lane.spans.length]), [
    ['economy', 'Economy Law', 2], ['political_advisor#0', 'Political Advisor', 2], ['political_advisor#1', 'Political Advisor', 1],
  ]);
  assert.deepEqual(lanes[0]!.spans.map(span => span.label), ['Idea name unavailable', 'High Levy']);
  assert.deepEqual(lanes[1]!.spans.map(span => span.id), ['adv_one:10', 'adv_three:320']);
  assert.deepEqual(lanes[2]!.spans.map(span => span.id), ['adv_two:50']);
  assert.equal(lawLevel(GOVERNMENT, REPLY.looks[1]!.info), '3 of 3');
  assert.equal(lawLevel(GOVERNMENT, REPLY.looks[2]!.info), null);
});

test('holders beyond the seats get their own lane instead of overlapping', () => {
  const crowded = replyOf(1000, [{ token: 'adv_one', first: 10, last: 300 }, { token: 'adv_two', first: 50, last: 400 }, { token: 'extra', first: 60, last: 70 }]);
  assert.equal(lanesFor(crowded, GOVERNMENT).length, 4);
});

test('a recorded seat on a holding places it, ahead of the seat its look names', () => {
  const moved = replyOf(1000, [{ token: 'adv_one', first: 700, last: 800, slot: 'economy' }]);
  assert.equal(lanesFor(moved, GOVERNMENT)[0]!.spans.length, 1);
});

test('content choices keep raw scalar keys', () => {
  const field: OptionField = { key: 'category', section: 'options', label: 'Category', kind: 'content-choice', automatic: 'First', default: '' };
  assert.equal(validOption(field, ''), true);
  assert.equal(validOption(field, 'research_production'), true);
  assert.equal(validOption(field, 'mod category,slot:20:'), true);
  assert.equal(validOption(field, 3), false);
});

test('each advisor holding is labelled from its own look, so a later or renamed holder names nothing earlier', () => {
  const holder = (id: number, token: string, name: string): IdeaCharacter => ({ id, token, name });
  const reply = replyOf(300, [
    { token: 'adv_one', first: 10, last: 20, slot: 'political_advisor', character: holder(7, 'X', 'Old Name'), name: 'Old Name' },
    { token: 'adv_one', first: 21, last: 100, slot: 'political_advisor', character: holder(7, 'X', 'New Name'), name: 'New Name' },
    { token: 'adv_one', first: 101, last: 200, slot: 'political_advisor', character: holder(8, '', ''), name: null },
    { token: 'adv_one', first: 201, last: 300, slot: 'political_advisor', character: holder(9, 'B', 'B'), name: 'Holder B' },
  ]);
  const seat = lanesFor(reply, GOVERNMENT).find(lane => lane.id === 'political_advisor#0')!;
  assert.deepEqual(seat.spans.map(span => span.label), ['Old Name', 'New Name', 'Holder not recorded', 'Holder B']);
});

test('a newcomer starting the same hour never takes the seat a continuing holder keeps', () => {
  const reply = replyOf(1000, [
    { token: 'b', first: 10, last: 20, slot: 'political_advisor', character: { id: 2, token: '', name: 'B' } },
    { token: 'c', first: 10, last: 20, slot: 'political_advisor', character: { id: 3, token: 'C', name: 'C' } },
    { token: 'a', first: 21, last: 40, slot: 'political_advisor', character: { id: 1, token: 'A', name: 'A' } },
    { token: 'b', first: 21, last: 40, slot: 'political_advisor', character: { id: 2, token: 'B', name: 'B' } },
  ]);
  const lanes = lanesFor(reply, GOVERNMENT).filter(lane => lane.id.startsWith('political_advisor#'));
  assert.deepEqual(lanes.map(lane => lane.spans.map(span => span.id)), [['b:10', 'b:21'], ['c:10', 'a:21']]);
});

test('a slot that holds several items at once gets a lane per item held together, so no lane overlaps itself', () => {
  const council: CategoryInfo = { token: 'mod_council', name: 'Council', kind: null, hidden: false, politicsTab: true, slots: [
    { token: 'mod_faction', name: 'Internal faction', character: false, seats: 1, laws: [] },
  ] };
  const reply = replyOf(1000, [
    { token: 'a', first: 0, last: 100, info: idea('a', 'mod_faction', 'mod_council') },
    { token: 'b', first: 0, last: 100, info: idea('b', 'mod_faction', 'mod_council') },
    { token: 'c', first: 101, last: 200, info: idea('c', 'mod_faction', 'mod_council') },
  ], [council]);
  assert.deepEqual(lanesFor(reply, council).map(lane => [lane.id, lane.spans.map(span => span.holding.token)]), [['mod_faction', ['a', 'c']], ['mod_faction#1', ['b']]]);
});
