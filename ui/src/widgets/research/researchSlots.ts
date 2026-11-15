import { researchNumber } from './researchBreakdown.ts';
import type { ResearchSlotNow } from '../../generated/dashboard';
import type { ResearchSlotsReply } from '../../generated/dashboard';

/** What renders of the slots at the head: which slots hold what, and which numbers are known at all. Their values
 * (progress, days left, saved days) are painted, so a new read renders nothing unless this changes. */
export function slotsStructure(reply: ResearchSlotsReply): string {
  return JSON.stringify([reply.missing, reply.slots.map(slot => [slot.index, slot.state.token, slot.state.entryFirstObservedHour,
    slot.progress?.baseDaysRaw != null, slot.state.details?.remainingDays != null, BigInt(slot.state.bankedDaysRaw) > 0n]),
  reply.grants.unavailable, reply.grants.grants.map(grant => [grant.kind, grant.ordinal, grant.token, grant.uses, grant.activeClaims])]);
}

export interface SlotNumbers {
  /** Recorded progress toward the base cost, 0 to 1; null where not known. */
  fraction: number | null;
  percent: string;
  /** The game's days-left estimate at `hour`, counted down from the read; null where not known. */
  left: number | null;
  days: string;
  saved: string;
}

/** The read's progress toward the base cost, 0 to 1; null where not known. */
function readFraction(read: ResearchSlotNow): number | null {
  const sample = read.progress;
  if (!sample?.baseDaysRaw) return null;
  const base = BigInt(sample.baseDaysRaw), done = BigInt(sample.progressRaw);
  return base > 0n && done >= 0n ? done >= base ? 1 : Number(done * 10_000n / base) / 10_000 : null;
}

/** The numbers one slot shows at `hour`, from its latest read; `progress` is its live progress cell where followed
 * (progress over the base cost), which a value not known leaves to the read's. */
export function slotNumbers(read: ResearchSlotNow, hour: number, progress?: number): SlotNumbers {
  const live = progress !== undefined && Number.isFinite(progress) && progress >= 0 && read.progress?.baseDaysRaw ? Math.min(1, progress) : undefined;
  const fraction = live ?? readFraction(read);
  const remaining = read.state.details?.remainingDays ?? null;
  const left = remaining === null ? null : Math.max(0, remaining - Math.floor((hour - read.state.firstHour) / 24));
  const banked = BigInt(read.state.bankedDaysRaw);
  return { fraction, percent: fraction === null ? '' : `${Math.round(fraction * 100)}%`, left,
    days: left === null ? 'Researching' : `${left} ${left === 1 ? 'day' : 'days'} left`,
    saved: banked > 0n ? `${researchNumber(banked)} days saved` : '' };
}

/** Writes each slot's numbers at `hour` into the elements under `root` naming it (`data-research-slot`), never by a
 * render: `data-paint` names the text (`percent`, `days`, `saved`); a slot's meter is its `.progress-meter > i`.
 * `progress` gives a slot's live progress cell. */
export function paintResearchSlots(root: Element | null, reply: ResearchSlotsReply, hour: number, progress: (slot: number) => number | undefined = () => undefined): void {
  if (!root) return;
  for (const read of reply.slots) {
    const numbers = slotNumbers(read, hour, progress(read.index));
    for (const element of root.querySelectorAll(`[data-research-slot="${read.index}"]`)) {
      for (const name of ['percent', 'days', 'saved'] as const) {
        const target = element.querySelector(`[data-paint="${name}"]`);
        if (target && target.textContent !== numbers[name]) target.textContent = numbers[name];
      }
      const width = numbers.fraction === null ? null : `${numbers.fraction * 100}%`;
      const bar = element.querySelector<HTMLElement>('[data-paint="bar"]');
      if (bar && width !== null && bar.style.width !== width) bar.style.width = width;
      const meter = element.querySelector<HTMLElement>('.progress-meter > i');
      if (meter && width !== null && meter.style.width !== width) meter.style.width = width;
    }
  }
}
