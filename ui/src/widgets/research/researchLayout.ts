import type { ResearchSlotHistory } from './types';
import { formatDate, gameDate } from '../../dashboard/calendar.ts';
import { researchNumber } from './researchBreakdown.ts';
import { researchAssignments, researchStartKnown } from './researchJourney.ts';
import type { ResearchCoverage, ResearchEntry } from '../../generated/dashboard';

export function coverageAt(coverage: readonly ResearchCoverage[], hour: number): ResearchCoverage[] {
  return coverage.filter(span => span.firstHour <= hour).map(span => ({ firstHour: span.firstHour, lastHour: Math.min(span.lastHour, hour) }));
}

export function coverageGaps(coverage: readonly ResearchCoverage[]): { firstHour: number; lastHour: number }[] {
  return coverage.slice(1).map((span, index) => ({ firstHour: coverage[index]!.lastHour + 1, lastHour: span.firstHour - 1 }))
    .filter(gap => gap.lastHour >= gap.firstHour);
}

interface CompletedResearchRow {
  readonly kind: 'completed';
  readonly key: string;
  readonly hour: number;
  readonly slot: ResearchSlotHistory;
  readonly entry: ResearchEntry;
  readonly token: string;
  readonly started: { readonly hour: number; readonly known: boolean };
  readonly startSlot: number | null;
  readonly tookHours: number | null;
}

export function completedRow(slots: readonly ResearchSlotHistory[], slot: ResearchSlotHistory, entry: ResearchEntry, hour: number): CompletedResearchRow {
  const first = researchAssignments(slots, slot, entry)[0]!;
  const known = researchStartKnown(first.slot, first.entry);
  return {
    kind: 'completed',
    key: `c:${slot.index}:${entry.firstObservedHour}:${entry.token}`,
    hour,
    slot,
    entry,
    token: entry.token,
    started: { hour: first.entry.firstObservedHour, known },
    startSlot: first.slot.index !== slot.index ? first.slot.index : null,
    tookHours: known ? hour - first.entry.firstObservedHour : null,
  };
}

export function researchDate(hour: number): string {
  const date = gameDate(hour);
  return date ? formatDate(date) : 'Unavailable date';
}

export function researchDays(hours: number): string {
  const text = researchNumber(BigInt(hours) * 100_000n / 24n);
  return `${text} ${text === '1' ? 'day' : 'days'}`;
}
