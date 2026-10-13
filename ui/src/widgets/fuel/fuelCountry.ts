import { clipToSpans, runAt } from '../../charts/series.ts';
import type { Span } from '../../charts/series';
import { countryDrillTagAt } from '../../dashboard/countryIdentity.ts';
import type { FuelReply } from '../../generated/dashboard';
import { fuelPartHistory } from './fuelData.ts';

export { datedCountryToken as fuelCountryToken, readDatedCountry as readFuelCountry, datedCountrySpans as fuelCountrySpans } from '../../dashboard/datedCountry.ts';
export type { DatedCountrySelection as FuelCountrySelection } from '../../dashboard/datedCountry';

/** Keep supplying reads intact; clipping belongs to plotting runs, not read dates. */
export function fuelCountryRead(replies: readonly FuelReply[], spans: ReadonlyMap<string, readonly Span[]> | undefined, hour: number) {
  const tag = spans ? countryDrillTagAt(spans, hour) : replies[0]?.tag;
  const reply = replies.find(reply => reply.tag === tag), read = reply && runAt(reply.reads, hour);
  return reply && read ? { reply, read } : null;
}

export function fuelCountryHistory(replies: readonly FuelReply[], spans: ReadonlyMap<string, readonly Span[]> | undefined, measure: 'gain' | 'burn', digits?: number) {
  const parts = new Map<string, ReturnType<typeof fuelPartHistory>[number]>();
  for (const reply of replies) for (const part of fuelPartHistory(reply, measure, reply, digits)) {
    const runs = spans ? clipToSpans(part.runs, spans.get(reply.tag) ?? []) : part.runs;
    const previous = parts.get(part.id);
    if (previous) previous.runs.push(...runs); else parts.set(part.id, { ...part, runs: [...runs] });
  }
  return [...parts.values()].map(part => ({ ...part, runs: part.runs.sort((a, b) => a.firstHour - b.firstHour) }));
}
