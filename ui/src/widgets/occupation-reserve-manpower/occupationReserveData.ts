import { countryPart, keyedKnown, keyedRead, runAtHour, validateKeyedWindow } from '../../dashboard/keyedHistory.ts';
import type { KeyedWindow } from '../../dashboard/keyedHistory';
import type { Run } from '../../charts/series';

export interface OccupationReserveReply {
  window: KeyedWindow; holders: readonly { tag: string; slot: number }[];
  /** Source countries by the ids their entries name. */
  sources: Readonly<Record<string, string>>; warnings: readonly string[];
}

export function validateOccupationReserve(value: unknown): OccupationReserveReply {
  const reply = value as OccupationReserveReply;
  if (!reply || typeof reply !== 'object' || !Array.isArray(reply.holders) || !reply.sources || typeof reply.sources !== 'object'
    || !reply.holders.every(holder => holder && typeof holder.tag === 'string' && Number.isSafeInteger(holder.slot))
    || !Object.values(reply.sources).every(source => typeof source === 'string' && countryPart(source) !== null)
    || !Array.isArray(reply.warnings) || !reply.warnings.every(warning => typeof warning === 'string'))
    throw new Error('The occupation reserve reply is malformed.');
  return { ...reply, window: validateKeyedWindow(reply.window) };
}

/** One recorded entry of a holder's reserve: its place in the holder's list, its source
 * country (null when the reference does not resolve) and its signed amount. */
export interface ReserveEntry { ordinal: number; source: string | null; amount: bigint }
export interface HolderReserve { holder: string; tag: string; entries: ReserveEntry[]; positive: bigint; negative: ReserveEntry[] }

/** Each holder's entries at `hour`, in recorded order; null when not recorded then. A holder
 * without entries at a known hour holds no reserve from occupied countries. */
export function reservesAt(reply: OccupationReserveReply, hour: number): { holders: HolderReserve[]; readHour: number | undefined } | null {
  if (!keyedKnown(reply.window, hour)) return null;
  const holders = new Map<string, Map<number, { source?: string | null; amount?: bigint }>>();
  for (const series of reply.window.series) {
    const [holder, ordinal, field] = series.name.split('|');
    const run = runAtHour(series.runs, hour);
    if (!run || (field !== 'amount' && field !== 'source')) continue;
    const entries = holders.get(holder) ?? new Map();
    const entry = entries.get(Number(ordinal)) ?? {};
    if (field === 'amount') entry.amount = BigInt(run[2]);
    else entry.source = reply.sources[run[2]] ?? null;
    entries.set(Number(ordinal), entry);
    holders.set(holder, entries);
  }
  const out: HolderReserve[] = [];
  for (const [holder, entries] of holders) {
    const list = [...entries].filter(([, entry]) => entry.amount !== undefined).sort(([a], [b]) => a - b)
      .map(([ordinal, entry]) => ({ ordinal, source: entry.source ?? null, amount: entry.amount! }));
    out.push({ holder, tag: countryPart(holder)?.tag ?? holder, entries: list,
      positive: list.reduce((sum, entry) => entry.amount > 0n ? sum + entry.amount : sum, 0n),
      negative: list.filter(entry => entry.amount < 0n) });
  }
  return { holders: out, readHour: keyedRead(reply.window, hour) };
}

/** A holder's positive entries summed by source country, largest first (repeated sources
 * add up on the bar; the tooltip keeps each entry). Unresolved sources stay apart. */
export function sourceParts(reserve: HolderReserve): { source: string | null; amount: bigint }[] {
  const by = new Map<string | null, bigint>();
  for (const entry of reserve.entries) if (entry.amount > 0n) by.set(entry.source, (by.get(entry.source) ?? 0n) + entry.amount);
  return [...by].map(([source, amount]) => ({ source, amount })).sort((a, b) => a.amount > b.amount ? -1 : a.amount < b.amount ? 1 : (a.source ?? '').localeCompare(b.source ?? ''));
}

/** The holder drill's bands: each resolved source country, then one band for positive amounts
 * whose source does not resolve at their hour, kept only where such amounts occur. Built from
 * the dated segments themselves, so an amount whose source covers only part of its run keeps
 * the rest in the unresolved band. */
export function holderSourceBands(reply: OccupationReserveReply, holder: string): { sources: (string | null)[]; bands: Run[][] } {
  const resolved = [...new Set(Object.values(reply.sources))];
  const bands = holderBands(reply, holder, [...resolved, null]);
  return bands.at(-1)!.length ? { sources: [...resolved, null], bands } : { sources: resolved, bands: bands.slice(0, -1) };
}

/** One holder's positive reserve from each source over the window, as stacked chart runs
 * (one per source, in the order of `sources`). */
export function holderBands(reply: OccupationReserveReply, holder: string, sources: readonly (string | null)[]): Run[][] {
  const hours = new Set<number>();
  for (const series of reply.window.series) if (series.name.startsWith(`${holder}|`)) for (const run of series.runs) { hours.add(run[0]); hours.add(run[1] + 1); }
  const stops = [...hours].sort((a, b) => a - b);
  const bands = sources.map(() => [] as Run[]);
  for (let index = 0; index + 1 < stops.length; index++) {
    const first = stops[index], last = stops[index + 1] - 1;
    const at = reservesAt(reply, first)?.holders.find(item => item.holder === holder);
    if (!at) continue;
    const parts = new Map(sourceParts(at).map(part => [part.source, part.amount]));
    sources.forEach((source, band) => {
      const amount = parts.get(source);
      if (amount === undefined) return;
      const value = Number(amount), runs = bands[band], previous = runs.at(-1);
      if (previous && previous.lastHour + 1 === first && previous.value === value) previous.lastHour = last;
      else runs.push({ firstHour: first, lastHour: last, value, breakBefore: !previous || previous.lastHour + 1 !== first });
    });
  }
  return bands;
}

export const persons = (value: bigint) => value.toLocaleString('en-US');
