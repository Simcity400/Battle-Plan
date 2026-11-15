import assert from 'node:assert/strict';
import test from 'node:test';
import type { ResearchHistory } from '../research/types';
import { HOUR_EPOCH } from '../../dashboard/calendar.ts';
import { paintResearchSlots, slotNumbers, slotsStructure } from '../research/researchSlots.ts';
import { activeSlots } from './activeResearchData.ts';
import type { ResearchEntry, ResearchSlotNow, ResearchSlotState } from '../../generated/dashboard';
import type { ResearchSlotsAt } from '../research/types';

const base = HOUR_EPOCH + 1944 * 8760 + 12;
const day = (n: number) => base + n * 24;
const state = (token: string | null, entry: number | null, extra: Partial<ResearchSlotState> = {}): ResearchSlotState => ({
  firstHour: day(96), lastHour: day(100), token, entryFirstObservedHour: entry, speedMultiplierRaw: '100000', bankedDaysRaw: '0',
  appliedDaysRaw: '0', aheadReductionYearsRaw: token ? '0' : null, details: null, ...extra,
});
const active: ResearchEntry = { token: 'radar', technologyHandle: 1, databaseIndex: 4, continuityStartHour: day(0), level: 0, firstObservedHour: day(0),
  startedAfterHour: null, lastObservedHour: day(100), endedByHour: null, outcomeObservedHour: null, outcome: 'active' };
const modifiers = (year: number) => ({ year, totalBonusRaw: '0', aheadFactorRaw: '200000', minimumSpeedRaw: '10000', components: [] });
const open = [{ firstHour: day(0), lastHour: day(100) }];
const reply = (hour: number, slots: ResearchSlotNow[], missing: string | null = null): ResearchSlotsAt => ({
  tag: 'MOD', hour, readHour: missing ? null : day(96), missing, slots, warnings: [], grants: { nextId: 0, grants: [], unavailable: null } });

test('each open slot says what it researches, whether it is ahead of time, and its painted numbers count down from the read', () => {
  const history: ResearchHistory = { coverage: open, slots: [
    { index: 0, availability: open, entries: [active], states: [], progress: [] },
    { index: 1, availability: open, entries: [], states: [], progress: [] },
    { index: 2, availability: open, entries: [], states: [], progress: [] },
    { index: 3, availability: [{ firstHour: day(0), lastHour: day(50) }], entries: [], states: [], progress: [] },
  ] };
  const radar: ResearchSlotNow = { index: 0, state: state('radar', day(0), { details: { remainingDays: 40, bonuses: null, modifiers: modifiers(1945) } }),
    progress: { firstHour: day(96), lastHour: day(96), entryFirstObservedHour: day(0), baseDaysRaw: '10000000', progressRaw: '6200000' } };
  const sonar: ResearchSlotNow = { index: 1, state: state('sonar', null, { details: { remainingDays: 12, bonuses: null, modifiers: modifiers(1940) } }), progress: null };
  const idle: ResearchSlotNow = { index: 2, state: state(null, null, { bankedDaysRaw: '1200000' }), progress: null };
  const now = reply(day(100), [radar, sonar, idle]);
  const slots = activeSlots(history, now);
  assert.deepEqual(slots.map(slot => slot.kind), ['researching', 'researching', 'idle'], 'a slot closed by then is not shown');
  const [first, second] = slots;
  assert.ok(first.kind === 'researching' && second.kind === 'researching');
  assert.equal(first.entry?.token, 'radar');
  assert.equal(first.aheadOf, 1945);
  assert.equal(second.entry, null, 'a technology no assignment joins is shown without a history to open');
  assert.equal(second.aheadOf, null, 'its year has passed');
  assert.deepEqual(slotNumbers(radar, day(100)), { fraction: 0.62, percent: '62%', left: 36, days: '36 days left', saved: '' }, 'the estimate counts down from its read');
  assert.equal(slotNumbers(idle, day(100)).saved, '12 days saved');
  assert.equal(slotNumbers(sonar, day(100)).fraction, null, 'no progress is guessed for it');
  // A later read with new numbers renders nothing: only what the slots hold is structure.
  const later = reply(day(101), [{ ...radar, state: { ...radar.state, firstHour: day(101) }, progress: { ...radar.progress!, progressRaw: '6400000' } }, sonar, idle]);
  assert.equal(slotsStructure(later), slotsStructure(now));
  assert.notEqual(slotsStructure(reply(day(101), [radar, sonar])), slotsStructure(now));
});

test('the painter writes a slot\'s numbers into the elements naming it', () => {
  const texts = new Map<string, { textContent: string }>([['percent', { textContent: '' }], ['days', { textContent: '' }], ['saved', { textContent: 'x' }]]);
  const bar = { style: { width: '' } };
  const card = { querySelector: (selector: string) => selector === '[data-paint="bar"]' ? bar : selector.startsWith('[data-paint=') ? texts.get(selector.slice(13, -2)) ?? null : null };
  const root = { querySelectorAll: (selector: string) => selector === '[data-research-slot="0"]' ? [card] : [] } as unknown as Element;
  const read: ResearchSlotNow = { index: 0, state: state('radar', day(0), { details: { remainingDays: 3, bonuses: null, modifiers: null } }),
    progress: { firstHour: day(96), lastHour: day(96), entryFirstObservedHour: day(0), baseDaysRaw: '100000', progressRaw: '50000' } };
  paintResearchSlots(root, reply(day(97), [read]), day(97));
  assert.deepEqual([...texts.values()].map(item => item.textContent), ['50%', '2 days left', '']);
  assert.equal(bar.style.width, '50%');
  // Followed, the progress is the slot's live cell and the days are counted at the head, not at the read.
  paintResearchSlots(root, reply(day(97), [read]), day(98), slot => slot === 0 ? 0.625 : undefined);
  assert.deepEqual([...texts.values()].map(item => item.textContent), ['63%', '1 day left', '']);
  assert.equal(bar.style.width, '62.5%');
});

test('at an hour without a read, slots open at the last read are unavailable rather than guessed', () => {
  const history: ResearchHistory = { coverage: [{ firstHour: day(0), lastHour: day(50) }, { firstHour: day(80), lastHour: day(100) }], slots: [
    { index: 0, availability: [{ firstHour: day(0), lastHour: day(50) }, { firstHour: day(80), lastHour: day(100) }], entries: [], states: [], progress: [] },
    { index: 1, availability: [{ firstHour: day(0), lastHour: day(20) }], entries: [], states: [], progress: [] },
  ] };
  const slots = activeSlots(history, reply(day(60), [], 'No research read covers this hour'));
  assert.deepEqual(slots.map(slot => [slot.kind, slot.slot.index]), [['unavailable', 0]]);
  const covered = activeSlots(history, reply(day(90), [], 'This country\'s research slots were not read together'));
  assert.deepEqual(covered.map(slot => slot.kind === 'unavailable' && slot.reason), ['This country\'s research slots were not read together']);
});
