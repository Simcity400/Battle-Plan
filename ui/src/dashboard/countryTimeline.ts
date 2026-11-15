import { asFailure, type Failure } from './queryError.ts';
import type { CountryFactsReply, CountryFlagChoice, CountryStyle } from '../generated/dashboard';
import type { DashboardClient, DashboardSource } from './client';
import type { FactSource } from './countryFacts';
import { loadTransientProduct } from './products.ts';
import { PerClient } from './prepare.ts';

/** A country's style at an hour, with the warnings that concern it: the recording's and its own. */
export type HeldStyle = CountryStyle & { warnings: readonly string[] };

type Run = readonly [number, number, number, number, number];
/** A recording branch's country timeline: every name, colour and flag choice preparation holds, as runs. */
export interface CountryTimeline {
  reply: CountryFactsReply; runs: ReadonlyMap<string, readonly Run[]>; final: boolean;
  /** One colour per recorded map colour, shared by every country and hour drawn in it; kept as the timeline grows and released with it. */
  colours: Map<number, { color: [number, number, number] }>;
}
interface Entry { content: number | null; timeline?: CountryTimeline; controller?: AbortController; askAt?: number; error?: Failure }

/** A spec's values for `tags` at `hour` from a timeline, or undefined when it does not hold the hour. */
export type HeldLookup<V> = (timeline: CountryTimeline, hour: number, tags: readonly string[]) => ReadonlyMap<string, V> | undefined;

/** How long a recording whose facts could not be read reads them hour by hour before asking again. */
const RETRY_MS = 8000;
/** While preparation covers more, how often an hour the timeline does not hold asks for it again. */
const GROWING_MS = 2000;

/** Each recording and branch's timeline for the content it was read under, per client. */
const held = new PerClient<Map<string, Entry>>();
/** Who is told when a recording and branch's timeline changes, per client. */
const watchers = new PerClient<Map<string, Set<() => void>>>();

const timelineKey = (source: Pick<FactSource, 'sourceId' | 'revision' | 'branchId'>) => JSON.stringify([source.sourceId, source.revision, source.branchId]);

/** Sets `key`'s entry, telling its watchers when the timeline it holds changes. */
function setEntry(client: DashboardClient, entries: Map<string, Entry>, key: string, entry: Entry | undefined): void {
  const before = entries.get(key);
  if (entry) entries.set(key, entry); else entries.delete(key);
  // A read that settles tells its watchers even when it keeps the timeline: one waiting for an
  // hour asks again once the read's retry may.
  const settled = !!before?.controller && !entry?.controller;
  if (before?.timeline !== entry?.timeline || before?.error !== entry?.error || settled) for (const changed of [...watchers.get(client)?.get(key) ?? []]) changed();
}

const integer = (value: unknown): value is number => Number.isInteger(value);
const texts = (value: unknown): value is string[] => Array.isArray(value) && value.every(text => typeof text === 'string');
/** One entry per flag atlas (normal, medium, small), each a text or null. */
const atlases = (value: unknown) => Array.isArray(value) && value.length === 3 && value.every(text => text === null || typeof text === 'string');

/** A `country-facts` reply, or a refusal naming what is malformed. */
export function validateCountryFacts(data: unknown): CountryFactsReply {
  const reply = data as Partial<CountryFactsReply> | null;
  const ok = !!reply && typeof reply === 'object' && typeof reply.branchId === 'string'
    && Array.isArray(reply.known) && reply.known.every(span => Array.isArray(span) && span.length === 2 && span.every(integer) && span[0] <= span[1])
    && texts(reply.names) && texts(reply.reasons) && texts(reply.warnings)
    && Array.isArray(reply.flags) && reply.flags.every(choice => !!choice && typeof choice === 'object' && atlases(choice.keys) && atlases(choice.reasons))
    && Array.isArray(reply.countries) && reply.countries.every(country => !!country && typeof country.tag === 'string'
      && Array.isArray(country.runs) && country.runs.every((run, index) => Array.isArray(run) && run.length === 5 && run.every(integer)
        && run[1] < reply.names!.length && run[3] < reply.reasons!.length && run[4] < reply.flags!.length && (index === 0 || run[0] > country.runs[index - 1][0])));
  if (!ok) throw new Error('Country facts reply is malformed');
  return reply as CountryFactsReply;
}

/** The timeline held for `source`'s content, as read so far, without asking for it. */
export function heldTimeline(client: DashboardClient, source: FactSource): CountryTimeline | undefined {
  const entry = held.get(client)?.get(timelineKey(source));
  return entry && entry.content === (source.content ?? null) ? entry.timeline : undefined;
}

/** Tells `changed` whenever the timeline held for `source`'s recording and branch changes. */
export function watchTimeline(client: DashboardClient, source: Pick<FactSource, 'sourceId' | 'revision' | 'branchId'>, changed: () => void): () => void {
  let byKey = watchers.get(client);
  if (!byKey) watchers.set(client, byKey = new Map());
  const key = timelineKey(source);
  const set = byKey.get(key) ?? new Set<() => void>();
  byKey.set(key, set);
  set.add(changed);
  return () => { set.delete(changed); if (!set.size && byKey.get(key) === set) byKey.delete(key); };
}

/** The timeline of `source` (every name and colour preparation holds, as runs), read once and
 * held, or undefined while it is read or when it cannot be. While preparation covers more, an
 * hour it does not hold (`wanted`) reads it again, at most every [`GROWING_MS`]. */
export function askTimeline(client: DashboardClient, source: FactSource, recording: DashboardSource, wanted: number): CountryTimeline | undefined {
  let entries = held.get(client);
  if (!entries) held.set(client, entries = new Map());
  const key = timelineKey(source);
  const content = source.content ?? null;
  let entry = entries.get(key);
  // Content read anew replaces what the old content gave, and its read.
  if (entry && entry.content !== content) { entry.controller?.abort(); entry = undefined; }
  entry ??= { content };
  const current = entry.timeline;
  if (entry.controller || (entry.askAt ?? 0) > Date.now() || current && (current.final || holds(current.reply.known, wanted))) {
    setEntry(client, entries, key, entry);
    return current;
  }
  const controller = new AbortController();
  setEntry(client, entries, key, { content, timeline: current, controller });
  void loadTransientProduct<unknown>(client, { sourceId: source.sourceId, revision: source.revision, product: 'country-facts',
    params: { branchId: source.branchId } }, controller.signal, recording).then(data => {
    const reply = validateCountryFacts(data);
    if (entries.get(key)?.controller === controller) setEntry(client, entries, key, { content, askAt: Date.now() + GROWING_MS,
      timeline: { reply, runs: new Map(reply.countries.map(country => [country.tag, country.runs as Run[]])), final: true,
        colours: current?.colours ?? new Map() } });
  }).catch(error => {
    if (entries.get(key)?.controller === controller) setEntry(client, entries, key, { content, timeline: current, error: asFailure(error), askAt: Date.now() + RETRY_MS });
  });
  return current;
}

/** When `source`'s held timeline may be read again for an hour it does not hold, or undefined
 * while a read of it answers or nothing holds a read back. */
export function askableAt(client: DashboardClient, source: FactSource): number | undefined {
  const entry = held.get(client)?.get(timelineKey(source));
  return entry && !entry.controller ? entry.askAt : undefined;
}

export function timelineFailure(client: DashboardClient, source: FactSource): Failure | undefined {
  const entry = held.get(client)?.get(timelineKey(source));
  return entry?.content === (source.content ?? null) ? entry.error : undefined;
}

export function retryTimelines(client: DashboardClient, sourceId: string): void {
  const entries = held.get(client);
  if (!entries) return;
  for (const [key, entry] of entries) {
    if ((JSON.parse(key) as string[])[0] === sourceId && entry.error) {
      setEntry(client, entries, key, { ...entry, error: undefined, askAt: undefined });
    }
  }
}

/** The span of `known` holding `hour`, or undefined. */
function spanAt(known: readonly (readonly number[])[], hour: number): readonly number[] | undefined {
  let low = 0, high = known.length;
  while (low < high) { const middle = (low + high) >> 1; if (known[middle][1] < hour) low = middle + 1; else high = middle; }
  return low < known.length && known[low][0] <= hour ? known[low] : undefined;
}

/** Whether the timeline holds `hour`. */
const holds = (known: readonly (readonly number[])[], hour: number) => spanAt(known, hour) !== undefined;

/** The last hour at or before `hour` at which one of `tags`' name, colour, reason or flag in `facts`
 * changed, so each reads the same there as at `hour`; undefined when `facts` does not hold `hour`. */
export function identitySince(facts: CountryTimeline, hour: number, tags: readonly string[]): number | undefined {
  const span = spanAt(facts.reply.known, hour);
  if (!span) return undefined;
  let since = span[0];
  for (const tag of tags) {
    const run = runAt(facts.runs.get(tag) ?? [], hour);
    if (run && run[0] > since) since = run[0];
  }
  return since;
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
  const facts = recording && askTimeline(client, source, recording, hour);
  return facts && stylesIn(facts, hour, tags);
}

/** Each of `tags`' style at `hour` in `facts`, as `country-styles` answers it, or undefined when
 * `facts` does not hold the hour. Each carries the recording's warnings and only its own reason,
 * so a style kept while it is shown never reports a country no longer asked about. */
export function stylesIn(facts: CountryTimeline, hour: number, tags: readonly string[]): ReadonlyMap<string, HeldStyle> | undefined {
  if (!holds(facts.reply.known, hour)) return undefined;
  const { names, reasons, warnings } = facts.reply;
  return new Map(tags.map(tag => {
    const run = runAt(facts.runs.get(tag) ?? [], hour);
    const reason = !run ? `${tag}: not a country at this hour` : run[3] >= 0 ? `${tag}: ${reasons[run[3]]}` : null;
    const colour = run?.[2] ?? -1;
    return [tag, { tag, name: run && run[1] >= 0 ? names[run[1]] : null,
      color: colour >= 0 ? [(colour >> 16) & 255, (colour >> 8) & 255, colour & 255] as [number, number, number] : null,
      warnings: reason === null ? warnings : [...warnings, reason] }] as const;
  }));
}

/** Each of `tags`' recorded map colour at `hour` in `facts`, as `0xRRGGBB` (-1 when it has none), or
 * undefined when `facts` does not hold the hour. */
export function coloursIn(facts: CountryTimeline, hour: number, tags: readonly string[]): ReadonlyMap<string, number> | undefined {
  if (!holds(facts.reply.known, hour)) return undefined;
  return new Map(tags.map(tag => [tag, runAt(facts.runs.get(tag) ?? [], hour)?.[2] ?? -1] as const));
}

/** The one colour `facts` keeps for the recorded map colour `packed` (`0xRRGGBB`). */
export function colourOf(facts: CountryTimeline, packed: number): { color: [number, number, number] } {
  let colour = facts.colours.get(packed);
  if (!colour) facts.colours.set(packed, colour = { color: [(packed >> 16) & 255, (packed >> 8) & 255, packed & 255] });
  return colour;
}

/** `tag`'s flag at `hour` in `facts`: the choice preparation made, or why there is none (a tag
 * that is not a country, or an hour a finished preparation holds no facts of), or undefined when
 * `facts` does not hold the hour yet. */
export function flagIn(facts: CountryTimeline, hour: number, tag: string): { choice: CountryFlagChoice } | { reason: string } | undefined {
  if (!holds(facts.reply.known, hour)) return facts.final ? { reason: 'country facts were not recorded at this hour' } : undefined;
  const run = runAt(facts.runs.get(tag) ?? [], hour);
  if (run && run[4] >= 0) return { choice: facts.reply.flags[run[4]] };
  return { reason: run && run[3] >= 0 ? facts.reply.reasons[run[3]] : 'not a country at this hour' };
}

/** Forgets a recording's timelines, and stops their reads; with `settled`, only those no read
 * is still answering, so they are read again. */
export function forgetTimelines(client: DashboardClient, sourceId: string, settled = false): void {
  const entries = held.get(client);
  for (const [key, entry] of entries ?? []) {
    if ((JSON.parse(key) as unknown[])[0] !== sourceId || settled && entry.controller) continue;
    entry.controller?.abort();
    setEntry(client, entries!, key, undefined);
  }
}

/** A fact spec's `held` lookup: `pick`'s value of each style held at an hour, leaving out a
 * country it gives none for, as the spec's `extract` leaves it out of a reply. */
export const heldFacts = <V>(pick: (style: HeldStyle) => V | null | undefined): HeldLookup<V> => (timeline, hour, tags) => {
  const styles = stylesIn(timeline, hour, tags);
  if (!styles) return undefined;
  const found = new Map<string, V>();
  for (const [tag, style] of styles) { const value = pick(style); if (value != null) found.set(tag, value); }
  return found;
};
