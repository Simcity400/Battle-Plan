import assert from 'node:assert/strict';
import test from 'node:test';
import type { ResearchSlotHistory } from './types';
import { researchAt, researchRateAt } from './researchData.ts';
import { researchBreakdown, researchNumber, researchSplit, researchWork } from './researchBreakdown.ts';
import { researchJourney, researchTimeSplit } from './researchJourney.ts';
import type { ResearchEntry, ResearchSlotState } from '../../generated/dashboard';

const start = 43_800_000 + 1938 * 8760;
const assignment: ResearchEntry = {
  token: 'radio', technologyHandle: 1, databaseIndex: 1, continuityStartHour: start, level: 0, firstObservedHour: start, startedAfterHour: start - 1,
  lastObservedHour: start + 150 * 24 - 1, endedByHour: start + 150 * 24, outcomeObservedHour: null, outcome: 'completed',
};
function state(from: number, to: number, bonus: string): ResearchSlotState {
  return {
    firstHour: start + from * 24, lastHour: start + to * 24 - 1, token: 'radio', databaseIndex: 1, entryFirstObservedHour: start,
    speedMultiplierRaw: '100000', bankedDaysRaw: '0', appliedDaysRaw: '0', aheadReductionYearsRaw: '0',
    details: { remainingDays: null, estimateRaw: null, bonuses: [], modifiers: {
      components: ['country', 'category', 'designTeam', 'experience', 'license', 'scientist', 'applied', 'sharing']
        .map(kind => ({ kind, raw: kind === 'country' ? bonus : '0', sources: [] })),
      totalBonusRaw: bonus, year: 1938, aheadFactorRaw: '200000', minimumSpeedRaw: '10000',
    } },
  };
}
function fixture(): ResearchSlotHistory {
  return {
    index: 0, availability: [{ firstHour: start, lastHour: assignment.endedByHour! }], entries: [assignment],
    states: [state(0, 60, '0'), state(60, 120, '50000'), state(120, 150, '0')],
    progress: Array.from({ length: 151 }, (_, day) => ({
      firstHour: start + day * 24, lastHour: Math.min(start + (day + 1) * 24 - 1, assignment.endedByHour!),
      entryFirstObservedHour: start, baseDaysRaw: '18000000',
      progressRaw: (BigInt(day <= 60 ? day * 2 : day <= 120 ? 120 + (day - 60) * 3 : 300 + (day - 120) * 2) * 50000n).toString(),
    })),
  };
}

function recordedAt(slot: ResearchSlotHistory, only: ResearchSlotState, days: number, base: bigint): ResearchSlotHistory {
  let done = 0n;
  const progress = [];
  for (let day = 0; day <= days; day++) {
    progress.push({ firstHour: start + day * 24, lastHour: start + day * 24 + 23, entryFirstObservedHour: start, baseDaysRaw: base.toString(), progressRaw: done.toString() });
    done += researchWork(only, start + day * 24, start + (day + 1) * 24)! / 24n;
  }
  return { ...slot, states: [only], progress };
}

test('a temporary bonus earns progress only during its active period', () => {
  const result = researchBreakdown(fixture(), assignment);
  assert.equal(result.base, 18000000n);
  assert.deepEqual(result.periods.map(p => p.work! / 24n), [6000000n, 9000000n, 3000000n]);
  assert.deepEqual(result.comparison, { observed: 18000000n, calculated: 18000000n, difference: 0n });
  assert.equal(result.complete, true);
});

test('rewind clips progress and modifier periods; it never uses a future completion', () => {
  const slot = fixture();
  const clipped = researchAt({ coverage: slot.availability, slots: [slot] }, start + 90 * 24)![0];
  const result = researchBreakdown(clipped, clipped.entries[0]);
  assert.equal(clipped.entries[0].outcome, 'active');
  assert.equal(result.observed, 10500000n);
  assert.deepEqual(result.periods.map(p => p.work! / 24n), [6000000n, 4500000n]);
  assert.equal(result.complete, false);
  assert.equal(result.comparison?.difference, 0n);
});

test('ahead penalties decline across days without creating modifier periods', () => {
  const active = state(0, 365, '0');
  active.details!.modifiers!.year = 1939;
  const first = BigInt(researchRateAt(active, start)!.rateRaw);
  const second = BigInt(researchRateAt(active, start + 24)!.rateRaw);
  assert.ok(second > first);
  assert.equal(researchWork(active, start + 12, start + 36), first * 12n + second * 12n);
});

test('countdown-only states merge visually; missing modifier hours cannot be bridged', () => {
  const slot = fixture();
  slot.states.splice(0, 1, state(0, 30, '0'), state(30, 60, '0'));
  slot.states[1].details = { ...slot.states[1].details!, remainingDays: 150, estimateRaw: '15000000' };
  assert.equal(researchBreakdown(slot, assignment).periods.length, 3);
  slot.states.splice(1, 1);
  const result = researchBreakdown(slot, assignment);
  assert.equal(result.periods[1].state, null);
  assert.equal(result.comparison, null);
});

test('initial progress is not counted again; missing progress and resets prevent reconciliation', () => {
  const slot = fixture();
  for (const sample of slot.progress) sample.progressRaw = (BigInt(sample.progressRaw) + 3000000n).toString();
  const result = researchBreakdown(slot, assignment);
  assert.equal(result.initial, 3000000n);
  assert.equal(result.comparison?.difference, 0n);
  slot.progress.splice(10, 1);
  assert.equal(researchBreakdown(slot, assignment).comparison, null);
  const reset = fixture(); reset.progress[60].progressRaw = '0';
  assert.equal(researchBreakdown(reset, assignment).comparison, null);
});

test('unknown starts and changing costs do not claim a full duration saving', () => {
  const slot = fixture();
  assert.equal(researchBreakdown(slot, { ...assignment, startedAfterHour: null }).complete, false);
  slot.progress[60].baseDaysRaw = '20000000';
  const result = researchBreakdown(slot, assignment);
  assert.equal(result.costChanged, true);
  assert.equal(result.complete, false);
  assert.equal(researchNumber(-123456n), '−1.2');
  assert.equal(researchNumber(9223372036854775807n, 2), '92233720368547.76');
});

test('time split: base, penalty and bonuses add up to the time actually taken', () => {
  const slot = fixture();
  const split = researchTimeSplit(researchJourney([slot], slot, assignment), false)!;
  assert.deepEqual({ ...split, modifiers: undefined }, { base: 18000000n, penalty: 0n, bonus: 3000000n, juggle: 0n, earlier: 0n, remaining: 0n,
    paused: 0n, taken: 15000000n, modifiers: undefined, other: 0n, floored: false });
  assert.deepEqual(split.modifiers.map(m => [m.label, m.days]), [['Research speed', 3000000n]]);
});

test('time split names every modifier source; a component keeps what its sources do not explain', () => {
  const slot = fixture();
  const boosted = state(0, 150, '30000');
  const country = boosted.details!.modifiers!.components[0];
  country.raw = '30000';
  country.sources = [{ token: 'german_engineering', name: 'German Engineering', raw: '20000' }];
  boosted.details!.modifiers!.components[1].raw = '-10000';
  boosted.details!.modifiers!.totalBonusRaw = '20000';
  slot.states = [boosted];
  const split = researchTimeSplit(researchJourney([slot], slot, assignment), false)!;
  assert.deepEqual(split.modifiers.map(m => [m.label, m.days]), [
    ['German Engineering', 3000000n], ['Other research speed', 1500000n], ['Category bonus', -1500000n],
  ]);
  assert.equal(split.other, 0n);
  assert.equal(split.modifiers.reduce((sum, m) => sum + m.days, 0n) + split.other, split.bonus);
});

test('the minimum research speed is its own part of the time, not blamed on a modifier', () => {
  const slowed = state(0, 150, '-950000');
  const slot = recordedAt(fixture(), slowed, 150, 1500000n);
  const split = researchTimeSplit(researchJourney([slot], slot, assignment), false)!;
  assert.equal(split.floored, true);
  assert.ok(split.other > 0n);
  assert.equal(split.modifiers.reduce((sum, m) => sum + m.days, 0n) + split.other, split.bonus);
});

test('time split: the ahead-of-time penalty stretches the base, and active research keeps the rest as remaining', () => {
  const ahead = state(0, 150, '0');
  ahead.details!.modifiers!.year = 1939;
  const slot = recordedAt(fixture(), ahead, 150, 18000000n);
  const split = researchSplit(ahead, start, start + 24)!;
  assert.equal(split.work + split.penalty - split.bonus, 100_000n * 24n);
  assert.ok(split.penalty > 0n);
  const clipped = researchAt({ coverage: slot.availability, slots: [slot] }, start + 30 * 24)![0];
  const partial = researchTimeSplit(researchJourney([clipped], clipped, clipped.entries[0]), true)!;
  assert.equal(partial.base + partial.penalty - partial.bonus - partial.juggle - partial.earlier - partial.remaining, partial.taken);
  assert.equal(partial.taken, 3000000n);
  assert.ok(partial.remaining > 0n);
});

test('time split refuses to guess across a recording gap', () => {
  const slot = { ...fixture(), states: [state(0, 60, '0'), state(90, 150, '0')] };
  assert.equal(researchTimeSplit(researchJourney([slot], slot, assignment), false), null);
});
