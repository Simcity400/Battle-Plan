import { researchNumber } from './researchBreakdown.ts';
import type { ResearchTimeSplit } from './researchJourney.ts';

export type ResearchSegmentTone = 'base' | 'penalty' | 'core' | 'slower' | 'paused' | 'saved' | 'juggle' | 'earlier' | 'remaining';

export interface ResearchSegment { key: string; value: bigint; tone: ResearchSegmentTone; name: string; kind?: string; effect: string; index?: number }

export interface ResearchTimeRows {
  expected: ResearchSegment[];
  took: ResearchSegment[];
  expectedTotal: bigint;
  elapsed: bigint;
  axis: bigint;
}

const days = (raw: bigint) => { const text = researchNumber(raw < 0n ? -raw : raw); return `${text} ${text === '1' ? 'day' : 'days'}`; };
const kindOf = (m: { label: string; kind: string }) =>
  m.label.toLowerCase().includes(m.kind) ? undefined : m.kind.charAt(0).toUpperCase() + m.kind.slice(1);
const total = (segments: readonly ResearchSegment[]) => segments.reduce((sum, s) => sum + s.value, 0n);

export function researchTimeRows(split: ResearchTimeSplit, active: boolean): ResearchTimeRows | null {
  const expectedTotal = split.base + split.penalty;
  if (expectedTotal <= 0n) return null;
  const slowed = split.modifiers.filter(m => m.days < 0n);
  const saved = split.modifiers.filter(m => m.days > 0n);
  const expected: ResearchSegment[] = [
    { key: 'base', value: split.base, tone: 'base' as const, name: 'Base research time', effect: days(split.base) },
    { key: 'penalty', value: split.penalty, tone: 'penalty' as const, name: 'Ahead-of-time penalty', effect: `+${days(split.penalty)}` },
    ...slowed.map((m, i) => ({ key: m.key, value: -m.days, tone: 'slower' as const, name: m.label, kind: kindOf(m), effect: `+${days(m.days)}`, index: i })),
    { key: 'other-slower', value: -split.other, tone: 'slower' as const, name: 'Other effects', effect: `+${days(split.other)}` },
    { key: 'juggle-returned', value: -split.juggle, tone: 'slower' as const, name: 'Saved days returned', effect: `+${days(split.juggle)}` },
  ].filter(s => s.value > 0n);
  const took: ResearchSegment[] = [
    { key: 'core', value: split.taken, tone: 'core' as const, name: active ? 'Time researched so far' : 'Time researched', effect: days(split.taken) },
    { key: 'paused', value: split.paused, tone: 'paused' as const, name: 'Not in a research slot', effect: `+${days(split.paused)}` },
    ...saved.map((m, i) => ({ key: m.key, value: m.days, tone: 'saved' as const, name: m.label, kind: kindOf(m), effect: `−${days(m.days)}`, index: i })),
    { key: 'other-saved', value: split.other, tone: 'saved' as const, name: split.floored ? 'Minimum research speed' : 'Other effects', effect: `−${days(split.other)}`, index: saved.length },
    { key: 'juggle', value: split.juggle, tone: 'juggle' as const, name: 'Saved research days', effect: `−${days(split.juggle)}` },
    { key: 'earlier', value: split.earlier, tone: 'earlier' as const, name: 'Progress made earlier', effect: `−${days(split.earlier)}` },
    { key: 'remaining', value: split.remaining, tone: 'remaining' as const, name: 'Still to research', effect: days(split.remaining) },
  ].filter(s => s.value > 0n);
  const expectedRow = total(expected), tookRow = total(took);
  return { expected, took, expectedTotal, elapsed: split.taken + split.paused, axis: tookRow > expectedRow ? tookRow : expectedRow };
}
