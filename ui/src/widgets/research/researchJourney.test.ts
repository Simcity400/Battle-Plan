import assert from 'node:assert/strict';
import test from 'node:test';
import type { ResearchSlotHistory } from './types';
import { completedResearchJourney, researchJourney, researchTimeSplit, savedResearchCredit } from './researchJourney.ts';
import type { ResearchTimeSplit } from './researchJourney';
import { researchDaysLeftMissing, researchTimeRows } from './researchTimeBar.ts';
import { researchAt } from './researchData.ts';
import type { ResearchEntry, ResearchSlotState } from '../../generated/dashboard';

const start = 43_800_000 + 1938 * 8760;
function assignment(index: number, from: number, to: number, speed: bigint, saved: bigint, initial: bigint, outcome: ResearchEntry['outcome']): ResearchSlotHistory {
  const firstHour = start + from * 24, endHour = start + to * 24;
  const entry: ResearchEntry = {
    token: 'radio', technologyHandle: 7, databaseIndex: 7, continuityStartHour: start, level: 0,
    firstObservedHour: firstHour, startedAfterHour: firstHour - 1,
    lastObservedHour: outcome === 'active' ? endHour : endHour - 1,
    endedByHour: outcome === 'active' ? null : endHour, outcomeObservedHour: null, outcome,
  };
  const state: ResearchSlotState = {
    firstHour, lastHour: outcome === 'active' ? endHour : endHour - 1, entryFirstObservedHour: firstHour, token: 'radio', databaseIndex: 7,
    speedMultiplierRaw: speed.toString(), appliedDaysRaw: saved.toString(), bankedDaysRaw: '0', aheadReductionYearsRaw: '0',
    details: { remainingDays: null, estimateRaw: null, bonuses: [], modifiers: {
      components: [], totalBonusRaw: (speed - 100000n).toString(), year: 1938, aheadFactorRaw: '200000', minimumSpeedRaw: '10000',
    } },
  };
  return {
    index, entries: [entry], states: [state], availability: [{ firstHour: start, lastHour: endHour }],
    progress: Array.from({ length: to - from + (outcome === 'switched' ? 0 : 1) }, (_, day) => ({
      firstHour: firstHour + day * 24, lastHour: Math.min(firstHour + (day + 1) * 24 - 1, endHour),
      entryFirstObservedHour: firstHour, baseDaysRaw: '18000000', progressRaw: (initial + BigInt(day) * speed).toString(),
    })),
  };
}

test('juggling into a banked slot carries prior progress and adds the converted credit once', () => {
  const first = assignment(0, 0, 60, 100000n, 0n, 0n, 'switched');
  const second = assignment(1, 60, 110, 150000n, 3000000n, 10500000n, 'completed');
  const slots = [first, second];
  const result = researchJourney(slots, second, second.entries[0]);
  assert.equal(result.parts.length, 2);
  assert.equal(result.firstHour, start);
  assert.equal(result.endHour, start + 110 * 24);
  assert.deepEqual(result.credits.map(c => [c.slotIndex, c.work]), [[1, 4500000n]]);
  assert.equal(result.comparison?.rateGain, 13500000n);
  assert.equal(result.comparison?.creditGain, 4500000n);
  assert.equal(result.comparison?.difference, 0n);
  assert.deepEqual(slots.map(s => s.entries.filter(e => e.outcome === 'completed').length), [0, 1]);
  assert.equal(slots[0].entries[0].outcome, 'switched');
});

test('moving again refunds saved credit at its original rate; it cannot be counted twice', () => {
  const first = assignment(0, 0, 20, 100000n, 3000000n, 3000000n, 'switched');
  const second = assignment(1, 20, 40, 150000n, 3000000n, 6500000n, 'switched');
  const third = assignment(2, 40, 50, 200000n, 0n, 5000000n, 'active');
  const snapshot = structuredClone(first.states[0]);
  snapshot.details!.modifiers!.totalBonusRaw = '200000';
  assert.equal(savedResearchCredit(snapshot), 3000000n);
  const result = researchJourney([first, second, third], third, third.entries[0]);
  assert.deepEqual(result.credits.map(c => c.work), [3000000n, -3000000n, 4500000n, -4500000n]);
  assert.equal(result.comparison?.rateGain, 7000000n);
  assert.equal(result.comparison?.creditGain, -3000000n);
  assert.equal(result.comparison?.observed, 4000000n);
  assert.equal(result.comparison?.difference, 0n);
});

test('pauses earn no research and native observed applied days may be less than 30', () => {
  const first = assignment(0, 0, 10, 100000n, 0n, 0n, 'switched');
  const second = assignment(1, 20, 25, 100000n, 500000n, 1500000n, 'active');
  const result = researchJourney([first, second], second, second.entries[0]);
  assert.equal(result.periods.find(p => p.paused)?.endHour, start + 20 * 24);
  assert.equal(result.comparison?.creditGain, 500000n);
  assert.equal(result.comparison?.calculated, 2000000n);
  assert.equal(result.comparison?.difference, 0n);
});

test('rewind shows the new assignment credit immediately and hides future refunds', () => {
  const first = assignment(0, 0, 20, 100000n, 0n, 0n, 'switched');
  const second = assignment(1, 20, 40, 150000n, 3000000n, 6500000n, 'switched');
  const third = assignment(2, 40, 50, 200000n, 0n, 5000000n, 'active');
  const slots = researchAt({ coverage: [{ firstHour: start, lastHour: start + 50 * 24 }], slots: [first, second, third] }, start + 20 * 24)!;
  const result = researchJourney(slots, slots[1], slots[1].entries[0]);
  assert.equal(result.credits.length, 1);
  assert.equal(result.credits[0].kind, 'applied');
  assert.equal(result.credits[0].work, 4500000n);
  assert.equal(result.parts.length, 2);
  assert.equal(result.comparison?.difference, 0n);
});

test('a new continuity run, technology level, or native handle prevents a false transfer link', () => {
  const first = assignment(0, 0, 10, 100000n, 0n, 0n, 'switched');
  for (const change of [{ continuityStartHour: start + 10 * 24 }, { level: 1 }, { technologyHandle: 8 }, { databaseIndex: 8 }, { databaseIndex: null }]) {
    const second = assignment(1, 10, 20, 100000n, 0n, 1000000n, 'active');
    Object.assign(second.entries[0], change);
    assert.equal(researchJourney([first, second], second, second.entries[0]).parts.length, 1);
  }
});

test('completion keeps its credit even if the linked slot resets its fields at that hour', () => {
  const slot = assignment(0, 0, 150, 100000n, 3000000n, 3000000n, 'completed');
  slot.states.push({ ...slot.states[0], firstHour: start + 150 * 24, lastHour: start + 150 * 24, appliedDaysRaw: '0' });
  const result = researchJourney([slot], slot, slot.entries[0]);
  assert.deepEqual(result.credits.map(c => [c.kind, c.work]), [['applied', 3000000n]]);
  assert.equal(result.comparison?.creditGain, 0n);
  assert.equal(result.comparison?.difference, 0n);
});

test('a witnessed assignment of parked research does not claim a whole-technology duration saving', () => {
  const slot = assignment(0, 60, 150, 100000n, 3000000n, 9000000n, 'completed');
  const result = researchJourney([slot], slot, slot.entries[0]);
  assert.equal(result.startedAfterHour, start + 60 * 24 - 1);
  assert.equal(result.initial, 9000000n);
  assert.equal(result.startKnown, false);
  assert.equal(result.complete, false);
  assert.equal(result.comparison?.difference, 0n);
});

test('opening saved credit and at most one witnessed daily update can explain initial progress', () => {
  for (const initial of [3000000n, 3100000n]) {
    const slot = assignment(0, 0, 150, 100000n, 3000000n, initial, 'completed');
    const result = researchJourney([slot], slot, slot.entries[0]);
    assert.equal(result.startKnown, true);
    assert.equal(result.complete, true);
  }
  for (const initial of [2999999n, 3100001n]) {
    const slot = assignment(0, 0, 150, 100000n, 3000000n, initial, 'completed');
    assert.equal(researchJourney([slot], slot, slot.entries[0]).startKnown, false);
  }
});

test('daily-update tolerance requires a valid rate and adjacent witnessed observations', () => {
  const slot = assignment(0, 0, 150, 100000n, 3000000n, 3100000n, 'completed');
  slot.entries[0].startedAfterHour = start - 2;
  assert.equal(researchJourney([slot], slot, slot.entries[0]).startKnown, false);
  slot.entries[0].startedAfterHour = start - 1;
  slot.states[0].details = null;
  assert.equal(researchJourney([slot], slot, slot.entries[0]).startKnown, false);
});

test('missing or invalid saved-credit observations suppress full-duration claims', () => {
  const slot = assignment(0, 0, 150, 100000n, 3000000n, 3000000n, 'completed');
  slot.states[0].firstHour++;
  assert.equal(researchJourney([slot], slot, slot.entries[0]).complete, false);
  slot.states[0].firstHour--;
  slot.states[0].appliedDaysRaw = '-1';
  assert.equal(researchJourney([slot], slot, slot.entries[0]).complete, false);
  slot.states[0].appliedDaysRaw = '3000000';
  slot.states[0].lastHour -= 24;
  const result = researchJourney([slot], slot, slot.entries[0]);
  assert.equal(result.creditsComplete, false);
  assert.equal(result.complete, false);
});

test('completed explanations clip immutable history at outcome knowledge and disappear on rewind', () => {
  const first = assignment(0, 0, 10, 100000n, 0n, 0n, 'switched');
  const second = assignment(1, 10, 20, 100000n, 0n, 1000000n, 'completed');
  second.entries[0].outcomeObservedHour = start + 21 * 24;
  first.entries[0].outcome = 'completed';
  first.entries[0].outcomeObservedHour = start + 30 * 24;
  const history = { coverage: [{ firstHour: start, lastHour: start + 40 * 24 }], slots: [first, second] };
  const early = researchAt(history, start + 20 * 24)![1].entries[0];
  assert.equal(completedResearchJourney(history, 1, early), null);
  const visible = researchAt(history, start + 21 * 24)![1].entries[0];
  const result = completedResearchJourney(history, 1, visible)!;
  assert.equal(result.parts.length, 1);
  assert.equal(result.startKnown, false);
  const later = researchAt(history, start + 40 * 24)![1].entries[0];
  assert.deepEqual(completedResearchJourney(history, 1, later), result);
  const replacement = structuredClone(history);
  replacement.slots[1].progress[0].progressRaw = '0';
  assert.equal(completedResearchJourney(replacement, 1, later)!.startKnown, true);
});

test('time out of a slot lengthens the Took row instead of replacing research time', () => {
  const first = assignment(0, 0, 10, 100000n, 0n, 0n, 'switched');
  const second = assignment(1, 20, 25, 100000n, 500000n, 1500000n, 'active');
  const split = researchTimeSplit(researchJourney([first, second], second, second.entries[0]), true)!;
  assert.equal(split.paused, 1000000n);
  assert.equal(split.taken, 1500000n);
  const left = { left: Number(split.remaining / 100_000n), rounding: 0n };
  const rows = researchTimeRows(split, true, left)!;
  assert.equal(rows.elapsed, 2500000n);
  const value = (key: string) => rows.took.find(s => s.key === key)?.value;
  assert.equal(value('core'), 1500000n, 'every researched day stays in the row');
  assert.equal(value('paused'), 1000000n);
  const saved = researchTimeRows({ ...split, juggle: 100000n }, true, left)!.took.find(segment => segment.key === 'juggle');
  assert.equal(saved?.name, 'Saved research days');
  assert.equal(saved?.effect, '−1 day');
  assert.equal(rows.took.reduce((sum, s) => sum + s.value, 0n), rows.expectedTotal + split.paused);
});

test('maluses adding more days than were researched keep every segment its own effect and both rows closing', () => {
  const total = (segments: readonly { value: bigint }[]) => segments.reduce((sum, s) => sum + s.value, 0n);
  const modifier = (key: string, days: bigint) => ({ key, kind: 'research speed', label: key, days });
  const mixed: ResearchTimeSplit = { base: 10000000n, penalty: 0n, bonus: 5000000n, juggle: 0n, earlier: 0n, remaining: 0n, paused: 0n, taken: 5000000n,
    modifiers: [modifier('Bonus', 15000000n), modifier('Malus', -10000000n)], other: 0n, floored: false };
  const floored: ResearchTimeSplit = { base: 10000000n, penalty: 0n, bonus: -90000000n, juggle: 0n, earlier: 0n, remaining: 0n, paused: 0n, taken: 100000000n,
    modifiers: [modifier('Malus', -150000000n)], other: 60000000n, floored: true };
  for (const [split, row] of [[mixed, 20000000n], [floored, 160000000n]] as const) {
    const rows = researchTimeRows(split, false, null)!;
    assert.ok([...rows.expected, ...rows.took].every(s => s.value > 0n));
    assert.equal(rows.took.find(s => s.key === 'core')?.value, split.taken, 'every researched day stays in the row');
    for (const effect of split.modifiers)
      assert.equal([...rows.expected, ...rows.took].find(s => s.key === effect.key)?.value, effect.days < 0n ? -effect.days : effect.days);
    assert.equal(total(rows.expected), row);
    assert.equal(total(rows.took), row);
    assert.equal(rows.axis, row);
  }
});

test('after a witnessed start an unexplained surplus shows no breakdown', () => {
  const slot = assignment(0, 0, 90, 100000n, 0n, 0n, 'completed');
  const journey = researchJourney([slot], slot, slot.entries[0]);
  assert.equal(journey.startKnown, true);
  assert.equal(researchTimeSplit(journey, false), null);
  const parked = assignment(0, 60, 150, 100000n, 3000000n, 9000000n, 'completed');
  const split = researchTimeSplit(researchJourney([parked], parked, parked.entries[0]), false)!;
  assert.equal(split.earlier, 6000000n);
});

test('with the game days left, the rest of an active research reads in calendar days and both rows still close', () => {
  const split: ResearchTimeSplit = { base: 13750000n, penalty: 0n, bonus: 5430000n, juggle: 0n, earlier: 0n, remaining: 5700000n, paused: 0n, taken: 2620000n,
    modifiers: [{ key: 'Bonus', kind: 'research speed', label: 'Bonus', days: 5430000n }], other: 0n, floored: false };
  const total = (segments: readonly { value: bigint }[]) => segments.reduce((sum, s) => sum + s.value, 0n);
  const rows = researchTimeRows(split, true, { left: 19, rounding: 0n })!;
  assert.equal(rows.left, 1900000n);
  assert.deepEqual(rows.took.filter(s => s.key === 'rest' || s.key === 'remaining').map(s => [s.key, s.value, s.name]),
    [['rest', 3800000n, 'Bonuses on the rest'], ['remaining', 1900000n, 'Time left']]);
  assert.equal(total(rows.took), total(rows.expected));
  const slow = researchTimeRows(split, true, { left: 70, rounding: 0n })!;
  assert.equal(slow.expected.find(s => s.key === 'rest-slower')?.value, 1300000n);
  assert.equal(total(slow.took), total(slow.expected));
  assert.equal(researchDaysLeftMissing(split, true, null), true);
  assert.equal(researchTimeRows(split, true, null), null, 'without the game estimate the active breakdown is withheld, not drawn from base days');
  const paused = researchTimeRows(split, false, null)!;
  assert.equal(researchDaysLeftMissing(split, false, null), false);
  assert.equal(paused.took.find(s => s.key === 'remaining')?.name, 'Base research time still to do');
  const half = { ...split, remaining: 5650000n, taken: 2670000n };
  const exact = researchTimeRows(half, true, { left: 57, rounding: 50000n })!;
  assert.equal(exact.expected.find(s => s.key === 'rest-slower'), undefined, 'the game rounding its estimate up is no slowdown');
  assert.deepEqual(exact.expected.filter(s => s.key === 'rounding').map(s => [s.value, s.name]), [[50000n, 'Rounded up to whole days']]);
  assert.equal(total(exact.took), total(exact.expected));
  const slower = researchTimeRows(half, true, { left: 58, rounding: 50000n })!;
  assert.deepEqual(slower.expected.filter(s => s.key === 'rest-slower' || s.key === 'rounding').map(s => [s.key, s.value]), [['rest-slower', 100000n], ['rounding', 50000n]]);
  assert.equal(total(slower.took), total(slower.expected));
});

test('the rounding is the game rounding its own estimate at the recorded rate, never the base days', () => {
  const at = (remaining: bigint, left: number, estimateRaw: string) => {
    const split: ResearchTimeSplit = { base: 2000000n, penalty: 0n, bonus: 0n, juggle: 0n, earlier: 0n, remaining, paused: 0n, taken: 2000000n - remaining,
      modifiers: [], other: 0n, floored: false };
    const rows = researchTimeRows(split, true, { left, rounding: BigInt(left) * 100_000n - BigInt(estimateRaw) })!;
    const value = (segments: readonly { key: string; value: bigint }[], key: string) => segments.find(s => s.key === key)?.value ?? 0n;
    assert.equal(rows.took.reduce((sum, s) => sum + s.value, 0n), rows.expected.reduce((sum, s) => sum + s.value, 0n));
    return [value(rows.expected, 'rounding'), value(rows.took, 'rest'), value(rows.took, 'remaining')];
  };
  assert.deepEqual(at(1050000n, 7, '700000'), [0n, 350000n, 700000n], '10.5 base days at 1.5x is exactly 7 days: nothing is rounded, 3.5 days are saved');
  assert.deepEqual(at(1000000n, 4, '333333'), [66667n, 666667n, 400000n], '10 base days at 3x is 3 1/3 days, rounded up to 4');
});
