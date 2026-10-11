import type { NavyParams } from '../../generated/dashboard';
import { validateNativePartsRaceReply } from '../../charts/race/partsRaceDrill.ts';
import type { NativePartsRaceReply } from '../../charts/race/partsRaceDrill';
import type { OptionValue } from '../../dashboard/options';
import { formatAxisValue, formatFixed } from '../../charts/valueAxis.ts';
import { usNumber } from '../../charts/numberFormat.ts';

export function validateNavyReply(data: unknown): NativePartsRaceReply {
  const reply = validateNativePartsRaceReply(data);
  if (reply.series.some(series => series.runs.some(run => run.referenceTotal === undefined))) {
    throw new Error('The navy size reply is malformed.');
  }
  return reply;
}

/** Hull HP is fixed5; preserve all five recorded decimals. */
export const formatNavyValue = (value: number, options: Readonly<Record<string, OptionValue>>) =>
  options.measure === 'strength' ? formatFixed(BigInt(Math.round(value)), 5, 5).replace(/\.?0+$/, '') : usNumber.format(value);

/** Settings are in displayed HP; chart geometry and recorded totals are fixed5. */
export function navyChartOptions(options: Readonly<Record<string, OptionValue>>): Readonly<Record<string, OptionValue>> {
  if (options.measure !== 'strength') return options;
  return { ...options,
    yMin: typeof options.yMin === 'number' ? options.yMin * 100_000 : options.yMin,
    yMax: typeof options.yMax === 'number' ? options.yMax * 100_000 : options.yMax };
}

/** Axis geometry may be fractional; fixed5 HP uses the shared tick precision. */
export const formatHullTick = (value: number, ticks: readonly number[]) =>
  formatAxisValue(value / 100_000, ticks.map(tick => tick / 100_000));

export function navyRequest(branchId: string, resolved: { tags: readonly string[]; top?: number },
  options: Readonly<Record<string, OptionValue>>): NavyParams {
  return { branchId, tags: resolved.top == null ? [...resolved.tags] : [], top: resolved.top ?? null,
    ownership: options.ownership === 'logical' ? 'logical' : 'roster', grouping: options.grouping === 'type' ? 'type' : 'class',
    measure: options.measure === 'strength' ? 'strength' : 'ships' };
}
