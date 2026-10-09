import type { BalanceDrop, FixedRun, MeasureReply, MeasureSeries } from '../../generated/dashboard';
import type { Run } from '../../charts/series';
import { runAt } from '../../charts/series.ts';
import { fixedNumber, formatFixed } from '../../charts/valueAxis.ts';

const RAW = /^-?\d{1,20}$/;

export const isFixedRun = (value: unknown): value is FixedRun => typeof value === 'object' && value != null
  && Number.isInteger((value as FixedRun).firstHour) && Number.isInteger((value as FixedRun).lastHour)
  && typeof (value as FixedRun).startsSegment === 'boolean'
  && (value as FixedRun).firstHour <= (value as FixedRun).lastHour && RAW.test((value as FixedRun).valueRaw);
const isDrop = (value: unknown): value is BalanceDrop => typeof value === 'object' && value != null
  && Number.isInteger((value as BalanceDrop).hour) && RAW.test((value as BalanceDrop).beforeRaw) && RAW.test((value as BalanceDrop).afterRaw);

/** Fixed runs in hour order that never overlap. */
export const orderedRuns = (runs: unknown): runs is FixedRun[] => Array.isArray(runs) && runs.every(isFixedRun)
  && runs.every((run, index) => index === 0 || runs[index - 1].lastHour < run.firstHour);

/** Checks the reply's shape; runs must be in hour order and never overlap. */
export function validateMeasureReply(data: unknown): MeasureReply {
  const reply = data as MeasureReply;
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string'
    && Number.isInteger(reply.decimals) && reply.decimals >= 0 && reply.decimals <= 12
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string')
    && Array.isArray(reply.series) && reply.series.every(series => typeof series.tag === 'string'
      && orderedRuns(series.runs)
      && Array.isArray(series.drops) && series.drops.every(isDrop));
  if (!ok) throw new Error('The balance reply is malformed.');
  return reply;
}

/** Runs as plotting numbers; labels use the exact raw values. */
export function toRuns(series: Pick<MeasureSeries, 'runs'>, decimals: number): Run[] {
  return series.runs.map(run => ({ firstHour: run.firstHour, lastHour: run.lastHour, value: fixedNumber(run.valueRaw, decimals), breakBefore: run.startsSegment }));
}

/** The exact recorded balance at an hour, or null when it was not recorded. */
export function rawAt(series: Pick<MeasureSeries, 'runs'>, hour: number): string | null {
  return runAt(series.runs, hour)?.valueRaw ?? null;
}

export const formatBalance = (raw: string, decimals: number) => formatFixed(raw, decimals, 2);

/** A drop's net change, negative, formatted exactly. */
export function netSpend(drop: BalanceDrop, decimals: number): string {
  return formatFixed(BigInt(drop.afterRaw) - BigInt(drop.beforeRaw), decimals, 2);
}

/** The drop nearest `hour` within `tolerance` hours, for snapping hover to a tick. */
export function nearestDrop<T extends { hour: number }>(drops: readonly T[], hour: number, tolerance: number): T | null {
  let best: T | null = null;
  for (const drop of drops) {
    const distance = Math.abs(drop.hour - hour);
    if (distance <= tolerance && (!best || distance < Math.abs(best.hour - hour))) best = drop;
  }
  return best;
}
