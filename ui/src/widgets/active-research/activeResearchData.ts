import type { ResearchHistory, ResearchSlotHistory, ResearchSlotsAt } from '../research/types';
import { isUnjoinedResearch, researchAt, researchEntryAt } from '../research/researchData.ts';
import { coverageAt } from '../research/researchLayout.ts';
import type { ResearchEntry, ResearchSlotNow } from '../../generated/dashboard';

export type ActiveSlot =
  | { kind: 'researching'; slot: ResearchSlotHistory; read: ResearchSlotNow; entry: ResearchEntry | null; token: string }
  | { kind: 'idle'; slot: ResearchSlotHistory; read: ResearchSlotNow }
  | { kind: 'unavailable'; slot: ResearchSlotHistory; reason: string };

export function completedResearch(slots: readonly ResearchSlotHistory[], token: string, hour: number) {
  let found: { slot: ResearchSlotHistory; entry: ResearchEntry; hour: number } | null = null;
  for (const slot of slots) for (const entry of slot.entries) {
    if (entry.token !== token || entry.outcome !== 'completed' || entry.endedByHour === null) continue;
    if (Math.max(entry.endedByHour, entry.outcomeObservedHour ?? entry.endedByHour) > hour) continue;
    if (!found || entry.endedByHour > found.hour) found = { slot, entry, hour: entry.endedByHour };
  }
  return found;
}

export function activeSlots(history: ResearchHistory, now: ResearchSlotsAt): ActiveSlot[] {
  const hour = now.hour;
  const coverage = coverageAt(history.coverage, hour);
  const covered = coverage.some(span => span.firstHour <= hour && hour <= span.lastHour);
  const lastRead = [...coverage].reverse().find(span => span.lastHour < hour)?.lastHour;
  const reads = new Map(now.slots.map(read => [read.index, read]));
  return researchAt(history, hour).sort((a, b) => a.index - b.index).flatMap((slot): ActiveSlot[] => {
    if (!covered) return lastRead !== undefined && slot.availability.some(span => span.lastHour === lastRead)
      ? [{ kind: 'unavailable', slot, reason: 'Not recorded at this hour' }] : [];
    if (!slot.availability.some(span => span.firstHour <= hour && hour <= span.lastHour)) return [];
    const read = reads.get(slot.index);
    if (!read) return [{ kind: 'unavailable', slot, reason: now.missing ?? 'Not recorded at this hour' }];
    const full = history.slots.find(owner => owner.index === slot.index) ?? slot;
    const entry = researchEntryAt(full, read.state, hour);
    if (!entry && !isUnjoinedResearch(read.state)) return [{ kind: 'idle', slot, read }];
    return [{ kind: 'researching', slot, read, entry, token: entry?.token ?? read.state.token! }];
  });
}
