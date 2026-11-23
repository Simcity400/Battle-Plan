import type { ResearchHistory, ResearchSlotHistory, ResearchSlotsAt } from '../research/types';
import { isUnjoinedResearch, researchAt, researchEntryAt } from '../research/researchData.ts';
import { coverageAt, researchDate } from '../research/researchLayout.ts';
import type { ResearchDefinitionText, ResearchEntry, ResearchSlotNow } from '../../generated/dashboard';

export type ActiveSlot =
  | { kind: 'researching'; slot: ResearchSlotHistory; read: ResearchSlotNow; entry: ResearchEntry | null; token: string; databaseIndex: number }
  | { kind: 'idle'; slot: ResearchSlotHistory; read: ResearchSlotNow }
  | { kind: 'unavailable'; slot: ResearchSlotHistory; reason: string };

export type OpenedResearch = { slot: number; token: string; databaseIndex: number }
  & ({ firstObservedHour: number; readHour: null } | { firstObservedHour: null; readHour: number; definition: ResearchDefinitionText | null });

type Researching = Extract<ActiveSlot, { kind: 'researching' }>;

export const openedResearch = ({ slot, token, databaseIndex, entry, read }: Researching): OpenedResearch => entry
  ? { slot: slot.index, token, databaseIndex, firstObservedHour: entry.firstObservedHour, readHour: null }
  : { slot: slot.index, token, databaseIndex, firstObservedHour: null, readHour: read.state.firstHour, definition: read.definition };

export const isOpenedResearch = (item: Researching, opened: OpenedResearch) => item.slot.index === opened.slot && item.token === opened.token
  && item.databaseIndex === opened.databaseIndex
  && (item.entry ? item.entry.firstObservedHour === opened.firstObservedHour : opened.readHour !== null && item.read.state.firstHour === opened.readHour);

export type ResearchAfter = { kind: 'completed'; slot: ResearchSlotHistory; entry: ResearchEntry; hour: number } | { kind: 'gone'; message: string };

export function researchAfter(slots: readonly ResearchSlotHistory[], opened: OpenedResearch, hour: number): ResearchAfter {
  const { firstObservedHour } = opened;
  if (firstObservedHour === null) return { kind: 'gone', message: hour < opened.readHour
    ? `Not read in its slot yet at this hour: opened from the read of ${researchDate(opened.readHour)}.`
    : 'Its assignment was not recorded, so no other read of its slot can be told to be this research.' };
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
    if (read.state.token === null || read.state.databaseIndex === null) return [{ kind: 'unavailable', slot, reason: 'The research read names no technology definition' }];
    return [{ kind: 'researching', slot, read, entry, token: read.state.token, databaseIndex: read.state.databaseIndex }];
  });
}
