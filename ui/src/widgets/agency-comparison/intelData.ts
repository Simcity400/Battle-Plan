import { formatDateTime, isEngineHour } from '../../dashboard/calendar.ts';
import type { TooltipContent } from '../../charts/tooltip';
import type { IntelDetail, IntelDetailsReply, IntelValue } from '../../generated/dashboard';

const values = (items: unknown): items is IntelValue[] => Array.isArray(items) && items.every(item => item && typeof item.name === 'string' && typeof item.value === 'string');
export function validateIntelDetails(input: unknown): IntelDetailsReply {
  const data = input as IntelDetailsReply;
  if (!data || !(data.viewer === null || Number.isInteger(data.viewer)) || !Number.isInteger(data.hour) || !(data.country === null || Number.isInteger(data.country))
    || !Array.isArray(data.countries) || !data.countries.every(country => country && Number.isInteger(country.id) && typeof country.tag === 'string')
    || !Array.isArray(data.entries) || !data.entries.every(entry => entry && typeof entry.key === 'string' && typeof entry.group === 'string' && typeof entry.label === 'string'
      && Number.isInteger(entry.viewer) && (entry.target === null || Number.isInteger(entry.target))
      && (entry.supplyingHour === null || (isEngineHour(entry.supplyingHour) && entry.supplyingHour <= data.hour)) && values(entry.values) && values(entry.explanations) && (entry.unavailable === null || typeof entry.unavailable === 'string'))
    || new Set(data.entries.map(entry => entry.key)).size !== data.entries.length
    || !Array.isArray(data.warnings) || !data.warnings.every(warning => typeof warning === 'string')) throw new Error('The country intel reply is malformed.');
  return data;
}
export function intelValues(row: IntelDetail): IntelValue[] {
  return row.supplyingHour === null ? row.values
    : [...row.values, { name: 'As of', value: formatDateTime(row.supplyingHour) }];
}
export function intelTooltip(row: IntelDetail): TooltipContent {
  return { title: row.label, subtitle: row.group, scrollable: true, scrollId: row.key,
    rows: [...intelValues(row), ...row.explanations].map(value => ({ label: value.name, value: value.value })),
    note: row.unavailable ? `Explanation unavailable: ${row.unavailable}` : undefined };
}
