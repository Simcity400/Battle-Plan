import { countryKey } from '../../dashboard/countryKey.ts';
import type { SeriesDrop, SeriesRun, SeriesState } from '../../generated/dashboard';

export interface MeasureStates {
  branchId: string;
  measure: string;
  decimals: number;
  states: readonly SeriesState[];
}
const raw = (value: unknown): value is string => typeof value === 'string' && /^-?\d+$/.test(value);
const hour = (value: unknown): value is number => Number.isSafeInteger(value);
const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null;
const decimals = (value: unknown): value is number => Number.isInteger(value) && (value as number) >= 0 && (value as number) <= 12;
const drop = (value: unknown): value is SeriesDrop => object(value) && hour(value.hour) && raw(value.beforeRaw) && raw(value.afterRaw);
const run = (value: unknown): value is SeriesRun => object(value) && hour(value.firstHour) && hour(value.lastHour)
  && value.firstHour <= value.lastHour && raw(value.valueRaw) && typeof value.startsSegment === 'boolean'
  && Number.isSafeInteger(value.slot) && Number.isSafeInteger(value.countryId) && hour(value.segmentFirstHour)
  && value.segmentFirstHour <= value.firstHour && typeof value.firstExact === 'boolean' && typeof value.lastExact === 'boolean';

export function validateMeasureStates(value: unknown): MeasureStates {
  if (!object(value) || typeof value.branchId !== 'string' || typeof value.measure !== 'string' || !decimals(value.decimals)
    || !Array.isArray(value.states) || !value.states.every(state => object(state) && (typeof state.country === 'string' || typeof state.country === 'number') && hour(state.hour)
      && (state.run === null || run(state.run)) && (state.drop === null || drop(state.drop))
      && (state.missing === null || typeof state.missing === 'string')
      && (state.notApplicable === undefined || typeof state.notApplicable === 'boolean')))
    throw new Error('The balance hour reply is malformed.');
  return value as unknown as MeasureStates;
}

export interface ReachedLine {
  tag: string;
  runs: SeriesRun[];
  drops: SeriesDrop[];
  missing: Map<number, string>;
  notApplicable: Set<number>;
}

export function reachedLines(tags: readonly string[], states: MeasureStates | undefined): ReachedLine[] {
  return tags.map(tag => {
    const reached = states?.states.filter(state => countryKey(state.country) === tag) ?? [];
    const unique = new Map<string, SeriesRun>();
    for (const state of reached) if (state.run) unique.set(JSON.stringify([state.run.firstHour, state.run.lastHour, state.run.valueRaw, state.run.countryId]), state.run);
    return { tag, runs: [...unique.values()].sort((a, b) => a.firstHour - b.firstHour),
      drops: reached.flatMap(state => state.drop ? [state.drop] : []),
      missing: new Map(reached.flatMap(state => state.missing === null ? [] : [[state.hour, state.missing] as const])),
      notApplicable: new Set(reached.flatMap(state => state.notApplicable === true ? [state.hour] : [])) };
  });
}
