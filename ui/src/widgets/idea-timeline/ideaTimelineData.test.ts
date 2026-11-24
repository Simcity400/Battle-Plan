import test from 'node:test';
import assert from 'node:assert/strict';
import { validOption } from '../../dashboard/options.ts';
import type { OptionField } from '../../dashboard/options';
import { validateIdeaHistoryReply } from '../../dashboard/ideaHistoryReply.ts';
import { chosenCategory, lanesFor, lawLevel, timelineCategories, UNIDENTIFIED_HOLDER } from './ideaTimelineData.ts';
import type { CategoryInfo, IdeaHistoryReply, IdeaInfo } from '../../generated/dashboard';

const idea = (token: string, slot: string | null, category = 'government'): IdeaInfo => ({ token, category, slot, law: false, designer: false,
  effects: [], targeted: [], equipment: [], research: [], traits: [], tooltips: [], effectsKnown: true, unknown: null, nameMissing: null });
const GOVERNMENT: CategoryInfo = { token: 'government', name: 'Laws & Government', kind: null, hidden: false, politicsTab: true, slots: [
  { token: 'economy', name: 'Economy Law', character: false, seats: 1, laws: ['civilian_economy', 'partial_mobilisation', 'war_economy'] },
  { token: 'political_advisor', name: 'Political Advisor', character: true, seats: 2, laws: [] },
] };
const OTHERS: CategoryInfo[] = [
  { token: 'country', name: 'Country', kind: 'national_spirit', hidden: false, politicsTab: true, slots: [] },
  { token: 'army_spirit', name: null, kind: 'army_spirit', hidden: false, politicsTab: false, slots: [{ token: 'army_spirit', name: null, character: false, seats: 1, laws: [] }] },
];
const REPLY: IdeaHistoryReply = {
  branchId: '1', tag: 'GER', asked: 'GER', unobserved: [], identityStarts: [], modifiers: [], texts: [], warnings: [], categories: [GOVERNMENT, ...OTHERS],
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

test('a mod category is offered with its own name, and one the mod removed is gone', () => {
  const modded: CategoryInfo[] = [
    { token: 'mod_council', name: 'Council', kind: null, hidden: false, politicsTab: true, slots: [{ token: 'mod_seat', name: 'Seat', character: false, seats: 1, laws: ['mod_open'] }] },
    { token: 'mod_hidden', name: null, kind: null, hidden: true, politicsTab: true, slots: [{ token: 'x', name: null, character: false, seats: 1, laws: [] }] },
    ...OTHERS,
  ];
  assert.deepEqual(timelineCategories(modded).map(category => category.token), ['mod_council']);
  assert.equal(chosenCategory(modded, '')?.token, 'mod_council');
  assert.equal(chosenCategory(modded, 'government'), null);
  const reply = { ...REPLY, categories: modded, spans: [{ token: 'mod_open', firstHour: 0, lastHour: 10, slot: null, character: null }], ideas: [idea('mod_open', 'mod_seat', 'mod_council')] };
  const lanes = lanesFor(reply, modded[0], { laws: 'both', holders: 'both' });
  assert.deepEqual(lanes.map(lane => [lane.label, lane.spans.map(span => span.label)]), [['Seat', ['Idea name unavailable']]]);
});

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
  assert.deepEqual(lanes[0].spans.map(span => [span.label, span.shade]), [['Idea name unavailable', 0], ['War Economy', 1]]);
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



test('content choices keep raw scalar keys, and malformed replies are refused', () => {
  const field: OptionField = { key: 'category', section: 'options', label: 'Category', kind: 'content-choice', automatic: 'First', default: '' };
  assert.equal(validOption(field, ''), true);
  assert.equal(validOption(field, 'research_production'), true);
  assert.equal(validOption(field, 'mod category,slot:20:'), true);
  assert.equal(validOption(field, 3), false);
  assert.throws(() => validateIdeaHistoryReply({ spans: [{ token: 'x', firstHour: 5, lastHour: 1, slot: null, character: null }] }));
  assert.equal(validateIdeaHistoryReply(REPLY), REPLY);
});

test('an advisor span shows its own recorded holder, not the idea’s latest one', () => {
  const art = (name: string) => ({ token: 'schacht', name, description: null, baseDurationDays: null, icon: null });
  const reply = { ...REPLY,
    spans: [
      { token: 'schacht', firstHour: 10, lastHour: 100, slot: 'political_advisor', character: { id: 1, token: 'A', name: 'A' } },
      { token: 'schacht', firstHour: 101, lastHour: 300, slot: 'political_advisor', character: { id: 2, token: 'B', name: 'B' } },
    ],
    ideas: [idea('schacht', 'political_advisor')],
    artwork: [art('Holder B')],
    holders: [{ token: 'schacht', character: 1, characterToken: 'A', name: 'A', artwork: art('Holder A') }, { token: 'schacht', character: 2, characterToken: 'B', name: 'B', artwork: art('Holder B') }] };
  const seat = lanesFor(reply, GOVERNMENT, { laws: 'both', holders: 'both' }).find(lane => lane.id === 'political_advisor#0')!;
  assert.deepEqual(seat.spans.map(span => span.label), ['Holder A', 'Holder B']);
});

test('a holder the recording could not identify stays unnamed, not the latest holder', () => {
  const art = (name: string) => ({ token: 'schacht', name, description: null, baseDurationDays: null, icon: null });
  const reply = { ...REPLY,
    spans: [
      { token: 'schacht', firstHour: 10, lastHour: 100, slot: 'political_advisor', character: { id: 1, token: '', name: '' } },
      { token: 'schacht', firstHour: 101, lastHour: 300, slot: 'political_advisor', character: { id: 2, token: 'B', name: 'B' } },
    ],
    ideas: [idea('schacht', 'political_advisor')],
    artwork: [art('Holder B')],
    holders: [{ token: 'schacht', character: 2, characterToken: 'B', name: 'B', artwork: art('Holder B') }] };
  const seat = lanesFor(reply, GOVERNMENT, { laws: 'both', holders: 'both' }).find(lane => lane.id === 'political_advisor#0')!;
  assert.deepEqual(seat.spans.map(span => span.label), [UNIDENTIFIED_HOLDER, 'Holder B']);
});

test('a renamed holder shows each recorded name for its own span', () => {
  const art = (name: string) => ({ token: 'schacht', name, description: null, baseDurationDays: null, icon: null });
  const reply = { ...REPLY,
    spans: [
      { token: 'schacht', firstHour: 10, lastHour: 20, slot: 'political_advisor', character: { id: 7, token: 'X', name: 'Old Name' } },
      { token: 'schacht', firstHour: 21, lastHour: 30, slot: 'political_advisor', character: { id: 7, token: 'X', name: 'New Name' } },
    ],
    ideas: [idea('schacht', 'political_advisor')],
    artwork: [art('New Name')],
    holders: [{ token: 'schacht', character: 7, characterToken: 'X', name: 'Old Name', artwork: art('Old Name') }, { token: 'schacht', character: 7, characterToken: 'X', name: 'New Name', artwork: art('New Name') }] };
  const seat = lanesFor(reply, GOVERNMENT, { laws: 'both', holders: 'both' }).find(lane => lane.id === 'political_advisor#0')!;
  assert.deepEqual(seat.spans.map(span => span.label), ['Old Name', 'New Name']);
});

test('holders whose texts contain separators never share a key', () => {
  const art = (name: string) => ({ token: 'schacht', name, description: null, baseDurationDays: null, icon: null });
  const reply = { ...REPLY,
    spans: [
      { token: 'schacht', firstHour: 10, lastHour: 20, slot: 'political_advisor', character: { id: 1, token: 'char', name: 'Alice#2#other#Bob' } },
      { token: 'schacht', firstHour: 21, lastHour: 30, slot: 'political_advisor', character: { id: 2, token: 'other', name: 'Bob' } },
    ],
    ideas: [idea('schacht', 'political_advisor')],
    artwork: [art('Bob')],
    holders: [{ token: 'schacht', character: 1, characterToken: 'char', name: 'Alice#2#other#Bob', artwork: art('Alice') },
      { token: 'schacht', character: 2, characterToken: 'other', name: 'Bob', artwork: art('Bob') }] };
  const seat = lanesFor(reply, GOVERNMENT, { laws: 'both', holders: 'both' }).find(lane => lane.id === 'political_advisor#0')!;
  assert.deepEqual(seat.spans.map(span => span.label), ['Alice', 'Bob']);
});

test('an advisor whose details resolve later stays in its seat', () => {
  const art = (name: string) => ({ token: 'b', name, description: null, baseDurationDays: null, icon: null });
  const reply = { ...REPLY,
    spans: [
      { token: 'a', firstHour: 10, lastHour: 20, slot: 'political_advisor', character: { id: 1, token: 'A', name: 'A' } },
      { token: 'b', firstHour: 11, lastHour: 30, slot: 'political_advisor', character: { id: 2, token: '', name: 'B' } },
      { token: 'b', firstHour: 31, lastHour: 40, slot: 'political_advisor', character: { id: 2, token: 'B', name: 'B' } },
    ],
    ideas: [idea('a', 'political_advisor'), idea('b', 'political_advisor')],
    artwork: [art('B')], holders: [] };
  const lanes = lanesFor(reply, GOVERNMENT, { laws: 'both', holders: 'both' }).filter(lane => lane.id.startsWith('political_advisor#'));
  assert.deepEqual(lanes.map(lane => lane.spans.map(span => span.firstHour)), [[10], [11, 31]]);
});

test('a newcomer starting the same hour never takes the seat a continuing holder keeps', () => {
  const reply = { ...REPLY,
    spans: [
      { token: 'b', firstHour: 10, lastHour: 20, slot: 'political_advisor', character: { id: 2, token: '', name: 'B' } },
      { token: 'c', firstHour: 10, lastHour: 20, slot: 'political_advisor', character: { id: 3, token: 'C', name: 'C' } },
      { token: 'a', firstHour: 21, lastHour: 40, slot: 'political_advisor', character: { id: 1, token: 'A', name: 'A' } },
      { token: 'b', firstHour: 21, lastHour: 40, slot: 'political_advisor', character: { id: 2, token: 'B', name: 'B' } },
    ],
    ideas: [idea('a', 'political_advisor'), idea('b', 'political_advisor'), idea('c', 'political_advisor')], artwork: [], holders: [] };
  const lanes = lanesFor(reply, GOVERNMENT, { laws: 'both', holders: 'both' }).filter(lane => lane.id.startsWith('political_advisor#'));
  assert.deepEqual(lanes.map(lane => lane.spans.map(span => span.id)), [['b:10', 'b:21'], ['c:10', 'a:21']]);
});
