import { countryPart, keyedKnown, keyedRead, runAtHour, validateKeyedWindow } from '../../dashboard/keyedHistory.ts';
import type { KeyedWindow } from '../../dashboard/keyedHistory';
import type { HolderAt } from '../../dashboard/holderLooks';
import type { Run, Span } from '../../charts/series';

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
/** `admitted`: the holder's occupation status was read with its entries, so a source
 * without an entry holds none. */
export interface HolderReserve { holder: string; tag: string; admitted: boolean; entries: ReserveEntry[]; positive: bigint; negative: ReserveEntry[] }

/** Each holder's entries at `hour`, in recorded order; null when not recorded then. An
 * admitted holder without entries holds no reserve. */
export function reservesAt(reply: OccupationReserveReply, hour: number): { holders: HolderReserve[]; readHour: number | undefined } | null {
  if (!keyedKnown(reply.window, hour)) return null;
  const holders = new Map<string, Map<number, { source?: string | null; amount?: bigint }>>();
  const admitted = new Set<string>();
  for (const series of reply.window.series) {
    const [holder, ordinal, field] = series.name.split('|');
    const run = runAtHour(series.runs, hour);
    if (run && ordinal === 'holder' && field === undefined) { admitted.add(holder); continue; }
    if (!run || (field !== 'amount' && field !== 'source')) continue;
    const entries = holders.get(holder) ?? new Map();
    const entry = entries.get(Number(ordinal)) ?? {};
    if (field === 'amount') entry.amount = BigInt(run[2]);
    else entry.source = reply.sources[run[2]] ?? null;
    entries.set(Number(ordinal), entry);
    holders.set(holder, entries);
  }
  for (const holder of admitted) if (!holders.has(holder)) holders.set(holder, new Map());
  const out: HolderReserve[] = [];
  for (const [holder, entries] of holders) {
    const list = [...entries].filter(([, entry]) => entry.amount !== undefined).sort(([a], [b]) => a - b)
      .map(([ordinal, entry]) => ({ ordinal, source: entry.source ?? null, amount: entry.amount! }));
    out.push({ holder, tag: countryPart(holder)?.tag ?? holder, admitted: admitted.has(holder), entries: list,
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
  return bands.at(-1)!.some(run => run.value > 0) ? { sources: [...resolved, null], bands } : { sources: resolved, bands: bands.slice(0, -1) };
}

/** One holder's positive reserve from each source over the window, as stacked chart runs
 * (one per source, in the order of `sources`); 0 where an admitted holder has no entry from it. */
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
      const amount = parts.get(source) ?? (at.admitted ? 0n : undefined);
      if (amount === undefined) return;
      const value = Number(amount), runs = bands[band], previous = runs.at(-1);
      if (previous && previous.lastHour + 1 === first && previous.value === value) previous.lastHour = last;
      else runs.push({ firstHour: first, lastHour: last, value, breakBefore: !previous || previous.lastHour + 1 !== first });
    });
  }
  return bands;
}

/** Each of `holder`'s source countries at an hour one of its entries names it, so the tag is
 * proven its own then: `sample` (the drill's look hour) while named then, else its latest
 * naming up to `hour`, else its first. A source's one colour and legend name are read there. */
export function sourceWitnesses(reply: OccupationReserveReply, holder: string, hour: number, sample: number): HolderAt[] {
  const named = new Map<string, Span[]>();
  for (const series of reply.window.series) {
    const [owner, , field] = series.name.split('|');
    if (owner !== holder || field !== 'source') continue;
    for (const run of series.runs) {
      const source = reply.sources[run[2]];
      if (source) named.set(source, [...named.get(source) ?? [], { firstHour: run[0], lastHour: run[1] }]);
    }
  }
  return [...named].map(([source, spans]) => {
    const latest = spans.filter(span => span.firstHour <= hour).sort((a, b) => b.lastHour - a.lastHour)[0];
    const at = spans.some(span => span.firstHour <= sample && sample <= span.lastHour) ? sample
      : latest ? latest.lastHour >= hour ? latest.firstHour : latest.lastHour
      : Math.min(...spans.map(span => span.firstHour));
    return { key: source, tag: countryPart(source)!.tag, hour: at };
  });
}

export const persons = (value: bigint) => value.toLocaleString('en-US');
