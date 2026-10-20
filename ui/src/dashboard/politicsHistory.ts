// Shared reading of the prepared country political-state history
// (`politics-state-history`): its envelope, holder tag runs and value runs.
import type { HolderTag, HourSpan, PoliticsHistoryMode, PoliticsHistoryReply, PoliticsRun } from '../generated/dashboard';
import type { Span } from '../charts/series';
import { isOrderedSpans, runAt } from '../charts/series.ts';
import { object, opaqueTag } from './validation.ts';
import { pendingAt, preparedPrefix } from './preparedPrefix.ts';

export const POLITICS_HISTORY = 'politics-state-history';

/** A decimal integer that is exact as a number. */
export const integerText = (value: string): boolean => /^-?\d{1,16}$/.test(value) && Number.isSafeInteger(Number(value));
/** A native fixed5 value as its exact decimal string. */
export const fixed5Text = (value: string): boolean => /^-?\d{1,19}$/.test(value)
  && BigInt(value) >= -(1n << 63n) && BigInt(value) < (1n << 63n);

/** Ordered, disjoint runs whose values all pass `check`. */
export function isPoliticsRuns(value: unknown, check: (value: string) => boolean): value is PoliticsRun[] {
  return isOrderedSpans(value) && value.every(run => typeof run.value === 'string' && check(run.value));
}

/** A holder's tags, each from its hour until the next entry's. */
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

/** The holder's tag at `hour`; null before it was first read with one. */
export function tagAt(tags: readonly HolderTag[], hour: number): string | null {
  let low = 0, high = tags.length - 1, found = -1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (tags[middle].hour <= hour) { found = middle; low = middle + 1; } else high = middle - 1;
  }
  return found < 0 ? null : tags[found].tag;
}

/** The value a run holds at `hour`; null where no run covers it (not read). */
export const valueAt = (runs: readonly PoliticsRun[], hour: number): string | null => runAt(runs, hour)?.value ?? null;
export function integerAt(runs: readonly PoliticsRun[], hour: number): number | null {
  const value = valueAt(runs, hour);
  return value === null ? null : Number(value);
}

/** The reply's envelope for `mode`; `body` checks the mode's own fields. */
export function validatePoliticsHistory(value: unknown, mode: PoliticsHistoryMode, body: (reply: Record<string, unknown>) => boolean): PoliticsHistoryReply {
  if (!object(value) || typeof value.branchId !== 'string' || value.mode !== mode
    || !(value.lastHour === null || typeof value.lastHour === 'number' && Number.isSafeInteger(value.lastHour))
    || !isOrderedSpans(value.known)
    || !Array.isArray(value.warnings) || value.warnings.some(warning => typeof warning !== 'string') || !body(value))
    throw new Error('The political state history reply is malformed.');
  // A malformed prepared prefix throws here, not while drawing.
  preparedPrefix(value);
  return value as unknown as PoliticsHistoryReply;
}

/** Whether the reply answers `hour`: it is prepared and not still pending. */
export function preparedThroughHour(reply: Pick<PoliticsHistoryReply, 'lastHour'>, hour: number): boolean {
  return reply.lastHour !== null && hour <= reply.lastHour && !pendingAt(preparedPrefix(reply), hour);
}

/** Whether the reply's part was read at `hour`. */
export const readAt = (known: readonly HourSpan[], hour: number): boolean => runAt(known, hour) !== null;

/** The hours `firstHour..=lastHour` the part was not read: a value cannot be said
 * to start or end there. */
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

/** The hours a part's values cannot be said to start or end at: recorded gaps, the
 * hours `firstHour..=lastHour` the part was not read, and hours still being prepared,
 * in order and merged where they overlap or meet, so a stretch inside another never
 * reads as its end. */
export function unknownHours(gaps: readonly HourSpan[], known: readonly HourSpan[], firstHour: number, lastHour: number | null,
  pending: readonly HourSpan[]): Span[] {
  const spans = [...gaps, ...unreadSpans(known, firstHour, lastHour), ...pending].sort((a, b) => a.firstHour - b.firstHour);
  const merged: Span[] = [];
  for (const span of spans) {
    const last = merged.at(-1);
    if (last && span.firstHour <= last.lastHour + 1) last.lastHour = Math.max(last.lastHour, span.lastHour);
    else merged.push({ firstHour: span.firstHour, lastHour: span.lastHour });
  }
  return merged;
}

/** The fixed week of hours holding `hour`, counted from `branchFirst`: one
 * exile history request answers a week, so following asks once a week. */
export const HISTORY_WINDOW_HOURS = 7 * 24;
export function historyWindow(branchFirst: number, hour: number): HourSpan {
  const firstHour = branchFirst + Math.floor((hour - branchFirst) / HISTORY_WINDOW_HOURS) * HISTORY_WINDOW_HOURS;
  return { firstHour, lastHour: firstHour + HISTORY_WINDOW_HOURS - 1 };
}

/** A stable key for a holder: its first recorded tag and hour. */
export const holderKey = (tags: readonly HolderTag[]): string | null => tags.length ? JSON.stringify([tags[0].hour, tags[0].tag]) : null;
