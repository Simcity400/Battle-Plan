import type { ResearchHistory, ResearchSlotHistory, ResearchSlotsAt } from '../research/types';
import { isUnjoinedResearch, researchEntryAt } from '../research/researchData.ts';
import { coverageAt } from '../research/researchLayout.ts';
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

const slotIndex = new WeakMap<ResearchHistory, readonly ResearchSlotHistory[]>();

function slotsByIndex(history: ResearchHistory): readonly ResearchSlotHistory[] {
  let held = slotIndex.get(history);
  if (!held) {
    held = [...history.slots].sort((a, b) => a.index - b.index);
    slotIndex.set(history, held);
  }
  return held;
}

export function activeSlots(history: ResearchHistory, now: ResearchSlotsAt): ActiveSlot[] {
  const hour = now.hour;
  const coverage = coverageAt(history.coverage, hour);
  const covered = coverage.some(span => span.firstHour <= hour && hour <= span.lastHour);
  const lastRead = [...coverage].reverse().find(span => span.lastHour < hour)?.lastHour;
  const reads = new Map(now.slots.map(read => [read.index, read]));
  const out: ActiveSlot[] = [];
  for (const slot of slotsByIndex(history)) {
    if (!(slot.availability[0]?.firstHour <= hour)) continue;
    if (!covered) {
      if (lastRead !== undefined && slot.availability.some(span => span.lastHour === lastRead)) out.push({ kind: 'unavailable', slot, reason: 'Not recorded at this hour' });
      continue;
    }
    if (!slot.availability.some(span => span.firstHour <= hour && hour <= span.lastHour)) continue;
    const read = reads.get(slot.index);
    if (!read) {
      out.push({ kind: 'unavailable', slot, reason: now.missing ?? 'Not recorded at this hour' });
      continue;
    }
    const entry = researchEntryAt(slot, read.state, hour);
    if (!entry && !isUnjoinedResearch(read.state)) {
      out.push({ kind: 'idle', slot, read });
      continue;
    }
    if (read.state.token === null || read.state.databaseIndex === null) {
      out.push({ kind: 'unavailable', slot, reason: 'The research read names no technology definition' });
      continue;
    }
    out.push({ kind: 'researching', slot, read, entry, token: read.state.token, databaseIndex: read.state.databaseIndex });
  }
  return out;
}
