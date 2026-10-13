import assert from 'node:assert/strict';
import test from 'node:test';
import type { ResearchEntry, ResearchHistory, ResearchSlotState } from '../../generated/dashboard';
import { HOUR_EPOCH } from '../../dashboard/calendar.ts';
import { researchAt } from '../research/researchData.ts';
import { coverageAt } from '../research/researchLayout.ts';
import { activeSlots } from './activeResearchData.ts';

const base = HOUR_EPOCH + 1944 * 8760 + 12;
const day = (n: number) => base + n * 24;
const state = (token: string | null, entry: number | null, extra: Partial<ResearchSlotState> = {}): ResearchSlotState => ({
  firstHour: day(0), lastHour: day(100), token, entryFirstObservedHour: entry, speedMultiplierRaw: '100000', bankedDaysRaw: '0',
  appliedDaysRaw: '0', aheadReductionYearsRaw: token ? '0' : null, ...extra,
});
const active: ResearchEntry = { token: 'radar', technologyHandle: 1, continuityStartHour: day(0), level: 0, firstObservedHour: day(0),
  startedAfterHour: null, lastObservedHour: day(100), endedByHour: null, outcome: 'active' };
const modifiers = (year: number) => ({ year, totalBonusRaw: '0', aheadFactorRaw: '200000', minimumSpeedRaw: '10000', components: [] });

test('each open slot says what it researches, how far, the game’s estimate and whether it is ahead of time', () => {
  const history: ResearchHistory = { coverage: [{ firstHour: day(0), lastHour: day(100) }], slots: [
    { index: 0, availability: [{ firstHour: day(0), lastHour: day(100) }], entries: [active],
      states: [state('radar', day(0), { lastHour: day(95) }), state('radar', day(0), { firstHour: day(96), details: { remainingDays: 40, bonuses: null, modifiers: modifiers(1945) } })],
      progress: [{ firstHour: day(96), lastHour: day(96), entryFirstObservedHour: day(0), baseDaysRaw: '10000000', progressRaw: '6200000' }] },
    { index: 1, availability: [{ firstHour: day(0), lastHour: day(100) }], entries: [],
      states: [state('sonar', null, { details: { remainingDays: 12, bonuses: null, modifiers: modifiers(1940) } })],
      progress: [{ firstHour: day(99), lastHour: day(99), entryFirstObservedHour: null, baseDaysRaw: '10000000', progressRaw: '5000000' }] },
    { index: 2, availability: [{ firstHour: day(0), lastHour: day(100) }], entries: [], states: [state(null, null, { bankedDaysRaw: '1200000' })], progress: [] },
    { index: 3, availability: [{ firstHour: day(0), lastHour: day(50) }], entries: [], states: [state(null, null, { lastHour: day(50) })], progress: [] },
  ] };
  const slots = activeSlots(researchAt(history, day(100)), coverageAt(history.coverage, day(100)), day(100));
  assert.deepEqual(slots.map(slot => slot.kind), ['researching', 'researching', 'idle'], 'a slot closed by then is not shown');
  const [radar, sonar, idle] = slots;
  assert.ok(radar.kind === 'researching' && sonar.kind === 'researching' && idle.kind === 'idle');
  assert.equal(radar.entry?.token, 'radar');
  assert.deepEqual(radar.progress, { fraction: 0.62, hour: day(96) }, 'the latest read, dated by that read');
  assert.equal(radar.left, 36, 'the estimate counts down from its read');
  assert.equal(radar.aheadOf, 1945);
  assert.equal(sonar.entry, null, 'an older recording names it but cannot join its assignment');
  assert.deepEqual(sonar.progress, { fraction: 0.5, hour: day(99) });
  assert.equal(sonar.aheadOf, null, 'its year has passed');
  assert.equal(idle.savedRaw, 1200000n);
});

test('at an hour without a read, slots open at the last read are unavailable rather than guessed', () => {
  const history: ResearchHistory = { coverage: [{ firstHour: day(0), lastHour: day(50) }, { firstHour: day(80), lastHour: day(100) }], slots: [
    { index: 0, availability: [{ firstHour: day(0), lastHour: day(50) }, { firstHour: day(80), lastHour: day(100) }], entries: [],
      states: [state('radar', null, { lastHour: day(50) }), state('radar', null, { firstHour: day(80) })], progress: [] },
    { index: 1, availability: [{ firstHour: day(0), lastHour: day(20) }], entries: [], states: [state(null, null, { lastHour: day(20) })], progress: [] },
  ] };
  const slots = activeSlots(researchAt(history, day(60)), coverageAt(history.coverage, day(60)), day(60));
  assert.deepEqual(slots.map(slot => [slot.kind, slot.slot.index]), [['unavailable', 0]]);
});
