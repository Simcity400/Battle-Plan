import { asFailure, type Failure } from './queryError.ts';
import type { CountryFactsReply, CountryFlagChoice, CountryStyle } from '../generated/dashboard';
import type { DashboardClient, DashboardSource } from './client';
import type { FactSource } from './countryFacts';
import { loadTransientProduct } from './products.ts';
import { PerClient } from './prepare.ts';
import { productRequest } from './request.ts';

export type HeldStyle = CountryStyle & { warnings: readonly string[] };

type Run = readonly [number, number, number, number, number];
export interface CountryTimeline {
  reply: CountryFactsReply; runs: ReadonlyMap<string, readonly Run[]>; final: boolean;
  colours: Map<number, { color: [number, number, number] }>;
}
interface Entry { content: number | null; timeline?: CountryTimeline; controller?: AbortController; askAt?: number; error?: Failure }

export type HeldLookup<V> = (timeline: CountryTimeline, hour: number, tags: readonly string[]) => ReadonlyMap<string, V> | undefined;

const RETRY_MS = 8000;
const GROWING_MS = 2000;

const held = new PerClient<Map<string, Entry>>();
const watchers = new PerClient<Map<string, Set<() => void>>>();

const timelineKey = (source: Pick<FactSource, 'sourceId' | 'revision' | 'branchId'>) => JSON.stringify([source.sourceId, source.revision, source.branchId]);

function setEntry(client: DashboardClient, entries: Map<string, Entry>, key: string, entry: Entry | undefined): void {
  const before = entries.get(key);
  if (entry) entries.set(key, entry); else entries.delete(key);
  const settled = !!before?.controller && !entry?.controller;
  if (before?.timeline !== entry?.timeline || before?.error !== entry?.error || settled) for (const changed of [...watchers.get(client)?.get(key) ?? []]) changed();
}

const integer = (value: unknown): value is number => Number.isInteger(value);
const texts = (value: unknown): value is string[] => Array.isArray(value) && value.every(text => typeof text === 'string');
const atlases = (value: unknown) => Array.isArray(value) && value.length === 3 && value.every(text => text === null || typeof text === 'string');

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

export function heldTimeline(client: DashboardClient, source: FactSource): CountryTimeline | undefined {
  const entry = held.get(client)?.get(timelineKey(source));
  return entry && entry.content === (source.content ?? null) ? entry.timeline : undefined;
}

export function watchTimeline(client: DashboardClient, source: Pick<FactSource, 'sourceId' | 'revision' | 'branchId'>, changed: () => void): () => void {
  let byKey = watchers.get(client);
  if (!byKey) watchers.set(client, byKey = new Map());
  const key = timelineKey(source);
  const set = byKey.get(key) ?? new Set<() => void>();
  byKey.set(key, set);
  set.add(changed);
  return () => { set.delete(changed); if (!set.size && byKey.get(key) === set) byKey.delete(key); };
}

export function askTimeline(client: DashboardClient, source: FactSource, recording: DashboardSource, wanted: number): CountryTimeline | undefined {
  let entries = held.get(client);
  if (!entries) held.set(client, entries = new Map());
  const key = timelineKey(source);
  const content = source.content ?? null;
  let entry = entries.get(key);
  if (entry && entry.content !== content) { entry.controller?.abort(); entry = undefined; }
  entry ??= { content };
  const current = entry.timeline;
  if (entry.controller || (entry.askAt ?? 0) > Date.now() || current && (current.final || holds(current.reply.known, wanted))) {
    setEntry(client, entries, key, entry);
    return current;
  }
  const controller = new AbortController();
  setEntry(client, entries, key, { content, timeline: current, controller });
  void loadTransientProduct<unknown>(client, productRequest({ sourceId: source.sourceId, revision: source.revision, product: 'country-facts',
    params: { branchId: source.branchId } }), controller.signal, recording).then(data => {
    const reply = validateCountryFacts(data);
    if (entries.get(key)?.controller === controller) setEntry(client, entries, key, { content, askAt: Date.now() + GROWING_MS,
      timeline: { reply, runs: new Map(reply.countries.map(country => [country.tag, country.runs as Run[]])), final: true,
        colours: current?.colours ?? new Map() } });
  }).catch(error => {
    if (entries.get(key)?.controller === controller) setEntry(client, entries, key, { content, timeline: current, error: asFailure(error), askAt: Date.now() + RETRY_MS });
  });
  return current;
}

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

function spanAt(known: readonly (readonly number[])[], hour: number): readonly number[] | undefined {
  let low = 0, high = known.length;
  while (low < high) { const middle = (low + high) >> 1; if (known[middle][1] < hour) low = middle + 1; else high = middle; }
  return low < known.length && known[low][0] <= hour ? known[low] : undefined;
}

const holds = (known: readonly (readonly number[])[], hour: number) => spanAt(known, hour) !== undefined;

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

export function lookStarts(timeline: CountryTimeline | undefined, tag: string, first: number, at: number): [number, boolean][] {
  const runs = timeline?.runs.get(tag);
  if (!timeline || !runs) return [[at, true], [at + 1, false]];
  const known = timeline.reply.known, hours = new Set([first, ...runs.map(run => run[0]).filter(hour => hour > first)]);
  for (const [from, to] of known) for (const edge of [from, to + 1]) if (edge > first) hours.add(edge);
  return [...hours].sort((a, b) => a - b).map(hour => [hour, holds(known, hour)]);
}

function runAt(runs: readonly Run[], hour: number): Run | undefined {
  let low = 0, high = runs.length;
  while (low < high) { const middle = (low + high) >> 1; if (runs[middle][0] <= hour) low = middle + 1; else high = middle; }
  return runs[low - 1];
}

export function heldStyles(client: DashboardClient, source: FactSource, recording: DashboardSource | undefined, hour: number,
  tags: readonly string[]): ReadonlyMap<string, HeldStyle> | undefined {
  const facts = recording && askTimeline(client, source, recording, hour);
  return facts && stylesIn(facts, hour, tags);
}

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

export function coloursIn(facts: CountryTimeline, hour: number, tags: readonly string[]): ReadonlyMap<string, number> | undefined {
  if (!holds(facts.reply.known, hour)) return undefined;
  return new Map(tags.map(tag => [tag, runAt(facts.runs.get(tag) ?? [], hour)?.[2] ?? -1] as const));
}

export function colourOf(facts: CountryTimeline, packed: number): { color: [number, number, number] } {
  let colour = facts.colours.get(packed);
  if (!colour) facts.colours.set(packed, colour = { color: [(packed >> 16) & 255, (packed >> 8) & 255, packed & 255] });
  return colour;
}

export function flagIn(facts: CountryTimeline, hour: number, tag: string): { choice: CountryFlagChoice } | { reason: string } | undefined {
  if (!holds(facts.reply.known, hour)) return facts.final ? { reason: 'country facts were not recorded at this hour' } : undefined;
  const run = runAt(facts.runs.get(tag) ?? [], hour);
  if (run && run[4] >= 0) return { choice: facts.reply.flags[run[4]] };
  return { reason: run && run[3] >= 0 ? facts.reply.reasons[run[3]] : 'not a country at this hour' };
}

export function forgetTimelines(client: DashboardClient, sourceId: string, settled = false): void {
  const entries = held.get(client);
  for (const [key, entry] of entries ?? []) {
    if ((JSON.parse(key) as unknown[])[0] !== sourceId || settled && entry.controller) continue;
    entry.controller?.abort();
    setEntry(client, entries!, key, undefined);
  }
}

export const heldFacts = <V>(pick: (style: HeldStyle) => V | null | undefined): HeldLookup<V> => (timeline, hour, tags) => {
  const styles = stylesIn(timeline, hour, tags);
  if (!styles) return undefined;
  const found = new Map<string, V>();
  for (const [tag, style] of styles) { const value = pick(style); if (value != null) found.set(tag, value); }
  return found;
};
