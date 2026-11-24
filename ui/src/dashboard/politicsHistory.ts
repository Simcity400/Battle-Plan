import type { HolderTag, HourSpan, PoliticsHistoryMode, PoliticsHistoryReply, PoliticsRun } from '../generated/dashboard';
import type { Span } from '../charts/series';
import { isOrderedSpans, runAt } from '../charts/series.ts';
import { object, opaqueTag } from './validation.ts';

export const POLITICS_HISTORY = 'politics-state-history';


export const integerText = (value: string): boolean => /^-?\d{1,16}$/.test(value) && Number.isSafeInteger(Number(value));
export const fixed5Text = (value: string): boolean => /^-?\d{1,19}$/.test(value)
  && BigInt(value) >= -(1n << 63n) && BigInt(value) < (1n << 63n);

export function isPoliticsRuns(value: unknown, check: (value: string) => boolean): value is PoliticsRun[] {
  return isOrderedSpans(value) && value.every(run => typeof run.value === 'string' && check(run.value));
}

export function isHolderTags(value: unknown): value is HolderTag[] {
  if (!Array.isArray(value)) return false;
  let last: number | null = null;
  for (const entry of value) {
    if (!object(entry) || typeof entry.hour !== 'number' || !Number.isSafeInteger(entry.hour) || !opaqueTag(entry.tag)
      || (last !== null && entry.hour <= last)) return false;
    last = entry.hour;
  }
  return true;
}

export function tagAt(tags: readonly HolderTag[], hour: number): string | null {
  let low = 0, high = tags.length - 1, found = -1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (tags[middle].hour <= hour) { found = middle; low = middle + 1; } else high = middle - 1;
  }
  return found < 0 ? null : tags[found].tag;
}

export const valueAt = (runs: readonly PoliticsRun[], hour: number): string | null => runAt(runs, hour)?.value ?? null;
export function integerAt(runs: readonly PoliticsRun[], hour: number): number | null {
  const value = valueAt(runs, hour);
  return value === null ? null : Number(value);
}

export function validatePoliticsHistory(value: unknown, mode: PoliticsHistoryMode, body: (reply: Record<string, unknown>) => boolean): PoliticsHistoryReply {
  if (!object(value) || typeof value.branchId !== 'string' || value.mode !== mode
    || !(value.lastHour === null || typeof value.lastHour === 'number' && Number.isSafeInteger(value.lastHour))
    || !isOrderedSpans(value.known)
    || !Array.isArray(value.warnings) || value.warnings.some(warning => typeof warning !== 'string') || !body(value))
    throw new Error('The political state history reply is malformed.');
  return value as unknown as PoliticsHistoryReply;
}

export const readAt = (known: readonly HourSpan[], hour: number): boolean => runAt(known, hour) !== null;

export function unreadSpans(known: readonly HourSpan[], firstHour: number, lastHour: number | null): Span[] {
  if (lastHour === null) return [];
  const spans: Span[] = [];
  let from = firstHour;
  for (const span of known) {
    if (span.firstHour > lastHour) break;
    if (span.firstHour > from) spans.push({ firstHour: from, lastHour: span.firstHour - 1 });
    from = Math.max(from, span.lastHour + 1);
  }
  if (from <= lastHour) spans.push({ firstHour: from, lastHour });
  return spans;
}

export function unknownHours(gaps: readonly HourSpan[], known: readonly HourSpan[], firstHour: number, lastHour: number | null): Span[] {
  const spans = [...gaps, ...unreadSpans(known, firstHour, lastHour)].sort((a, b) => a.firstHour - b.firstHour);
  const merged: Span[] = [];
  for (const span of spans) {
    const last = merged.at(-1);
    if (last && span.firstHour <= last.lastHour + 1) last.lastHour = Math.max(last.lastHour, span.lastHour);
    else merged.push({ firstHour: span.firstHour, lastHour: span.lastHour });
  }
  return merged;
}

export const holderKey = (tags: readonly HolderTag[]): string | null => tags.length ? JSON.stringify([tags[0].hour, tags[0].tag]) : null;
