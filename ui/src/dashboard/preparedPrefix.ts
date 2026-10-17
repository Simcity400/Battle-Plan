import type { HourSpan } from '../generated/dashboard';
import { isPreparedCoverage } from '../shell/preparation/contract.ts';
import type { PreparationReply, PreparedCoverage } from '../shell/preparation/contract';
import { WALK_GATE } from './client.ts';
import { isOrderedSpans } from '../charts/series.ts';

/** A reply answered from its gate's prepared prefix (`PreparedPrefix`): the
 * coverage it reads through and the requested hours still being prepared. */
export interface PreparedPrefixFields { coverage: PreparedCoverage; pending: readonly HourSpan[]; approximate?: readonly HourSpan[] }

/** The prepared prefix a reply's root carries; null for a reply without one. */
export function preparedPrefix(reply: unknown): PreparedPrefixFields | null {
  if (!reply || typeof reply !== 'object' || !('pending' in reply)) return null;
  const { pending, approximate } = reply as { pending?: unknown; approximate?: unknown };
  const coverage = preparationCoverage(reply);
  if (coverage === undefined || !isOrderedSpans(pending) || approximate !== undefined && !isOrderedSpans(approximate)) throw new Error('The prepared prefix of this reply is malformed.');
  return { coverage, pending, ...(approximate === undefined ? {} : { approximate: approximate as HourSpan[] }) };
}

/** Preparation's extent; a product may already use coverage for capture evidence. */
export function preparationCoverage(reply: unknown): PreparedCoverage | undefined {
  if (!reply || typeof reply !== 'object') return undefined;
  const fields = reply as { coverage?: unknown; preparedCoverage?: unknown };
  const value = 'preparedCoverage' in fields ? fields.preparedCoverage : fields.coverage;
  return isPreparationExtent(value) ? value : undefined;
}

/** A prepared reply is final only when it leaves no unfinished spans or extent.
 * Products without preparation metadata retain their ordinary one-shot semantics. */
export function finalPreparedReply(reply: unknown): boolean {
  if (reply == null) return false;
  if (typeof reply !== 'object') return true;
  const fields = reply as { coverage?: unknown; preparedCoverage?: unknown; pending?: unknown; approximate?: unknown };
  for (const spans of [fields.pending, fields.approximate])
    if (Array.isArray(spans) && spans.length > 0) return false;
  const coverage = preparationCoverage(reply);
  if (coverage) return coverage.complete;
  return !('preparedCoverage' in fields) && (fields.coverage === undefined || Array.isArray(fields.coverage));
}

/** Accepted hours, or the empty extent immediately before the first hour. */
export function isPreparationExtent(value: unknown): value is PreparedCoverage {
  if (isPreparedCoverage(value)) return true;
  if (!value || typeof value !== 'object') return false;
  const empty = value as PreparedCoverage;
  return Number.isSafeInteger(empty.firstHour) && Number.isSafeInteger(empty.lastHour)
    && empty.lastHour + 1 === empty.firstHour && empty.complete === false;
}

export const preparedPending = (reply: unknown): readonly HourSpan[] => preparedPrefix(reply)?.pending ?? [];
export const pendingReplyAt = (reply: unknown, hour: number) => pendingAt(preparedPrefix(reply), hour);

export const fieldPreparing = (row: unknown, field: string): boolean => !!row && typeof row === 'object'
  && 'preparing' in row && Array.isArray(row.preparing) && row.preparing.includes(field);

/** A presented value whose later observations or classifications can revise it. */
export const approximateAt = (prefix: Pick<PreparedPrefixFields, 'approximate'> | null | undefined, hour: number) =>
  !!prefix?.approximate?.some(span => span.firstHour <= hour && hour <= span.lastHour);

/** Whether `hour` is still being prepared in a reply answered from a prefix. */
export const pendingAt = (prefix: Pick<PreparedPrefixFields, 'pending'> | null | undefined, hour: number) =>
  !!prefix?.pending.some(span => span.firstHour <= hour && hour <= span.lastHour);

/** The hours a prefix reply marks as not observed, for semantics where an
 * unobserved hour beside a span is a gap rather than a change: the recorded
 * gaps the reply names, and the hours still being prepared, in order. */
export function unobservedOrPending(unobserved: readonly HourSpan[], prefix: PreparedPrefixFields | null | undefined): HourSpan[] {
  if (!prefix?.pending.length) return [...unobserved];
  return [...unobserved, ...prefix.pending].sort((a, b) => a.firstHour - b.firstHour || a.lastHour - b.lastHour);
}

/** The last hour every reply answers through `hour`: `hour`, or the earliest
 * last hour of a prefix reply that leaves hours still being prepared. */
export function preparedThrough(replies: readonly unknown[] | undefined, hour: number): number {
  let through = hour;
  for (const reply of replies ?? []) {
    const prefix = preparedPrefix(reply);
    if (prefix?.pending.length) through = Math.min(through, prefix.coverage.lastHour);
  }
  return through;
}

/** Whether `status` shows the coverage a prefix reply on `branch` waits for
 * (its `gate`: families prepared on their own, or the hourly walk for `walk`
 * or none) past the reply's `coverage`, so asking again answers more. A
 * status that lists none of it moves once preparation is ready. */
export function gateMoved(status: PreparationReply, gate: readonly string[] | undefined, branch: string, coverage: PreparedCoverage): boolean {
  const covered = (gate?.length ? gate : [WALK_GATE]).map(entry => (entry === WALK_GATE ? status.coverage : status.families?.[entry])?.[branch]);
  if (covered.some(value => !value)) return status.state === 'ready';
  return covered.every(value => value!.complete) || Math.min(...covered.map(value => value!.lastHour)) > coverage.lastHour;
}
