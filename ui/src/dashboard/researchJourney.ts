import type { ResearchEntry, ResearchHistory, ResearchSlotHistory, ResearchSlotState } from '../generated/analytics';
import { researchBreakdown, researchWork, type ResearchPeriod } from './researchBreakdown.ts';
import { researchAt, researchRateAt } from './researchData.ts';

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

/** ECE4F0 applies and refunds saved days at the SLOT'S stored assignment rate,
 * not the rate recalculated at the later playback hour. */
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

/** Follow a technology through slots within one continuous country identity.
 * Completion belongs only to its final slot, while its explanation retains the
 * earlier assignments. Never join across gaps, repeats, or ambiguous overlaps. */
export function researchJourney(slots: ResearchSlotHistory[], slot: ResearchSlotHistory, entry: ResearchEntry) {
  const candidates = slots.flatMap(owner => owner.entries.filter(e =>
    e.token === entry.token && e.technologyHandle === entry.technologyHandle && e.level === entry.level
    && e.continuityStartHour === entry.continuityStartHour && e.firstObservedHour <= entry.firstObservedHour,
  ).map(e => ({ slot: owner, entry: e }))).sort((a, b) => a.entry.firstObservedHour - b.entry.firstObservedHour || a.slot.index - b.slot.index);
  const target = candidates.findIndex(p => p.slot.index === slot.index && p.entry.firstObservedHour === entry.firstObservedHour);
  let firstIndex = target;
  while (firstIndex > 0) {
    const before = candidates[firstIndex - 1].entry, after = candidates[firstIndex].entry;
    if (before.outcome !== 'switched' || before.endedByHour === null || before.endedByHour > after.firstObservedHour) break;
    firstIndex--;
  }
  const assignments = target >= 0 ? candidates.slice(firstIndex, target + 1) : [{ slot, entry }];
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
  // Refunds precede new credits when two recorded assignments meet at an hour.
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
  // The first progress sample already contains all credits at that hour.
  // Include only later events in its observed-gain reconciliation.
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
  const openingState = first.slot.states.find(state => state.firstHour === first.entry.firstObservedHour
    && state.entryFirstObservedHour === first.entry.firstObservedHour && state.token === first.entry.token);
  const openingCredit = openingState ? savedResearchCredit(openingState) : null;
  const openingRate = openingState ? researchRateAt(openingState, first.entry.firstObservedHour) : null;
  // Seeing a slot assignment does not prove the technology was untouched before
  // recording. Its opening progress must be explained by the witnessed credit
  // and at most one native daily update between adjacent hourly observations.
  const openingDay = openingRate && first.entry.startedAfterHour === first.entry.firstObservedHour - 1
    ? BigInt(openingRate.rateRaw) : 0n;
  const startKnown = first.entry.startedAfterHour !== null && startSample?.firstHour === first.entry.firstObservedHour
    && openingCredit !== null && initial !== null && initial >= openingCredit
    && initial <= openingCredit + (openingDay > 0n ? openingDay : 0n);
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

/** Called only once per mounted completed explanation. Clip immutable source
 * history at the completion's observation bound before joining earlier work;
 * later assignments and later outcome discoveries must not enter its journey. */
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
