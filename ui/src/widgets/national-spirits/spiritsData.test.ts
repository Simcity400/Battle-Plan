import test from 'node:test';
import assert from 'node:assert/strict';
import { activeAt, endedBy, historyLanes, spiritState, spirits, startUnseen } from './spiritsData.ts';
import type { IdeaHistoryReply, IdeaInfo } from '../../generated/dashboard';

const info = (token: string, category: string): IdeaInfo => ({ token, category, slot: null, law: false, designer: false,
  effects: [], targeted: [], equipment: [], research: [], traits: [], tooltips: [], effectsKnown: true, unknown: null, nameMissing: null });
const REPLY: IdeaHistoryReply = {
  branchId: '1', tag: 'GER', asked: 'GER', unobserved: [], identityStarts: [], modifiers: [], texts: [], warnings: [],
  categories: [
    { token: 'mod_spirits', name: null, kind: 'national_spirit', hidden: false, politicsTab: true, slots: [] },
    { token: 'government', name: null, kind: null, hidden: false, politicsTab: true, slots: [] },
  ],
  ideas: [info('mefo', 'mod_spirits'), info('heer', 'mod_spirits'), info('blitz', 'mod_spirits'), info('war_economy', 'government')],
  spans: [
    { token: 'heer', firstHour: 0, lastHour: 999, slot: null, character: null },
    { token: 'mefo', firstHour: 100, lastHour: 399, slot: null, character: null },
    { token: 'war_economy', firstHour: 0, lastHour: 999, slot: null, character: null },
    { token: 'blitz', firstHour: 500, lastHour: 599, slot: null, character: null },
    { token: 'blitz', firstHour: 700, lastHour: 800, slot: null, character: null },
  ],
  artwork: [{ token: 'mefo', name: 'MEFO Bills', description: null, baseDurationDays: null, icon: null }],
  holders: [],
};
const describe = { since: (hour: number) => `since ${hour}`, before: (hour: number) => `before ${hour}`, ended: (hour: number) => `ended ${hour}`, endedBy: (hour: number) => `ended by ${hour}`, lastSeen: (hour: number) => `last ${hour}` };

test('spirits are the ideas of any national-spirit category, whatever content names it', () => {
  assert.deepEqual(spirits(REPLY).map(spirit => spirit.token).sort(), ['blitz', 'heer', 'mefo']);
  assert.equal(spirits(REPLY).find(spirit => spirit.token === 'mefo')?.name, 'MEFO Bills');
});

test('a spirit is named whether or not its art resolves; one the recording does not name reads "Name not recorded", its reason for the tooltip', () => {
  const missing = 'localisation MOD_heer is missing';
  const reply: IdeaHistoryReply = { ...REPLY,
    ideas: REPLY.ideas.map(item => item.token === 'heer' ? { ...item, nameMissing: missing } as IdeaInfo : item),
    artwork: [...REPLY.artwork, { token: 'blitz', name: 'Blitzkrieg', description: null, baseDurationDays: null, icon: null },
      { token: 'heer', name: null, description: null, baseDurationDays: null, icon: 'data:image/png;base64,AA==' }] };
  const names = new Map(spirits(reply).map(spirit => [spirit.token, spirit.name]));
  assert.equal(names.get('blitz'), 'Blitzkrieg');
  assert.equal(names.get('heer'), 'Name not recorded');
  assert.equal(historyLanes(spirits(reply), 650, 'gained', true, describe)[0].identity?.label, 'Name not recorded');
  const heer = spirits(reply).find(spirit => spirit.token === 'heer');
  assert.equal(heer?.nameMissing, `Name not recorded: ${missing}`);
  assert.equal(spirits(reply).find(spirit => spirit.token === 'blitz')?.nameMissing, null);
});

test('the grid shows spirits held at the hour, in the order they were gained', () => {
  const list = spirits(REPLY);
  assert.deepEqual(activeAt(list, 150).map(spirit => spirit.token), ['heer', 'mefo']);
  assert.deepEqual(activeAt(list, 600).map(spirit => spirit.token), ['heer']);
  assert.deepEqual(activeAt(list, 750).map(spirit => spirit.token), ['heer', 'blitz']);
});

test('history lanes sort by gained, ended or name, and can hide ended spirits', () => {
  const list = spirits(REPLY);
  const blitz = list.find(spirit => spirit.token === 'blitz')!;
  assert.equal(endedBy(blitz, 650), 600);
  assert.equal(endedBy(blitz, 750), null);
  const gained = historyLanes(list, 650, 'gained', true, describe);
  assert.deepEqual(gained.map(lane => [lane.id, lane.label]), [['heer', 'since 0'], ['mefo', 'ended 400'], ['blitz', 'ended 600']]);
  assert.deepEqual(historyLanes(list, 650, 'ended', true, describe).map(lane => lane.id), ['mefo', 'blitz', 'heer']);
  assert.deepEqual(historyLanes(list, 650, 'name', false, describe).map(lane => lane.id), ['heer']);
  assert.deepEqual(historyLanes(list, 50, 'gained', true, describe).map(lane => lane.id), ['heer']);
  assert.equal(gained[0].spans[0].emphasis, true);
  assert.equal(gained[1].spans[0].emphasis, false);
  assert.equal(gained[0].identity?.label, 'National spirit name unavailable');
});

test('a spirit whose next hours were not recorded is unknown, not ended', () => {
  const list = spirits(REPLY);
  const mefo = list.find(spirit => spirit.token === 'mefo')!;
  const gap = [{ firstHour: 400, lastHour: 450 }];
  assert.deepEqual(spiritState(mefo, 420, gap), { kind: 'unknown', lastSeen: 399 });
  assert.equal(endedBy(mefo, 420, gap), null);
  assert.deepEqual(historyLanes(list, 420, 'gained', false, describe, gap).map(lane => [lane.id, lane.label]), [['heer', 'since 0'], ['mefo', 'last 399']]);
});

test('the grid orders spirits by their latest acquisition', () => {
  const reacquired: typeof REPLY = { ...REPLY, spans: [
    { token: 'mefo', firstHour: 0, lastHour: 99, slot: null, character: null },
    { token: 'heer', firstHour: 200, lastHour: 499, slot: null, character: null },
    { token: 'mefo', firstHour: 300, lastHour: 499, slot: null, character: null },
  ] };
  assert.deepEqual(activeAt(spirits(reacquired), 350).map(spirit => spirit.token), ['heer', 'mefo']);
});

test('a spirit already held when recording began, or after an unrecorded stretch, is held before, not since', () => {
  const list = spirits(REPLY);
  assert.equal(startUnseen(0, 0), true);
  assert.equal(startUnseen(700, 0), false);
  assert.equal(startUnseen(700, 0, [{ firstHour: 650, lastHour: 699 }]), true);
  assert.deepEqual(historyLanes(list, 750, 'gained', false, describe, [], 0).map(lane => [lane.id, lane.label]), [['heer', 'before 0'], ['blitz', 'since 700']]);
  assert.deepEqual(historyLanes(list, 750, 'gained', false, describe, [{ firstHour: 650, lastHour: 699 }], 0).map(lane => lane.label), ['before 0', 'before 700']);
});

test('a spirit gone at the first recorded hour after a gap ended by then, at an unknown time', () => {
  const list = spirits(REPLY);
  const mefo = list.find(spirit => spirit.token === 'mefo')!;
  const gap = [{ firstHour: 400, lastHour: 450 }];
  assert.deepEqual(spiritState(mefo, 440, gap), { kind: 'unknown', lastSeen: 399 });
  assert.deepEqual(spiritState(mefo, 460, gap), { kind: 'ended', at: 451, lastSeen: 399 });
  assert.equal(endedBy(mefo, 460, gap), 451);
  assert.deepEqual(historyLanes(list, 460, 'gained', true, describe, gap).map(lane => [lane.id, lane.label]), [['heer', 'since 0'], ['mefo', 'ended by 451']]);
  assert.deepEqual(historyLanes(list, 460, 'gained', false, describe, gap).map(lane => lane.id), ['heer']);
});
