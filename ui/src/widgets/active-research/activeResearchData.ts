import type { ResearchHistory, ResearchSlotHistory, ResearchSlotsAt } from '../research/types';
import { isUnjoinedResearch, researchAt, researchEntryAt } from '../research/researchData.ts';
import { coverageAt, researchDate } from '../research/researchLayout.ts';
import type { ResearchEntry, ResearchSlotNow } from '../../generated/dashboard';

export type ActiveSlot =
  | { kind: 'researching'; slot: ResearchSlotHistory; read: ResearchSlotNow; entry: ResearchEntry | null; token: string }
  | { kind: 'idle'; slot: ResearchSlotHistory; read: ResearchSlotNow }
  | { kind: 'unavailable'; slot: ResearchSlotHistory; reason: string };

export interface OpenedResearch { slot: number; token: string; databaseIndex: number | null; firstObservedHour: number | null }

type Researching = Extract<ActiveSlot, { kind: 'researching' }>;

export const openedResearch = (item: Researching): OpenedResearch =>
  ({ slot: item.slot.index, token: item.token, databaseIndex: item.entry?.databaseIndex ?? null, firstObservedHour: item.entry?.firstObservedHour ?? null });

export const isOpenedResearch = (item: Researching, opened: OpenedResearch) => item.slot.index === opened.slot && item.token === opened.token
  && (item.entry?.databaseIndex ?? null) === opened.databaseIndex && (item.entry?.firstObservedHour ?? null) === opened.firstObservedHour;

export type ResearchAfter = { kind: 'completed'; slot: ResearchSlotHistory; entry: ResearchEntry; hour: number } | { kind: 'gone'; message: string };

export function researchAfter(slots: readonly ResearchSlotHistory[], opened: OpenedResearch, hour: number): ResearchAfter {
  const { firstObservedHour } = opened;
  if (firstObservedHour === null) return { kind: 'gone', message: 'This research is not in its slot at this hour, and its history was not joined, so what became of it is not recorded.' };
  if (hour < firstObservedHour) return { kind: 'gone', message: `Not in its slot yet at this hour: first seen there ${researchDate(firstObservedHour)}.` };
  const slot = slots.find(owner => owner.index === opened.slot);
  const entry = slot?.entries.find(item => item.token === opened.token && item.databaseIndex === opened.databaseIndex && item.firstObservedHour === firstObservedHour);
  if (!slot || !entry) return { kind: 'gone', message: 'This research is not recorded in its slot at this hour.' };
  if (entry.outcome === 'completed' && entry.endedByHour !== null) return { kind: 'completed', slot, entry, hour: entry.endedByHour };
  if (entry.outcome === 'switched' && entry.endedByHour !== null) return { kind: 'gone', message: `Switched out of its slot by ${researchDate(entry.endedByHour)}.` };
  if (entry.outcome === 'unknown') return { kind: 'gone', message: 'Left its slot; what became of it is not recorded at this hour.' };
  return { kind: 'gone', message: 'This research is not recorded in its slot at this hour.' };
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
