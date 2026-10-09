import type { TerritoryMeasure, TerritoryReply, TerritoryRule, TerritoryRun } from '../generated/dashboard';
import type { Span } from '../charts/series';
import { validCountryIdentityRuns } from './countryIdentity.ts';

export const MEASURES = [
  { value: 'states', label: 'States' }, { value: 'provinces', label: 'Provinces' },
  { value: 'victoryPoints', label: 'Victory points' }, { value: 'area', label: 'Map area' },
] as const;

export function measureOf(value: unknown): TerritoryMeasure {
  return MEASURES.find(measure => measure.value === value)?.value ?? 'states';
}

export function ruleOf(value: unknown): TerritoryRule {
  return value === 'owned' ? 'owned' : 'controlled';
}

export function measureChoices(reply: TerritoryReply | undefined) {
  return MEASURES.filter(measure => measure.value !== 'area' || reply?.areaUnit != null);
}

export function formatTerritory(value: number): string {
  return value.toLocaleString('en-US', { maximumFractionDigits: 0 });
}

/** Map cells describe the authored raster, whose projection enlarges the north. */
export function territoryNote(measure: TerritoryMeasure): string | undefined {
  return measure === 'area' ? 'Map cells are not equal-area; northern territory is enlarged.' : undefined;
}

export function territoryMeasureLabel(measure: TerritoryMeasure, rule: TerritoryRule, unit: string | null = null): string {
  const name = MEASURES.find(item => item.value === measure)!.label;
  return `${rule === 'owned' ? 'Owned' : 'Controlled'} ${name.toLowerCase()}${measure === 'area' && unit ? ` · ${unit}` : ''}`;
}

const hour = (value: unknown): value is number => Number.isInteger(value) && Number(value) >= -2147483648 && Number(value) <= 2147483647;
const spans = (value: unknown): value is Span[] => Array.isArray(value) && value.every((span, index) => span != null
  && hour(span.firstHour) && hour(span.lastHour) && span.firstHour <= span.lastHour
  && (index === 0 || value[index - 1].lastHour < span.firstHour));
const runs = (value: unknown): value is TerritoryRun[] => spans(value) && (value as TerritoryRun[]).every(run => typeof run.startsSegment === 'boolean'
  && Array.isArray(run.values) && run.values.length === 8
  && run.values.every((value, index) => value === null || (Number.isSafeInteger(value) && (index % 4 === 2 || value >= 0))));

export function validateTerritoryReply(data: unknown): TerritoryReply {
  const reply = data as TerritoryReply;
  if (reply == null || typeof reply !== 'object' || typeof reply.branchId !== 'string'
    || !Array.isArray(reply.warnings) || !reply.warnings.every(warning => typeof warning === 'string')
    || !(reply.areaUnit === null || typeof reply.areaUnit === 'string')
    || !(reply.areaReason === null || typeof reply.areaReason === 'string')
    || (reply.areaUnit === null) !== (reply.areaReason !== null)
    || !runs(reply.worldTotals)
    || !Array.isArray(reply.series) || new Set(reply.series.map(series => series?.tag)).size !== reply.series.length
    || !reply.series.every(series => series != null && typeof series.tag === 'string' && spans(series.segments)
      && validCountryIdentityRuns(series.identities) && (series.identities === undefined || spans(series.identities))
      && (series.membership === null || spans(series.membership)) && runs(series.runs))) {
    throw new Error('The territory reply is malformed.');
  }
  return reply;
}
