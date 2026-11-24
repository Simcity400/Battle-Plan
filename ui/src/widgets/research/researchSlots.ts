import { formatProgress } from '../../families/valueFormat.ts';
import { researchNumber } from './researchBreakdown.ts';
import { researchDate } from './researchLayout.ts';
import { isUnjoinedResearch } from './researchData.ts';
import type { ResearchSlotNow } from '../../generated/dashboard';
import type { ResearchSlotsReply } from '../../generated/dashboard';

export function slotsStructure(reply: ResearchSlotsReply): string {
  return JSON.stringify([reply.missing, reply.slots.map(slot => [slot.index, slot.state.token, slot.state.databaseIndex, slot.state.entryFirstObservedHour,
    isUnjoinedResearch(slot.state) ? slot.state.firstHour : null,
    readFraction(slot) !== null, slot.state.details?.remainingDays != null, BigInt(slot.state.bankedDaysRaw) > 0n])]);
}

export interface SlotNumbers {
  fraction: number | null;
  percent: string;
  left: number | null;
  days: string;
  saved: string;
}

function readFraction(read: ResearchSlotNow): number | null {
  const sample = read.progress;
  if (!sample?.baseDaysRaw) return null;
  const base = BigInt(sample.baseDaysRaw), done = BigInt(sample.progressRaw);
  return base > 0n && done >= 0n ? done >= base ? 1 : Number(done * 10_000n / base) / 10_000 : null;
}

export function slotNumbers(read: ResearchSlotNow, hour: number, progress?: number): SlotNumbers {
  const live = progress !== undefined && Number.isFinite(progress) && progress >= 0 && read.progress?.baseDaysRaw ? Math.min(1, progress) : undefined;
  const fraction = live ?? readFraction(read);
  const remaining = read.state.details?.remainingDays ?? null;
  const left = remaining === null ? null : Math.max(0, remaining - Math.floor((hour - read.state.firstHour) / 24));
  const banked = BigInt(read.state.bankedDaysRaw);
  return { fraction, percent: fraction === null ? 'Not recorded' : formatProgress(fraction), left,
    days: left === null ? 'Not recorded' : `${left} ${left === 1 ? 'day' : 'days'} left`,
    saved: banked > 0n || read.state.token === null ? `${researchNumber(banked)} days saved` : '' };
}

export function slotValues(reply: ResearchSlotsReply, hour: number, progress?: (slot: number) => number | undefined): Map<string, string> {
  const values = new Map([['date', researchDate(hour)]]);
  for (const read of reply.slots) {
    const numbers = slotNumbers(read, hour, progress?.(read.index));
    for (const field of ['percent', 'days', 'saved'] as const) values.set(read.index + ':' + field, numbers[field]);
    values.set(read.index + ':bar', numbers.fraction === null ? '' : String(numbers.fraction));
    values.set(read.index + ':left', numbers.left === null ? '' : String(numbers.left));
    values.set(read.index + ':estimate', numbers.left === null ? '' : String(effectiveHour(read, hour)));
  }
  return values;
}

const effectiveHour = (read: ResearchSlotNow, hour: number) =>
  hour < read.state.firstHour ? hour : read.state.firstHour + Math.floor((hour - read.state.firstHour) / 24) * 24;
