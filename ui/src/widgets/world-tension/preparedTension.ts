import type { TensionReply } from '../../generated/dashboard';
import type { ChartSummary } from '../../charts/chartSummary';
import { isChartSummary } from '../../charts/chartSummary.ts';
import { countryKey, validateTensionReply } from './tensionData.ts';
import { mergeReachedRows } from '../../dashboard/preparedSeries.ts';

export interface PreparedTension extends TensionReply {
  available: boolean;
  catalogue: { key: string; tag: string; slot: number; firstHour: number }[];
  summaries: Record<string, ChartSummary>;
  exactHours: number[];
}
export function validatePreparedTension(value: unknown): PreparedTension {
  const reply = validateTensionReply(value) as PreparedTension;
  if (typeof reply.available !== 'boolean' || !Array.isArray(reply.catalogue) || !reply.catalogue.every(item => item && typeof item.tag === 'string'
    && /^(0|[1-9]\d*)$/.test(item.key) && Number.isSafeInteger(item.slot) && item.slot >= 0 && Number.isSafeInteger(item.firstHour))
    || !Array.isArray(reply.exactHours) || !reply.exactHours.every(Number.isSafeInteger)
    || !reply.summaries || !Object.values(reply.summaries).every(isChartSummary)) throw new Error('The prepared tension reply is malformed.');
  return reply;
}
export const mergePreparedTension = (a:PreparedTension,b:PreparedTension): PreparedTension => ({...a,
  countries:mergeReachedRows(a.countries,b.countries,countryKey),summaries:{...a.summaries,...b.summaries},exactHours:[...new Set([...a.exactHours,...b.exactHours])],
  reads:[...new Map([...a.reads,...b.reads].map(read=>[JSON.stringify(read),read])).values()].sort((a,b)=>a.firstHour-b.firstHour)});
