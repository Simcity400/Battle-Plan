import type { ResearchHistory, ResearchSlotHistory } from './types';
import { researchBreakdown, researchModifierParts, researchSplit, researchWork, type ResearchPeriod } from './researchBreakdown.ts';
import { researchAt, researchRateAt } from './researchData.ts';
import type { ResearchEntry, ResearchSlotState } from '../../generated/dashboard';

export interface ResearchCredit {
  hour: number;
  slotIndex: number;
  kind: 'applied' | 'returned' | 'adjusted' | 'existing';
  days: bigint;
  work: bigint;
}
export interface ResearchJourneyPeriod extends ResearchPeriod {
  slotIndex: number | null;
  paused?: boolean;
}

export function savedResearchCredit(state: ResearchSlotState): bigint | null {
  const days = BigInt(state.appliedDaysRaw), speed = BigInt(state.speedMultiplierRaw);
  const product = days * speed;
  return days >= 0n && speed >= 0n && product <= 9223372036854775807n ? product / 100_000n : null;
}

function assignmentCredits(slot: ResearchSlotHistory, entry: ResearchEntry, endHour: number) {
  const credits: ResearchCredit[] = [];
  const states = slot.states.filter(s => s.entryFirstObservedHour === entry.firstObservedHour
    && s.token === entry.token && s.firstHour <= endHour && s.lastHour >= entry.firstObservedHour
    && (entry.endedByHour === null || s.firstHour < entry.endedByHour));
  let previous: ResearchSlotState | undefined;
  let previousCredit: bigint | null = null;
  let complete = true;
  for (const state of states) {
    const credit = savedResearchCredit(state);
    if (credit === null) complete = false;
    if (!previous) {
      if (state.firstHour !== entry.firstObservedHour) complete = false;
      else if (credit !== null && credit !== 0n) credits.push({
        hour: state.firstHour, slotIndex: slot.index, kind: entry.startedAfterHour === null ? 'existing' : 'applied', days: BigInt(state.appliedDaysRaw), work: credit,
      });
    } else if (previous.lastHour + 1 !== state.firstHour) complete = false;
    else if (credit !== null && previousCredit !== null && credit !== previousCredit) credits.push({
      hour: state.firstHour, slotIndex: slot.index, kind: 'adjusted',
      days: BigInt(state.appliedDaysRaw) - BigInt(previous.appliedDaysRaw), work: credit - previousCredit,
    });
    previous = state;
    previousCredit = credit;
  }
  if (!previous || previous.lastHour < Math.min(entry.lastObservedHour, endHour)) complete = false;
  if (entry.outcome === 'switched' && entry.endedByHour !== null && previous && previousCredit !== null) {
    if (previous.lastHour < entry.endedByHour - 1) complete = false;
    else if (previousCredit !== 0n) credits.push({
      hour: entry.endedByHour, slotIndex: slot.index, kind: 'returned',
      days: -BigInt(previous.appliedDaysRaw), work: -previousCredit,
    });
  } else if (entry.outcome === 'unknown') complete = false;
  return { credits, complete };
}

export function researchAssignments(slots: readonly ResearchSlotHistory[], slot: ResearchSlotHistory, entry: ResearchEntry) {
  if (entry.databaseIndex == null) return [{ slot, entry }];
  const candidates = slots.flatMap(owner => owner.entries.filter(e =>
    e.databaseIndex === entry.databaseIndex && e.token === entry.token && e.technologyHandle === entry.technologyHandle && e.level === entry.level
    && e.continuityStartHour === entry.continuityStartHour && e.firstObservedHour <= entry.firstObservedHour,
  ).map(e => ({ slot: owner, entry: e }))).sort((a, b) => a.entry.firstObservedHour - b.entry.firstObservedHour || a.slot.index - b.slot.index);
  const target = candidates.findIndex(p => p.slot.index === slot.index && p.entry.firstObservedHour === entry.firstObservedHour);
  let firstIndex = target;
  while (firstIndex > 0) {
    const before = candidates[firstIndex - 1].entry, after = candidates[firstIndex].entry;
    if (before.outcome !== 'switched' || before.endedByHour === null || before.endedByHour > after.firstObservedHour) break;
    firstIndex--;
  }
  return target >= 0 ? candidates.slice(firstIndex, target + 1) : [{ slot, entry }];
}

export function researchStartKnown(slot: ResearchSlotHistory, entry: ResearchEntry): boolean {
  if (entry.startedAfterHour === null) return false;
  const opening = slot.progress.find(p => p.entryFirstObservedHour === entry.firstObservedHour && p.firstHour === entry.firstObservedHour);
  const state = slot.states.find(s => s.firstHour === entry.firstObservedHour
    && s.entryFirstObservedHour === entry.firstObservedHour && s.token === entry.token);
  const credit = state ? savedResearchCredit(state) : null;
  if (!opening || !state || credit === null) return false;
  const rate = entry.startedAfterHour === entry.firstObservedHour - 1 ? researchRateAt(state, entry.firstObservedHour) : null;
  const day = rate ? BigInt(rate.rateRaw) : 0n;
  const initial = BigInt(opening.progressRaw);
  return initial >= credit && initial <= credit + (day > 0n ? day : 0n);
}

export function researchJourney(slots: ResearchSlotHistory[], slot: ResearchSlotHistory, entry: ResearchEntry) {
  const assignments = researchAssignments(slots, slot, entry);
  const parts = assignments.map(p => ({ ...p, data: researchBreakdown(p.slot, p.entry) }));
  const first = parts[0], last = parts[parts.length - 1];
  const periods: ResearchJourneyPeriod[] = [];
  const credits: ResearchCredit[] = [];
  let creditsComplete = true;
  let previousEnd: number | undefined;
  for (const part of parts) {
    if (previousEnd !== undefined && previousEnd < part.entry.firstObservedHour) periods.push({
      firstHour: previousEnd, endHour: part.entry.firstObservedHour, slotIndex: null, paused: true,
      state: null, work: 0n, firstRate: null, lastRate: null,
    });
    periods.push(...part.data.periods.map(p => ({ ...p, slotIndex: part.slot.index })));
    const attribution = assignmentCredits(part.slot, part.entry, part.data.endHour);
    credits.push(...attribution.credits);
    creditsComplete &&= attribution.complete;
    previousEnd = part.data.endHour;
  }
  credits.sort((a, b) => a.hour - b.hour || (a.kind === 'returned' ? -1 : 0) - (b.kind === 'returned' ? -1 : 0));
  const samples = parts.flatMap(part => part.slot.progress.filter(p => p.entryFirstObservedHour === part.entry.firstObservedHour
    && p.firstHour <= part.data.endHour && p.lastHour >= part.entry.firstObservedHour));
  const startSample = samples[0], endSample = samples.at(-1);
  const compareStart = startSample?.firstHour ?? first.entry.firstObservedHour;
  const compareEnd = endSample ? Math.min(endSample.lastHour, last.data.endHour) : compareStart;
  let work = 0n, covered = 0, ratesComplete = true;
  for (const period of periods) {
    const from = Math.max(compareStart, period.firstHour), to = Math.min(compareEnd, period.endHour);
    if (to <= from) continue;
    const amount = period.paused ? 0n : period.state ? researchWork(period.state, from, to) : null;
    if (amount === null) ratesComplete = false;
    else { work += amount; covered += to - from; }
  }
  const creditGain = credits.filter(c => c.hour > compareStart && c.hour <= compareEnd).reduce((sum, c) => sum + c.work, 0n);
  const sampleCoverage = parts.every(part => {
    const samples = part.slot.progress.filter(p => p.entryFirstObservedHour === part.entry.firstObservedHour);
    return samples.length > 0 && samples.every((p, i) => BigInt(p.progressRaw) >= 0n
      && (i === 0 || samples[i - 1].lastHour + 1 === p.firstHour));
  });
  const initial = startSample ? BigInt(startSample.progressRaw) : null;
  const observed = endSample ? BigInt(endSample.progressRaw) : null;
  const calculated = work / 24n + creditGain;
  const costChanged = samples.some(p => p.baseDaysRaw !== startSample?.baseDaysRaw);
  const startKnown = researchStartKnown(first.slot, first.entry);
  return {
    periods, credits, creditsComplete, parts, firstHour: first.entry.firstObservedHour,
    startedAfterHour: first.entry.startedAfterHour, startKnown,
    base: first.data.base, latestBase: last.data.latestBase, initial, observed, costChanged,
    endHour: last.data.endHour,
    comparison: ratesComplete && creditsComplete && sampleCoverage && covered === compareEnd - compareStart
      && initial !== null && initial >= 0n && observed !== null && observed >= 0n
      ? { observed: observed - initial, calculated, difference: observed - initial - calculated,
        rateGain: work / 24n, creditGain } : null,
    complete: startKnown && creditsComplete && entry.outcome === 'completed' && !costChanged,
  };
}

export interface ResearchTimeSplit {
  base: bigint; penalty: bigint; bonus: bigint; juggle: bigint; earlier: bigint; remaining: bigint; paused: bigint; taken: bigint;
  modifiers: ResearchModifierEffect[];
  other: bigint; floored: boolean;
}
export interface ResearchModifierEffect { key: string; kind: string; label: string; days: bigint }

export function researchTimeSplit(journey: ReturnType<typeof researchJourney>, active: boolean): ResearchTimeSplit | null {
  const base = active ? journey.latestBase : journey.base;
  if (base === null || base <= 0n || journey.costChanged || !journey.creditsComplete) return null;
  let work = 0n, penalty = 0n, hours = 0, pausedHours = 0, floored = false;
  const effects = new Map<string, ResearchModifierEffect>();
  for (const period of journey.periods) {
    if (period.paused) { pausedHours += period.endHour - period.firstHour; continue; }
    if (!period.state) return null;
    const split = researchSplit(period.state, period.firstHour, period.endHour);
    const parts = researchModifierParts(period.state);
    if (!split || !parts) return null;
    work += split.work; penalty += split.penalty;
    const span = period.endHour - period.firstHour;
    hours += span;
    floored ||= split.floored;
    for (const part of parts) {
      const effect = effects.get(part.key) ?? { key: part.key, kind: part.kind, label: part.label, days: 0n };
      effect.days += part.raw * BigInt(span);
      effects.set(part.key, effect);
    }
  }
  const done = active ? journey.observed : base;
  if (done === null) return null;
  const juggle = journey.credits.reduce((sum, credit) => sum + credit.work, 0n);
  let earlier = done - work / 24n - juggle;
  if (earlier > -100_000n && earlier < 100_000n) earlier = 0n;
  if (earlier < 0n || (earlier > 0n && journey.startKnown)) return null;
  const remaining = active && base > done ? base - done : 0n;
  const taken = BigInt(hours) * 100_000n / 24n;
  const bonus = base + penalty / 24n - juggle - earlier - remaining - taken;
  const modifiers = [...effects.values()].map(effect => ({ ...effect, days: effect.days / 24n })).filter(effect => effect.days !== 0n)
    .sort((a, b) => { const size = (days: bigint) => days < 0n ? -days : days;
      return size(b.days) > size(a.days) ? 1 : size(b.days) < size(a.days) ? -1 : b.days > a.days ? 1 : b.days < a.days ? -1 : a.label.localeCompare(b.label); });
  let other = bonus - modifiers.reduce((sum, effect) => sum + effect.days, 0n);
  if (!floored && other > -100_000n && other < 100_000n && modifiers.length) { modifiers[0].days += other; other = 0n; }
  return { base, penalty: penalty / 24n, bonus, juggle, earlier, remaining, paused: BigInt(pausedHours) * 100_000n / 24n, taken, modifiers, other, floored };
}

export function completedResearchJourney(history: ResearchHistory, slotIndex: number, entry: ResearchEntry) {
  if (entry.outcome !== 'completed' || entry.endedByHour === null) return null;
  const hour = Math.max(entry.endedByHour, entry.outcomeObservedHour ?? entry.endedByHour);
  const slots = researchAt(history, hour);
  const slot = slots?.find(owner => owner.index === slotIndex);
  const completed = slot?.entries.find(candidate => candidate.firstObservedHour === entry.firstObservedHour
    && candidate.technologyHandle === entry.technologyHandle && candidate.level === entry.level
    && candidate.token === entry.token && candidate.outcome === 'completed');
  return slots && slot && completed ? researchJourney(slots, slot, completed) : null;
}
