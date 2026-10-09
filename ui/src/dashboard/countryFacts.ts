/** Facts about countries as recorded at given hours (their names, their flags),
 * for labels and tooltips dated to a moment. Pure parts, shared by the
 * `useCountryNamesAt` and `useCountryFlagsAt` hooks: a store bounded to the hours
 * used most recently, the requests an hour still needs, and reading them on the
 * transient product path, so these many small per-hour replies never push a
 * widget's full-branch data out of the shared product cache. */
import { TRANSIENT_ERRORS } from '../generated/dashboard.ts';
import type { DashboardClient } from './client';
import { loadTransientProduct } from './products.ts';

/** A country at an hour. */
export interface NameAt { hour: number; tag: string }

export const nameKey = (hour: number, tag: string) => JSON.stringify([hour, tag]);

/** A failed read is unknown, distinct from an answered fact with no value. */
export const FAILED_FACT = Symbol('failed fact');

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

  /** Like `pick`, but a country whose read finished without a value is there as
   * null; one still being read is absent. */
  pickRead(wanted: readonly NameAt[]): Map<string, V | null> {
    const picked = new Map<string, V | null>();
    for (const item of wanted) {
      if (this.answered(item.hour, item.tag)) picked.set(nameKey(item.hour, item.tag), this.get(item.hour, item.tag) ?? null);
    }
    return picked;
  }

  clear(): void { this.hours.clear(); this.wanted.clear(); }
}

/** What a set of wanted facts is asked for by: the track and each (hour, country)
 * once, sorted, so the same countries in another order (a race re-ranking) are the
 * same request and never cancel one still being read. */
export function factsKey(trackKey: string, wanted: readonly NameAt[]): string {
  const pairs = [...new Set(wanted.map(item => JSON.stringify([item.hour, item.tag])))].sort();
  return JSON.stringify([trackKey, pairs]);
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

/** Whether a failed request may be tried again: it was cancelled, the service could
 * not be reached (the browser's fetch fails with a TypeError), or the service refused
 * it for want of room (`TRANSIENT_ERRORS`, from the service's own messages). Any
 * other error waits for an explicit retry or a change of content. */
export function transientError(error: unknown): boolean {
  if (typeof DOMException !== 'undefined' && error instanceof DOMException && error.name === 'AbortError') return true;
  if (error instanceof TypeError) return true;
  const message = error instanceof Error ? error.message : String(error);
  return TRANSIENT_ERRORS.some(refusal => message.startsWith(refusal));
}

/** Reads a plan; null reports failure, with whether it may retry automatically. */
export type FactRead<V> = (plan: readonly (readonly [number, readonly string[]])[], signal: AbortSignal,
  onChunk: (hour: number, tags: readonly string[], found: ReadonlyMap<string, V> | null, retryable?: boolean) => void) => Promise<void>;

/** Reads one store runs at once: a sweep of a tooltip across hours never piles up
 * requests past the desktop's limit. */
export const MAX_READING = 2;
/** The wait before a failed read is tried again: doubled each failure, up to the
 * most, so a desktop out of room (other widgets holding every request) is asked
 * again soon but never hammered. */
export const RETRY_FIRST_MS = 250;
export const RETRY_MOST_MS = 8000;

/** When to try a read again after `failures` failures in a row. */
export const retryDelay = (failures: number) => Math.min(RETRY_MOST_MS, RETRY_FIRST_MS * 2 ** Math.max(0, failures - 1));

/** Time for the reads: now, and a callback after a delay (cancellable). */
export interface FactClock {
  now: () => number;
  after: (ms: number, run: () => void) => () => void;
}
const REAL_CLOCK: FactClock = {
  now: () => (typeof performance !== 'undefined' ? performance.now() : Date.now()),
  after: (ms, run) => { const timer = setTimeout(run, ms); return () => clearTimeout(timer); },
};

/** The reads for one store: one request per hour and chunk of countries, at most
 * [`MAX_READING`] at once, the rest queued; the queue is always what is wanted now,
 * so a sweep across hours asks only for the hour it stops at.
 *
 * - A read of an hour no longer wanted is cancelled, freeing its place.
 * - A read of an hour still wanted goes on whatever countries join or leave (a
 *   top-N race's last place changing every few frames), and its answer is kept; a
 *   country being read is not asked for again.
 * - Only a read that completed decides a fact: a country its answer lacks is read
 *   with nothing. A read that failed (the desktop out of room, a transport error)
 *   decides nothing: it is tried again while its hour is wanted, after a wait that
 *   doubles each time ([`retryDelay`]). Product errors wait for `retry` or new content.
 * - Each read that ends starts the next queued, so nothing waits for a re-render.
 *
 * `cancel` (another track, nothing wanted, the widget off screen or gone) stops all. */
export class FactRequests<V> {
  /** Reads going, by id. */
  private readonly reading = new Map<number, { hour: number; tags: readonly string[]; controller: AbortController }>();
  /** The countries they read, by `nameKey`. */
  private readonly keys = new Set<string>();
  /** Failures in a row, and when each may be tried again, by `nameKey`. */
  private readonly failures = new Map<string, { hour: number; count: number; at: number }>();
  private wanted: readonly NameAt[] = [];
  private maxHours = MAX_NAME_HOURS;
  private perRequest = 1;
  private nextId = 0;
  /** The wait for the next retry, and when it is due. */
  private waiting: { at: number; stop: () => void } | null = null;
  private readonly store: HourStore<V>;
  private readonly read: FactRead<V>;
  private readonly changed: () => void;
  private readonly clock: FactClock;

  /** `changed` is told whenever a read completes or fails. */
  constructor(store: HourStore<V>, read: FactRead<V>, changed: () => void, clock: FactClock = REAL_CLOCK) {
    this.store = store;
    this.read = read;
    this.changed = changed;
    this.clock = clock;
  }

  /** The countries being read now, keyed by `nameKey`. */
  get pending(): ReadonlySet<string> { return this.keys; }

  /** How many reads are going. */
  get running(): number { return this.reading.size; }

  /** Completed values, explicit missing values, and failures; pending reads are absent. */
  pickRead(wanted: readonly NameAt[]): Map<string, V | null | typeof FAILED_FACT> {
    const picked: Map<string, V | null | typeof FAILED_FACT> = this.store.pickRead(wanted);
    for (const item of wanted) {
      const key = nameKey(item.hour, item.tag);
      if (this.failures.has(key)) picked.set(key, FAILED_FACT);
    }
    return picked;
  }

  /** Makes `wanted` what is wanted: cancels reads of hours it no longer names and
   * starts what it needs that is neither read nor being read, as places free up.
   * Returns the requests started now. */
  want(wanted: readonly NameAt[], maxHours: number, perRequest: number): number {
    this.wanted = wanted;
    this.maxHours = maxHours;
    this.perRequest = perRequest;
    const hours = new Set(wantedHours(wanted, maxHours));
    this.store.want(hours);
    // Configuration failures wait for retry only while their hours are wanted;
    // playback must not keep a failure for every hour it has passed.
    for (const [key, failure] of this.failures) if (!hours.has(failure.hour)) this.failures.delete(key);
    for (const [id, read] of [...this.reading]) if (!hours.has(read.hour)) this.stop(id);
    return this.pump();
  }

  /** Tries again at once what failed, without waiting. */
  retry(): number {
    this.failures.clear();
    return this.pump();
  }

  /** Stops every read going; what they would have answered is asked for again when wanted. */
  cancel(): void {
    for (const id of [...this.reading.keys()]) this.stop(id);
    this.failures.clear();
    this.waiting?.stop();
    this.waiting = null;
    this.wanted = [];
  }

  private pump(): number {
    const now = this.clock.now();
    let next = Infinity;
    const missing = this.wanted.filter(item => {
      const key = nameKey(item.hour, item.tag);
      if (this.keys.has(key)) return false;
      const failed = this.failures.get(key);
      if (failed && failed.at > now) { next = Math.min(next, failed.at); return false; }
      return true;
    });
    let started = 0;
    for (const [hour, tags] of planRequests(this.store, missing, this.maxHours, this.perRequest)) {
      if (this.reading.size >= MAX_READING) break;
      this.start(hour, tags);
      started++;
    }
    // A failed read still wanted is tried again when its wait is over.
    if (next < Infinity && (!this.waiting || this.waiting.at > next)) {
      this.waiting?.stop();
      const at = next;
      this.waiting = { at, stop: this.clock.after(Math.max(0, at - now), () => { if (this.waiting?.at === at) this.waiting = null; this.pump(); }) };
    }
    return started;
  }

  private start(hour: number, tags: readonly string[]): void {
    const id = this.nextId++;
    const controller = new AbortController();
    this.reading.set(id, { hour, tags, controller });
    for (const tag of tags) this.keys.add(nameKey(hour, tag));
    let reported = false;
    const done = (found: ReadonlyMap<string, V> | null, retryable = true) => {
      if (reported || controller.signal.aborted || !this.reading.has(id)) return;
      reported = true;
      this.stop(id, false);
      if (found) {
        // A completed read decides every country it asked for: one its answer lacks
        // is read with nothing.
        for (const tag of tags) this.failures.delete(nameKey(hour, tag));
        this.store.record(hour, tags, found);
        this.changed();
      } else {
        // Product/configuration errors wait for retry; transient errors back off.
        const at = this.clock.now();
        for (const tag of tags) {
          const key = nameKey(hour, tag);
          const count = (this.failures.get(key)?.count ?? 0) + 1;
          this.failures.set(key, { hour, count, at: retryable ? at + retryDelay(count) : Infinity });
        }
        this.changed();
      }
      this.pump();
    };
    // A read that ends without reporting its chunk failed.
    void this.read([[hour, tags]], controller.signal, (_hour, _tags, found, retryable) => done(found, retryable)).then(() => done(null), () => done(null));
  }

  private stop(id: number, abort = true): void {
    const read = this.reading.get(id);
    if (!read) return;
    if (abort) read.controller.abort();
    this.reading.delete(id);
    for (const tag of read.tags) this.keys.delete(nameKey(read.hour, tag));
  }
}

/** How one kind of fact is read: its product, its parameters for an hour's
 * countries, and the value per country its reply gives. */
export interface FactSpec<V> {
  product: string;
  /** Caller scope when the same product keys facts within a country. */
  identity?: string;
  perRequest: number;
  params: (branchId: string, hour: number, tags: readonly string[]) => unknown;
  extract: (data: unknown, params?: unknown) => ReadonlyMap<string, V>;
}

/** Reads a plan on the transient path, one request at a time, reporting each
 * chunk's values (null when its read failed: a gap or an error). Stops when aborted. */
export async function readFacts<V>(client: DashboardClient, source: { sourceId: string; revision: string; branchId: string }, spec: FactSpec<V>,
  plan: readonly (readonly [number, readonly string[]])[], signal: AbortSignal, onChunk: (hour: number, tags: readonly string[], found: ReadonlyMap<string, V> | null, retryable?: boolean) => void): Promise<void> {
  for (const [hour, tags] of plan) {
    let found: ReadonlyMap<string, V> | null = null;
    let retryable = false;
    try {
      const params = spec.params(source.branchId, hour, tags);
      const data = await loadTransientProduct<unknown>(client, { sourceId: source.sourceId, revision: source.revision, product: spec.product, params }, signal);
      found = spec.extract(data, params);
    } catch (error) {
      retryable = transientError(error);
    }
    if (signal.aborted) return;
    onChunk(hour, tags, found, retryable);
  }
}
