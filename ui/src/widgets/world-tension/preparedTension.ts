import type { HourSpan, TensionReply } from '../../generated/dashboard';
import type { ChartSummary } from '../../charts/chartSummary';
import { isChartSummary } from '../../charts/chartSummary.ts';
import { countryKey, validateTensionReply, tensionReadPreparing } from './tensionData.ts';
import { isOrderedSpans } from '../../charts/series.ts';
import { isPreparationExtent } from '../../dashboard/preparedPrefix.ts';
import type { PreparedCoverage } from '../../shell/preparation/contract';
import { overlayCountryIdentities } from '../../dashboard/countryIdentity.ts';

export interface PreparedTension extends TensionReply {
  available: boolean;
  coverage?: PreparedCoverage;
  pending?: HourSpan[];
  preparing?: string[];
  preparingContributors?: { hour: number; slot: number; valueRaw: string }[];
  catalogue: { key: string; tag: string; slot: number; firstHour: number }[];
  summaries: Record<string, ChartSummary>;
  exactHours: number[];
}
function validCatalogueKey(key: unknown): boolean {
  if (typeof key !== 'string') return false;
  if (/^(0|[1-9]\d*)$/.test(key)) return true;
  try { const parts: unknown = JSON.parse(key); return Array.isArray(parts) && parts.length === 2
    && typeof parts[0] === 'string' && Number.isSafeInteger(parts[1]) && parts[1] >= 0; } catch { return false; }
}
export function validatePreparedTension(value: unknown): PreparedTension {
  const reply = validateTensionReply(value) as PreparedTension;
  if (reply.coverage !== undefined && !isPreparationExtent(reply.coverage) || typeof reply.available !== 'boolean' || !Array.isArray(reply.catalogue) || !reply.catalogue.every(item => item && typeof item.tag === 'string'
    && validCatalogueKey(item.key) && Number.isSafeInteger(item.slot) && item.slot >= 0 && Number.isSafeInteger(item.firstHour))
    || !Array.isArray(reply.exactHours) || !reply.exactHours.every(Number.isSafeInteger)
    || (reply.pending !== undefined && !isOrderedSpans(reply.pending))
    || (reply.preparing !== undefined && (!Array.isArray(reply.preparing) || !reply.preparing.every(field => typeof field === 'string')))
    || (reply.preparingContributors !== undefined && (!Array.isArray(reply.preparingContributors) || !reply.preparingContributors.every(item => item
      && Number.isSafeInteger(item.hour) && Number.isSafeInteger(item.slot) && item.slot >= 0 && /^-?\d+$/.test(item.valueRaw))))
    || !reply.summaries || !Object.values(reply.summaries).every(isChartSummary)) throw new Error('The prepared tension reply is malformed.');
  return reply;
}
export function mergePreparedTension(a: PreparedTension, b: PreparedTension): PreparedTension {
  const preparedReads = [...a.reads, ...b.reads].filter(read => !tensionReadPreparing(read, 'historicalContributors'));
  const preparedAt = (first: number, last: number) => preparedReads.some(read => read.firstHour <= first && last <= read.lastHour);
  const countries = new Map<string, TensionReply['countries'][number]>();
  for (const reply of [a, b]) for (const country of reply.countries) {
    const previous = countries.get(countryKey(country));
    const runs = country.runs.filter(run => !reply.reads.some(read => tensionReadPreparing(read, 'historicalContributors')
      && read.firstHour <= run.firstHour && run.lastHour <= read.lastHour) || !preparedAt(run.firstHour, run.lastHour));
    const identityTag = countryKey(country);
    const identities = overlayCountryIdentities(
      [{ tag: identityTag, identities: previous?.identities ?? [] }],
      [{ tag: identityTag, identities: country.identities ?? [] }])[0].identities;
    countries.set(countryKey(country), { ...country,
      ...(country.identities !== undefined || previous?.identities !== undefined ? { identities } : {}),
      runs: [...new Map([...(previous?.runs ?? []), ...runs].map(run => [JSON.stringify(run), run])).values()].sort((x, y) => x.firstHour - y.firstHour) });
  }
  const catalogue = new Map<string, PreparedTension['catalogue'][number]>();
  for (const country of [...a.catalogue, ...b.catalogue]) {
    const previous = catalogue.get(countryKey(country));
    if (!previous || previous.key.startsWith('[') || !country.key.startsWith('[')) catalogue.set(countryKey(country), country);
  }
  const reads = [...new Map([...a.reads, ...b.reads].filter(read => !tensionReadPreparing(read, 'historicalContributors')
    || !preparedAt(read.firstHour, read.lastHour)).map(read => [JSON.stringify(read), read])).values()].sort((x, y) => x.firstHour - y.firstHour);
  const latest = (b.coverage?.complete || (b.coverage?.lastHour ?? -Infinity) > (a.coverage?.lastHour ?? -Infinity)) ? b : a;
  const complete = a.coverage?.complete || b.coverage?.complete;
  return { ...a, coverage: latest.coverage, events: latest.events, pending: complete ? [] : latest.pending,
    available: a.available || b.available, countries: [...countries.values()], catalogue: [...catalogue.values()],
    summaries: { ...a.summaries, ...b.summaries }, exactHours: [...new Set([...a.exactHours, ...b.exactHours])], reads,
    preparingContributors: [...new Map([...(a.preparingContributors ?? []), ...(b.preparingContributors ?? [])]
      .filter(item => !preparedAt(item.hour, item.hour)).map(item => [JSON.stringify([item.hour, item.slot]), item])).values()],
    preparing: complete ? [] : [...new Set([...(a.preparing ?? []), ...(b.preparing ?? [])])] };
}
