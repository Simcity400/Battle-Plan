import { isPreparationExtent } from '../../dashboard/preparedPrefix.ts';
import type { PreparedCoverage } from '../../shell/preparation/contract';
import type { BalanceDrop, FixedRun, MeasureReply, ExperienceRead } from '../../generated/dashboard';
import type { ChartSummary } from '../../charts/chartSummary';
import type { MarkSummary } from '../../charts/markSummary';
import { isChartSummary } from '../../charts/chartSummary.ts';
import { validateExperienceReply } from '../experience/experienceData.ts';

export interface MeasureState {
  tag: string;
  hour: number;
  approximate?: boolean;
  sampleHour?: number | null;
  run: (FixedRun & { slot: number; countryId: number; segmentFirstHour?: number; readHour?: number }) | null;
  drop: BalanceDrop | null;
  recorded?: boolean | null;
  readHour?: number | null;
  experience?: ExperienceRead | null;
  available?: boolean | null;
  segmentFirstHour?: number | null;
}
export interface MeasureStates {
  coverage?: PreparedCoverage;
  branchId: string;
  measure: string;
  decimals: number;
  states: readonly MeasureState[];
  scale?: number | null;
  dayHours?: number | null;
  warnings: string[];
}
export interface MeasureSummaries {
  coverage?: PreparedCoverage;
  branchId: string;
  measure: string;
  decimals: number;
  series: readonly { tag: string; summary: ChartSummary; marks: MarkSummary | null }[];
  warnings: string[];
}
export interface MeasureDrops {
  coverage?: PreparedCoverage;
  firstOrdinal: string;
  total: string;
  rows: { tag: string; hour: number; ordinal: string; row: BalanceDrop }[];
  warnings: string[];
  cursor?: string | null;
}

const raw = (value: unknown): value is string => typeof value === 'string' && /^-?\d+$/.test(value);
const ordinal = (value: unknown): value is string => typeof value === 'string' && /^(0|[1-9]\d*)$/.test(value);
const hour = (value: unknown): value is number => Number.isSafeInteger(value);
const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null;
const validCoverage = (value: Record<string, unknown>) => value.coverage === undefined || isPreparationExtent(value.coverage);
const warnings = (value: unknown): value is string[] => Array.isArray(value) && value.every(item => typeof item === 'string');
const drop = (value: unknown): value is BalanceDrop => object(value) && hour(value.hour) && raw(value.beforeRaw) && raw(value.afterRaw);
const run = (value: unknown): value is NonNullable<MeasureState['run']> => object(value) && hour(value.firstHour) && hour(value.lastHour)
  && value.firstHour <= value.lastHour && raw(value.valueRaw) && typeof value.startsSegment === 'boolean'
  && Number.isSafeInteger(value.slot) && Number.isSafeInteger(value.countryId)
  && (value.readHour === undefined || hour(value.readHour))
  && (value.segmentFirstHour === undefined || hour(value.segmentFirstHour) && value.segmentFirstHour <= value.firstHour);
const measure = (value: unknown): value is string => typeof value === 'string' && ['politicalPower', 'commandPower', 'stability', 'warSupport', 'maxCommandPower', 'states', 'provinces', 'victoryPoints', 'area'].includes(value) || typeof value === 'string' && /^(army|navy|air)-(banked|gain|change)$/.test(value);
const decimals = (measure: unknown) => /^(army|navy|air)-/.test(String(measure)) || ['states','provinces','victoryPoints','area'].includes(String(measure)) ? 0 : 5;

const validExperience = (read: unknown, scale: unknown, dayHours: unknown) => {
  try { validateExperienceReply({ branchId: '', scale, dayHours, warnings: [], series: [{ tag: 'read', reads: [read] }] }); return true; }
  catch { return false; }
};

export function validateMeasureStates(value: unknown): MeasureStates {
  if (!object(value) || !validCoverage(value) || typeof value.branchId !== 'string' || !measure(value.measure) || value.decimals !== decimals(value.measure)
    || !warnings(value.warnings) || !Array.isArray(value.states) || !value.states.every(state => object(state)
      && typeof state.tag === 'string' && hour(state.hour)
      && (state.approximate === undefined || typeof state.approximate === 'boolean')
      && (state.sampleHour === undefined || state.sampleHour === null || hour(state.sampleHour) && state.sampleHour <= state.hour)
      && (state.experience === undefined || state.experience === null || validExperience(state.experience, value.scale, value.dayHours))
      && (state.available === undefined || state.available === null || typeof state.available === 'boolean')
      && (state.segmentFirstHour === undefined || state.segmentFirstHour === null || hour(state.segmentFirstHour) && state.segmentFirstHour <= state.hour)
      && (state.recorded === undefined || state.recorded === null || typeof state.recorded === 'boolean')
      && (state.readHour === undefined || state.readHour === null || hour(state.readHour)) && (state.run === null || run(state.run)) && (state.drop === null || drop(state.drop))))
    throw new Error('The prepared balance hour reply is malformed.');
  return value as unknown as MeasureStates;
}

export function validateMeasureSummaries(value: unknown): MeasureSummaries {
  if (!object(value) || !validCoverage(value) || typeof value.branchId !== 'string' || !measure(value.measure) || value.decimals !== decimals(value.measure)
    || !warnings(value.warnings) || !Array.isArray(value.series) || !value.series.every(series => {
      if (!object(series) || typeof series.tag !== 'string' || !isChartSummary(series.summary)) return false;
      const summary = series.summary;
      if (series.marks === null) return true;
      if (!object(series.marks) || series.marks.from !== summary.from || series.marks.to !== summary.to || series.marks.width !== summary.width
        || !Array.isArray(series.marks.clusters) || series.marks.clusters.length > Math.ceil(Number(summary.width) / 18)) return false;
      return series.marks.clusters.every(cluster => object(cluster) && hour(cluster.firstHour) && hour(cluster.lastHour)
        && cluster.firstHour <= cluster.lastHour && ordinal(cluster.firstOrdinal) && ordinal(cluster.count) && cluster.count !== '0');
    })) throw new Error('The prepared balance summary reply is malformed.');
  return value as unknown as MeasureSummaries;
}

export function validateMeasureDrops(value: unknown): MeasureDrops {
  if (!object(value) || !validCoverage(value) || !ordinal(value.firstOrdinal) || !ordinal(value.total)
    || !(value.cursor === undefined || value.cursor === null || ordinal(value.cursor))
    || !warnings(value.warnings) || !Array.isArray(value.rows) || !value.rows.every(event => object(event) && typeof event.tag === 'string'
      && hour(event.hour) && ordinal(event.ordinal) && drop(event.row) && event.hour === event.row.hour))
    throw new Error('The prepared balance event page is malformed.');
  return value as unknown as MeasureDrops;
}

/** Only exact reached intervals become FixedRuns. Display extrema never supply
 * a balance or an invented integer amount. Duplicates in event pages stay exact. */
export function reachedMeasureReply<M extends string>(branchId: string, measure: M, tags: readonly string[], states: MeasureStates | undefined): Omit<MeasureReply, 'measure'> & { measure: M } {
  return { branchId, measure, decimals: states?.decimals ?? 5, warnings: states?.warnings ?? [], series: tags.map(tag => {
    const reached = states?.states.filter(state => state.tag === tag) ?? [];
    const unique = new Map<string, FixedRun>();
    for (const state of reached) if (state.run && !state.approximate) unique.set(JSON.stringify([state.run.firstHour, state.run.lastHour, state.run.valueRaw, state.run.slot, state.run.countryId]), state.run);
    return { tag, runs: [...unique.values()].sort((a, b) => a.firstHour - b.firstHour),
      drops: reached.flatMap(state => state.drop ? [state.drop] : []) };
  }) };
}

/** Scale only chart geometry; raw integers and supplying dates stay exact. */
export const scaleSummary = (summary: ChartSummary, factor: number): ChartSummary => ({ ...summary,
  runs: summary.runs?.map(run => ({ ...run, value: run.value * factor })), buckets: summary.buckets.map(bucket => bucket && ({ ...bucket,
    firstValue: bucket.firstValue * factor, lastValue: bucket.lastValue * factor, minValue: bucket.minValue * factor, maxValue: bucket.maxValue * factor })) });
