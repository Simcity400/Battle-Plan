import type { CountryFactsReply, CountryStyle } from '../generated/dashboard';
import type { DashboardClient, DashboardSource } from './client';
import type { FactSource } from './countryFacts';
import { loadTransientProduct } from './products.ts';
import { finalPreparedReply } from './preparedPrefix.ts';

/** A country's style at an hour, with what its `country-styles` reply would have warned. */
export type HeldStyle = CountryStyle & { warnings: readonly string[] };

type Run = readonly [number, number, number, number];
interface Timeline { reply: CountryFactsReply; runs: ReadonlyMap<string, readonly Run[]>; final: boolean }
interface Entry { content: number | null; timeline?: Timeline; controller?: AbortController; askAt?: number }

/** How long a recording whose facts could not be read reads them hour by hour before asking again. */
const RETRY_MS = 8000;
/** While preparation covers more, how often an hour the timeline does not hold asks for it again. */
const GROWING_MS = 2000;

/** Each recording and branch's timeline for the content it was read under, per client. */
const held = new WeakMap<DashboardClient, Map<string, Entry>>();

const integer = (value: unknown): value is number => Number.isInteger(value);
const texts = (value: unknown): value is string[] => Array.isArray(value) && value.every(text => typeof text === 'string');

/** A `country-facts` reply, or a refusal naming what is malformed. */
export function validateCountryFacts(data: unknown): CountryFactsReply {
  const reply = data as Partial<CountryFactsReply> | null;
  const ok = !!reply && typeof reply === 'object' && typeof reply.branchId === 'string'
    && Array.isArray(reply.known) && reply.known.every(span => Array.isArray(span) && span.length === 2 && span.every(integer) && span[0] <= span[1])
    && texts(reply.names) && texts(reply.reasons) && texts(reply.warnings)
    && Array.isArray(reply.countries) && reply.countries.every(country => !!country && typeof country.tag === 'string'
      && Array.isArray(country.runs) && country.runs.every((run, index) => Array.isArray(run) && run.length === 4 && run.every(integer)
        && run[1] < reply.names!.length && run[3] < reply.reasons!.length && (index === 0 || run[0] > country.runs[index - 1][0])));
  if (!ok) throw new Error('Country facts reply is malformed');
  return reply as CountryFactsReply;
}

/** The timeline of `source` (every name and colour preparation holds, as runs), read once and
 * held, or undefined while it is read or when it cannot be. While preparation covers more, an
 * hour it does not hold (`wanted`) reads it again, at most every [`GROWING_MS`]. */
function timeline(client: DashboardClient, source: FactSource, recording: DashboardSource, wanted: number): Timeline | undefined {
  let entries = held.get(client);
  if (!entries) held.set(client, entries = new Map());
  const key = JSON.stringify([source.sourceId, source.revision, source.branchId]);
  const content = source.content ?? null;
  let entry = entries.get(key);
  // Content read anew replaces what the old content gave, and its read.
  if (entry && entry.content !== content) { entry.controller?.abort(); entry = undefined; }
  entry ??= { content };
  const current = entry.timeline;
  if (entry.controller || (entry.askAt ?? 0) > Date.now() || current && (current.final || holds(current.reply.known, wanted))) {
    entries.set(key, entry);
    return current;
  }
  const controller = new AbortController();
  entries.set(key, { content, timeline: current, controller });
  void loadTransientProduct<unknown>(client, { sourceId: source.sourceId, revision: source.revision, product: 'country-facts',
    params: { branchId: source.branchId } }, controller.signal, recording).then(data => {
    const reply = validateCountryFacts(data);
    if (entries.get(key)?.controller === controller) entries.set(key, { content, askAt: Date.now() + GROWING_MS,
      timeline: { reply, runs: new Map(reply.countries.map(country => [country.tag, country.runs as Run[]])), final: finalPreparedReply(data) } });
  }).catch(() => {
    if (entries.get(key)?.controller === controller) entries.set(key, { content, timeline: current, askAt: Date.now() + RETRY_MS });
  });
  return current;
}

/** Whether the timeline holds `hour`. */
function holds(known: readonly (readonly number[])[], hour: number): boolean {
  let low = 0, high = known.length;
  while (low < high) { const middle = (low + high) >> 1; if (known[middle][1] < hour) low = middle + 1; else high = middle; }
  return low < known.length && known[low][0] <= hour;
}

/** The run of `runs` in force at `hour`, or undefined before the first. */
function runAt(runs: readonly Run[], hour: number): Run | undefined {
  let low = 0, high = runs.length;
  while (low < high) { const middle = (low + high) >> 1; if (runs[middle][0] <= hour) low = middle + 1; else high = middle; }
  return runs[low - 1];
}

/** Each of `tags`' style at `hour` from the held timeline, as `country-styles` answers it, or
 * undefined when the timeline does not hold the hour (the hour is asked for instead). */
export function heldStyles(client: DashboardClient, source: FactSource, recording: DashboardSource | undefined, hour: number,
  tags: readonly string[]): ReadonlyMap<string, HeldStyle> | undefined {
  const facts = recording && timeline(client, source, recording, hour);
  if (!facts || !holds(facts.reply.known, hour)) return undefined;
  const { names, reasons } = facts.reply;
  const warnings = [...facts.reply.warnings];
  const styles = tags.map(tag => {
    const run = runAt(facts.runs.get(tag) ?? [], hour);
    if (!run) warnings.push(`${tag}: not a country at this hour`);
    else if (run[3] >= 0) warnings.push(`${tag}: ${reasons[run[3]]}`);
    const colour = run?.[2] ?? -1;
    return { tag, name: run && run[1] >= 0 ? names[run[1]] : null,
      color: colour >= 0 ? [(colour >> 16) & 255, (colour >> 8) & 255, colour & 255] as [number, number, number] : null };
  });
  return new Map(styles.map(style => [style.tag, { ...style, warnings }]));
}

/** Forgets a recording's timelines, and stops their reads; with `settled`, only those no read
 * is still answering, so they are read again. */
export function forgetTimelines(client: DashboardClient, sourceId: string, settled = false): void {
  const entries = held.get(client);
  for (const [key, entry] of entries ?? []) {
    if ((JSON.parse(key) as unknown[])[0] !== sourceId || settled && entry.controller) continue;
    entry.controller?.abort();
    entries!.delete(key);
  }
}

/** A fact spec's `held` lookup: `pick`'s value of each style held at an hour, leaving out a
 * country it gives none for, as the spec's `extract` leaves it out of a reply. */
export const heldFacts = <V>(pick: (style: HeldStyle) => V | null | undefined) =>
  (client: DashboardClient, source: FactSource, recording: DashboardSource | undefined, hour: number, tags: readonly string[]): ReadonlyMap<string, V> | undefined => {
    const styles = heldStyles(client, source, recording, hour, tags);
    if (!styles) return undefined;
    const found = new Map<string, V>();
    for (const [tag, style] of styles) { const value = pick(style); if (value != null) found.set(tag, value); }
    return found;
  };
