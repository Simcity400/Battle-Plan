import type { BalanceDrop, FixedRun, PoliticalPowerReply, PoliticalPowerSeries } from '../../generated/dashboard';
import type { Run } from '../../charts/series';
import { fixedNumber, formatFixed } from '../../charts/valueAxis.ts';

/** Most countries one Political power widget draws. */
export const MAX_COUNTRIES = 16;
const RAW = /^-?\d{1,20}$/;

const isRun = (value: unknown): value is FixedRun => typeof value === 'object' && value != null
  && Number.isInteger((value as FixedRun).firstHour) && Number.isInteger((value as FixedRun).lastHour)
  && typeof (value as FixedRun).startsSegment === 'boolean'
  && (value as FixedRun).firstHour <= (value as FixedRun).lastHour && RAW.test((value as FixedRun).valueRaw);
const isDrop = (value: unknown): value is BalanceDrop => typeof value === 'object' && value != null
  && Number.isInteger((value as BalanceDrop).hour) && RAW.test((value as BalanceDrop).beforeRaw) && RAW.test((value as BalanceDrop).afterRaw);

/** Checks the reply's shape; runs must be in hour order and never overlap. */
export function validatePoliticalPowerReply(data: unknown): PoliticalPowerReply {
  const reply = data as PoliticalPowerReply;
  const ok = typeof reply === 'object' && reply != null && typeof reply.branchId === 'string'
    && Number.isInteger(reply.decimals) && reply.decimals >= 0 && reply.decimals <= 12
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string')
    && Array.isArray(reply.series) && reply.series.every(series => typeof series.tag === 'string'
      && Array.isArray(series.runs) && series.runs.every(isRun)
      && series.runs.every((run, index) => index === 0 || series.runs[index - 1].lastHour < run.firstHour)
      && Array.isArray(series.drops) && series.drops.every(isDrop));
  if (!ok) throw new Error('The political power reply is malformed.');
  return reply;
}

/** Runs as plotting numbers; labels use the exact raw values. */
export function toRuns(series: PoliticalPowerSeries, decimals: number): Run[] {
  return series.runs.map(run => ({ firstHour: run.firstHour, lastHour: run.lastHour, value: fixedNumber(run.valueRaw, decimals), breakBefore: run.startsSegment }));
}

/** The exact recorded balance at an hour, or null when it was not recorded. */
export function rawAt(series: PoliticalPowerSeries, hour: number): string | null {
  let low = 0;
  let high = series.runs.length - 1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    const run = series.runs[middle];
    if (hour < run.firstHour) high = middle - 1;
    else if (hour > run.lastHour) low = middle + 1;
    else return run.valueRaw;
  }
  return null;
}

export const formatBalance = (raw: string, decimals: number) => formatFixed(raw, decimals, 2);

/** A drop's net change, negative, formatted exactly. */
export function netSpend(drop: BalanceDrop, decimals: number): string {
  return formatFixed(BigInt(drop.afterRaw) - BigInt(drop.beforeRaw), decimals, 2);
}

/** The drop nearest `hour` within `tolerance` hours, for snapping hover to a tick. */
export function nearestDrop(drops: readonly BalanceDrop[], hour: number, tolerance: number): BalanceDrop | null {
  let best: BalanceDrop | null = null;
  for (const drop of drops) {
    const distance = Math.abs(drop.hour - hour);
    if (distance <= tolerance && (!best || distance < Math.abs(best.hour - hour))) best = drop;
  }
  return best;
}
