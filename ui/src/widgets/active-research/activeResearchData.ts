import type { ResearchCoverage, ResearchEntry, ResearchSlotHistory, ResearchSlotState } from '../../generated/dashboard';
import { isUnjoinedResearch, researchLatestProgress, researchRateAt, researchStateAt } from '../research/researchData.ts';

/** What one slot is doing at an hour. A technology held without its recorded
 * occurrence (older recordings) has no entry, so no history to open. */
export type ActiveSlot =
  | { kind: 'researching'; slot: ResearchSlotHistory; state: ResearchSlotState; entry: ResearchEntry | null; token: string;
    progress: { fraction: number; hour: number } | null; left: number | null; aheadOf: number | null }
  | { kind: 'idle'; slot: ResearchSlotHistory; savedRaw: bigint }
  | { kind: 'unavailable'; slot: ResearchSlotHistory };

/** One card per slot open at `hour`, in slot order. At an hour without a read,
 * the slots open at the last read show as unavailable; a slot that had closed
 * by then is not shown. `slots` and `coverage` are clipped at `hour`. */
export function activeSlots(slots: readonly ResearchSlotHistory[], coverage: readonly ResearchCoverage[], hour: number): ActiveSlot[] {
  const read = coverage.some(span => span.firstHour <= hour && hour <= span.lastHour);
  const lastRead = [...coverage].reverse().find(span => span.lastHour < hour)?.lastHour;
  return [...slots].sort((a, b) => a.index - b.index).flatMap((slot): ActiveSlot[] => {
    if (!read) return lastRead !== undefined && slot.availability.some(span => span.lastHour === lastRead) ? [{ kind: 'unavailable', slot }] : [];
    if (!slot.availability.some(span => span.firstHour <= hour && hour <= span.lastHour)) return [];
    const state = researchStateAt(slot, hour);
    if (!state) return [{ kind: 'unavailable', slot }];
    const entry = slot.entries.find(candidate => candidate.outcome === 'active'
      && candidate.firstObservedHour === state.entryFirstObservedHour && candidate.token === state.token) ?? null;
    if (!entry && !isUnjoinedResearch(state)) return [{ kind: 'idle', slot, savedRaw: BigInt(state.bankedDaysRaw) }];
    const rate = researchRateAt(state, hour);
    return [{ kind: 'researching', slot, state, entry, token: entry?.token ?? state.token!, progress: researchLatestProgress(slot, state, hour),
      left: state.details?.remainingDays ?? null,
      // The ahead-of-time penalty applies at this hour: the technology's year is still to come.
      aheadOf: rate && BigInt(rate.penaltyRaw) > 0n ? state.details!.modifiers!.year : null }];
  });
}
