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

export function estimateHour(shown: { texts: ReadonlyMap<string, string>; hour: number; data: ResearchSlotsReply | undefined } | undefined, asked: ResearchSlotsReply & { hour: number },
  slot: number, token: string): number {
  if (shown) {
    const painted = shown.texts.get(slot + ':estimate');
    return shown.data?.slots.find(held => held.index === slot)?.state.token === token && painted ? Number(painted) : shown.hour;
  }
  const read = asked.slots.find(held => held.index === slot);
  return !read || read.state.token !== token || read.state.details?.remainingDays == null ? asked.hour : effectiveHour(read, asked.hour);
}

export interface SlotInspection { fraction: number | null; left: number | null; hour: number; token?: string | null }

export function inspectedSlot(shown: { texts: ReadonlyMap<string, string>; hour: number; data: { slots: readonly { index: number; state: { token: string | null } }[] } | undefined } | undefined,
  slot: number): SlotInspection | undefined {
  if (!shown) return undefined;
  const values = slotInspection(shown.texts, slot, shown.hour) ?? { fraction: null, left: null, hour: shown.hour };
  return { ...values, token: shown.data?.slots.find(read => read.index === slot)?.state.token };
}

export function slotInspection(texts: ReadonlyMap<string, string> | undefined, slot: number, hour: number | null): SlotInspection | undefined {
  const bar = texts?.get(slot + ':bar'), left = texts?.get(slot + ':left');
  if (bar === undefined || left === undefined || hour === null) return undefined;
  return { fraction: bar === '' ? null : Number(bar), left: left === '' ? null : Number(left), hour };
}
