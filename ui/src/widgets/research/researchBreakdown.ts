import type { ResearchEntry, ResearchSlotHistory, ResearchSlotState } from '../../generated/dashboard';
import { researchRateAt } from './researchData.ts';

export interface ResearchPeriod {
  firstHour: number;
  endHour: number;
  state: ResearchSlotState | null;
  /** Fixed5 research days multiplied by 24; divide only after summing. */
  work: bigint | null;
  firstRate: bigint | null;
  lastRate: bigint | null;
}

const effectKey = (state: ResearchSlotState) => JSON.stringify([
  state.details?.modifiers, state.details?.bonuses, state.appliedDaysRaw, state.aheadReductionYearsRaw,
]);

/** Integrate the recorded rate over [from, to). Re-evaluate at each game-day
 * boundary: an ahead penalty declines even when no modifier changes. This is
 * a continuous-rate estimate, not a claim about the native daily update phase. */
export function researchWork(state: ResearchSlotState, from: number, to: number): bigint | null {
  let work = 0n;
  let steps = 0;
  for (let hour = from; hour < to;) {
    if (++steps > 100_000) return null;
    const nextDay = hour + 24 - ((hour - 43_800_000) % 24 + 24) % 24;
    const rate = researchRateAt(state, hour);
    if (!rate) return null;
    const end = rate.penaltyRaw === '0' ? to : Math.min(to, nextDay);
    work += BigInt(rate.rateRaw) * BigInt(end - hour);
    hour = end;
  }
  return work;
}

/** Input must already be clipped by researchAt. No future samples or assumed
 * rates across missing hours enter the explanation. */
export function researchBreakdown(slot: ResearchSlotHistory, entry: ResearchEntry) {
  const firstHour = entry.firstObservedHour;
  const endHour = entry.outcome !== 'unknown' && entry.endedByHour !== null
    ? entry.endedByHour : entry.lastObservedHour;
  const periods: ResearchPeriod[] = [];
  let cursor = firstHour;
  let lastKey: string | null = null;
  for (const state of slot.states) {
    if (state.entryFirstObservedHour !== firstHour || state.token !== entry.token) continue;
    const from = Math.max(firstHour, state.firstHour);
    const to = Math.min(endHour, state.lastHour + 1);
    if (to <= from) continue;
    if (from > cursor) {
      periods.push({ firstHour: cursor, endHour: from, state: null, work: null, firstRate: null, lastRate: null });
      lastKey = null;
    }
    const key = effectKey(state);
    const work = researchWork(state, from, to);
    const firstRate = researchRateAt(state, from);
    const lastRate = researchRateAt(state, to - 1);
    const previous = periods.at(-1);
    if (previous && previous.endHour === from && key === lastKey) {
      previous.endHour = to;
      previous.work = previous.work !== null && work !== null ? previous.work + work : null;
      previous.lastRate = lastRate ? BigInt(lastRate.rateRaw) : null;
    } else {
      periods.push({ firstHour: from, endHour: to, state, work,
        firstRate: firstRate ? BigInt(firstRate.rateRaw) : null,
        lastRate: lastRate ? BigInt(lastRate.rateRaw) : null });
    }
    cursor = to;
    lastKey = key;
  }
  if (cursor < endHour) periods.push({ firstHour: cursor, endHour, state: null, work: null, firstRate: null, lastRate: null });

  const samples = (slot.progress ?? []).filter(p => p.entryFirstObservedHour === firstHour
    && p.firstHour <= endHour && p.lastHour >= firstHour);
  const first = samples[0], last = samples.at(-1);
  const base = first?.baseDaysRaw != null ? BigInt(first.baseDaysRaw) : null;
  const observed = last && BigInt(last.progressRaw) >= 0n ? BigInt(last.progressRaw) : null;
  const initial = first && BigInt(first.progressRaw) >= 0n ? BigInt(first.progressRaw) : null;
  const costChanged = samples.some(p => p.baseDaysRaw !== first?.baseDaysRaw);
  // A missing observation or a progress reset prevents a complete reconciliation.
  const continuousProgress = samples.every((p, i) => i === 0 ||
    (samples[i - 1].lastHour + 1 === p.firstHour && BigInt(p.progressRaw) >= BigInt(samples[i - 1].progressRaw)));
  const compareEnd = last ? Math.min(last.lastHour, endHour) : firstHour;
  let calculated = 0n;
  let covered = 0;
  let complete = true;
  for (const period of periods) {
    const from = Math.max(period.firstHour, first?.firstHour ?? firstHour);
    const to = Math.min(period.endHour, compareEnd);
    if (to <= from) continue;
    const work = period.state ? researchWork(period.state, from, to) : null;
    if (work === null) complete = false;
    else { calculated += work; covered += to - from; }
  }
  const reconciled = first && last && initial !== null && observed !== null && continuousProgress && complete
    && covered === compareEnd - first.firstHour;
  return {
    periods, base, observed, initial, costChanged, endHour,
    latestBase: last?.baseDaysRaw != null ? BigInt(last.baseDaysRaw) : null,
    comparison: reconciled && observed !== null && initial !== null ? { observed: observed - initial, calculated: calculated / 24n,
      difference: observed - initial - calculated / 24n } : null,
    complete: entry.startedAfterHour !== null && entry.outcome === 'completed' && !costChanged
      && first?.firstHour === firstHour,
  };
}

/** Only presentation rounds; arithmetic above remains integer fixed point. */
export function researchNumber(raw: bigint, precision = 1): string {
  const scale = 10n ** BigInt(precision);
  const magnitude = raw < 0n ? -raw : raw;
  const rounded = (magnitude * scale + 50_000n) / 100_000n;
  const whole = rounded / scale;
  const fraction = (rounded % scale).toString().padStart(precision, '0').replace(/0+$/, '');
  return `${raw < 0n && rounded !== 0n ? '−' : ''}${whole}${fraction ? `.${fraction}` : ''}`;
}
