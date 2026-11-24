import { clipToSpans, runAt } from '../charts/series.ts';
import type { Span } from '../charts/series';
import { airCauseName, ledgerRuns, ledgerScale, ledgerTotals } from './militaryAirLedger.ts';
import type { LedgerIndex } from './militaryAirLedger';
import type { MilitaryAirPart } from '../generated/dashboard';

export function airLossDrillActors(index: LedgerIndex, hour: number, tags: readonly string[]): string[] {
  if (!runAt(index.reply.observed, hour)) return [];
  return [...new Set(tags)].flatMap(tag => {
    const series = index.byTag.get(tag)?.find(series => runAt(series.country.present, hour));
    return series ? [JSON.stringify([tag, series.country.identity])] : [];
  });
}

export function airLossDrillActor(key: string): { tag: string; identity: string } {
  const [tag, identity] = JSON.parse(key) as [string, string];
  return { tag, identity };
}

export function airLossDrillHistory(index: LedgerIndex, identity: string, from: number, end: number, dealt: boolean) {
  const aliases = index.series.filter(series => series.country.identity === identity);
  if (!aliases.length) return [];
  const present: Span[] = [];
  for (const span of clipToSpans([{ firstHour: from, lastHour: end }], aliases.flatMap(series => series.country.present))) {
    const previous = present.at(-1);
    if (previous && previous.lastHour + 1 === span.firstHour) previous.lastHour = span.lastHour;
    else present.push({ ...span });
  }
  const country = { ...aliases[0].country, present };
  const causes = new Map<number, MilitaryAirPart[]>();
  for (const part of aliases[0].parts) {
    if (part.cause === null) continue;
    const parts = causes.get(part.cause) ?? [];
    parts.push(part); causes.set(part.cause, parts);
  }
  return [...causes].sort(([a], [b]) => a - b).map(([cause, parts]) => ({ id: String(cause), cause, label: airCauseName(cause), parts,
    runs: ledgerRuns({ country, parts, points: ledgerTotals(parts), openings: [] }, from, end, dealt, ledgerScale('airLosses')) }));
}
