import type { CountryMetadata, CountrySample, CountrySeries, CountryValues, ReplayMetadata } from '../api';
import { formatDateTime, isEngineHour } from './calendar';

export type MetricKey = keyof CountryValues;

export const metrics: ReadonlyArray<{ key: MetricKey; label: string; multiplier: bigint; suffix: string }> = [
  { key: 'politicalPower', label: 'Political power', multiplier: 1n, suffix: '' },
  { key: 'commandPower', label: 'Command power', multiplier: 1n, suffix: '' },
  { key: 'stability', label: 'Stability', multiplier: 100n, suffix: '%' },
  { key: 'warSupport', label: 'War support', multiplier: 100n, suffix: '%' },
  { key: 'convoys', label: 'Convoys', multiplier: 1n, suffix: '' },
];

export interface HourRange { firstHour: number; lastHour: number }

export interface AnalyticsMemory {
  path: string | null;
  metadata: ReplayMetadata | null;
  branchId: string;
  countryId: string;
  metric: MetricKey;
  fromInput: string;
  toInput: string;
  range: HourRange | null;
  series: CountrySeries | null;
  seriesKey: string | null;
  selectedHour: number | null;
  scrollTop: number;
}

export function emptyMemory(path: string | null): AnalyticsMemory {
  return {
    path, metadata: null, branchId: '', countryId: '', metric: 'politicalPower',
    fromInput: '', toInput: '', range: null, series: null, seriesKey: null,
    selectedHour: null, scrollTop: 0,
  };
}

export const formatHour = (hour: number): string => hour.toLocaleString('en-US', { maximumFractionDigits: 0 });

export function formatMetric(raw: string | null | undefined, scale: string, key: MetricKey): string {
  if (raw == null) return 'Unavailable';
  const metric = metrics.find((item) => item.key === key)!;
  if (!/^-?\d+$/.test(raw) || !/^10*$/.test(scale)) return 'Unavailable';
  const numerator = BigInt(raw) * metric.multiplier;
  const denominator = BigInt(scale);
  const absolute = numerator < 0n ? -numerator : numerator;
  const whole = (absolute / denominator).toLocaleString('en-US');
  const places = scale.length - 1;
  const remainder = (absolute % denominator).toString().padStart(places, '0').replace(/0+$/, '');
  const fraction = remainder ? `.${remainder}` : '';
  return `${numerator < 0n ? '−' : ''}${whole}${fraction}${metric.suffix}`;
}

export function chartValue(raw: string | null | undefined, scale: string, key: MetricKey): number | null {
  if (raw == null || !/^-?\d+$/.test(raw) || !/^10*$/.test(scale)) return null;
  const metric = metrics.find((item) => item.key === key)!;
  // Floating point is used only to place a pixel. Text retains the exact integer.
  const value = Number(BigInt(raw) * metric.multiplier) / Number(scale);
  return Number.isFinite(value) ? value : null;
}

export function retainedSamples(series: CountrySeries): CountrySample[] {
  const samples = new Map<number, CountrySample>();
  for (const point of series.points) {
    if (point.first) samples.set(point.first.hour, point.first);
    if (point.last) samples.set(point.last.hour, point.last);
  }
  return [...samples.values()].sort((left, right) => left.hour - right.hour);
}

export function countryOptions(
  catalogue: CountryMetadata[],
  branchId: string,
  scanned: CountryMetadata[] = [],
): CountryMetadata[] {
  const byId = new Map<string, CountryMetadata>();
  for (const country of catalogue) {
    if (!byId.has(country.id) || country.observedBranchId === branchId) byId.set(country.id, country);
  }
  for (const country of scanned) byId.set(country.id, country);
  return [...byId.values()].sort((a, b) => countryLabel(a).localeCompare(countryLabel(b)) || a.id.localeCompare(b.id));
}

export function countryLabel(country: CountryMetadata): string {
  if (country.tag && country.name && country.tag !== country.name) return `${country.tag} · ${country.name}`;
  return country.name || country.tag || `Country ${country.id}`;
}

export function parseRange(from: string, to: string, bounds: HourRange): HourRange | string {
  if (!/^-?\d+$/.test(from) || !/^-?\d+$/.test(to)) return 'Choose a date and time for both ends of the range.';
  const firstHour = Number(from);
  const lastHour = Number(to);
  if (!isEngineHour(firstHour) || !isEngineHour(lastHour)) return 'Choose valid dates and times.';
  if (firstHour > lastHour) return 'The From date and time must be at or before To.';
  if (firstHour < bounds.firstHour || lastHour > bounds.lastHour) {
    return `Choose dates from ${formatDateTime(bounds.firstHour)} through ${formatDateTime(bounds.lastHour)} for this branch.`;
  }
  return { firstHour, lastHour };
}

export const coverageLabels = {
  recorded: 'Recorded', gap: 'Recording gap', unavailable: 'Unavailable',
  absent: 'Country absent', mixed: 'Mixed coverage',
} as const;
