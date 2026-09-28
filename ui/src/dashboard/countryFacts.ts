/** Facts about countries as recorded at given hours (their names, their flags),
 * for labels and tooltips dated to a moment. Pure parts, shared by the
 * `useCountryNamesAt` and `useCountryFlagsAt` hooks: a store bounded to the hours
 * used most recently, the requests an hour still needs, and reading them on the
 * transient product path, so these many small per-hour replies never push a
 * widget's full-branch data out of the shared product cache. */
import type { DashboardClient } from './client';
import { loadTransientProduct } from './products.ts';

/** A country at an hour. */
export interface NameAt { hour: number; tag: string }

export const nameKey = (hour: number, tag: string) => JSON.stringify([hour, tag]);

/** Most distinct hours one label set may ask about by default, and how many hours
 * no longer wanted a store keeps. */
export const MAX_NAME_HOURS = 64;

/** What each hour's countries were found to be. Every hour currently wanted
 * (`want`) is kept; of the others, at most `capacity`, the least recently used
 * dropped first. So a caller that wants more hours than the capacity is never
 * made to ask again for what it still wants. A country asked about whose reply
 * lacked it is answered with no value, so it is not asked again while its hour is kept. */
export class HourStore<V> {
  private hours = new Map<number, { answered: Set<string>; values: Map<string, V> }>();
  private wanted = new Set<number>();
  readonly capacity: number;
  constructor(capacity: number) { this.capacity = Math.max(0, Math.floor(capacity)); }

  /** Hours held. */
  get size(): number { return this.hours.size; }

  /** The hours currently wanted: never dropped until no longer wanted. */
  want(hours: Iterable<number>): void {
    this.wanted = new Set(hours);
    this.touch(this.wanted);
    this.evict();
  }

  answered(hour: number, tag: string): boolean { return this.hours.get(hour)?.answered.has(tag) ?? false; }

  get(hour: number, tag: string): V | undefined { return this.hours.get(hour)?.values.get(tag); }

  /** Marks hours as just used, so they are the last dropped. */
  touch(hours: Iterable<number>): void {
    for (const hour of hours) {
      const entry = this.hours.get(hour);
      if (entry) { this.hours.delete(hour); this.hours.set(hour, entry); }
    }
  }

  /** Records a reply for `tags` at `hour` and drops the least recently used hours
   * not wanted past the capacity. */
  record(hour: number, tags: readonly string[], found: ReadonlyMap<string, V>): void {
    const entry = this.hours.get(hour) ?? { answered: new Set<string>(), values: new Map<string, V>() };
    for (const tag of tags) {
      entry.answered.add(tag);
      const value = found.get(tag);
      if (value !== undefined) entry.values.set(tag, value);
    }
    this.hours.delete(hour);
    this.hours.set(hour, entry);
    this.evict();
  }

  private evict(): void {
    let unwanted = 0;
    for (const hour of this.hours.keys()) if (!this.wanted.has(hour)) unwanted++;
    for (const hour of [...this.hours.keys()]) {
      if (unwanted <= this.capacity) break;
      if (this.wanted.has(hour)) continue;
      this.hours.delete(hour);
      unwanted--;
    }
  }

  /** The values held for `wanted`, keyed by `nameKey`: only what is asked for, never the whole store. */
  pick(wanted: readonly NameAt[]): Map<string, V> {
    const picked = new Map<string, V>();
    for (const item of wanted) {
      const value = this.get(item.hour, item.tag);
      if (value !== undefined) picked.set(nameKey(item.hour, item.tag), value);
    }
    return picked;
  }

  clear(): void { this.hours.clear(); this.wanted.clear(); }
}

/** The hours `wanted` names, earliest first, at most `maxHours` of them. */
export function wantedHours(wanted: readonly NameAt[], maxHours: number): number[] {
  return [...new Set(wanted.map(item => item.hour))].sort((a, b) => a - b).slice(0, maxHours);
}

/** The requests `wanted` still needs: per hour (earliest first, at most `maxHours`),
 * the countries not yet answered, in chunks of at most `perRequest`. */
export function planRequests<V>(store: HourStore<V>, wanted: readonly NameAt[], maxHours: number, perRequest: number): [number, string[]][] {
  const hours = new Set(wantedHours(wanted, maxHours));
  const byHour = new Map<number, Set<string>>();
  for (const item of wanted) {
    if (!hours.has(item.hour) || store.answered(item.hour, item.tag)) continue;
    const tags = byHour.get(item.hour) ?? new Set<string>();
    tags.add(item.tag);
    byHour.set(item.hour, tags);
  }
  const plan: [number, string[]][] = [];
  for (const hour of [...byHour.keys()].sort((a, b) => a - b)) {
    const tags = [...byHour.get(hour)!].sort();
    for (let start = 0; start < tags.length; start += Math.max(1, perRequest)) plan.push([hour, tags.slice(start, start + Math.max(1, perRequest))]);
  }
  return plan;
}

/** How one kind of fact is read: its product, its parameters for an hour's
 * countries, and the value per country its reply gives. */
export interface FactSpec<V> {
  product: string;
  perRequest: number;
  params: (branchId: string, hour: number, tags: readonly string[]) => unknown;
  extract: (data: unknown) => ReadonlyMap<string, V>;
}

/** Reads a plan on the transient path, one request at a time, reporting each
 * chunk's values (null when its read failed: a gap or an error). Stops when aborted. */
export async function readFacts<V>(client: DashboardClient, source: { sourceId: string; revision: string; branchId: string }, spec: FactSpec<V>,
  plan: readonly (readonly [number, readonly string[]])[], signal: AbortSignal, onChunk: (hour: number, tags: readonly string[], found: ReadonlyMap<string, V> | null) => void): Promise<void> {
  for (const [hour, tags] of plan) {
    let found: ReadonlyMap<string, V> | null = null;
    try {
      const data = await loadTransientProduct<unknown>(client, { sourceId: source.sourceId, revision: source.revision, product: spec.product,
        params: spec.params(source.branchId, hour, tags) }, signal);
      found = spec.extract(data);
    } catch {
      // Not read: the caller's fallback shows, and it is asked again next time.
    }
    if (signal.aborted) return;
    onChunk(hour, tags, found);
  }
}
