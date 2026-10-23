/** Facts about countries as recorded at given hours (their names, their flags,
 * their colours), for labels, tooltips and charts dated to a moment. Pure parts,
 * shared by the dated fact hooks: a store bounded to the hours used most recently,
 * the requests an hour still needs, and reading them, per hour or many hours in one
 * request, on the transient product path, so these replies never push a widget's
 * full-branch data out of the shared product cache. */
import { TRANSIENT_ERRORS } from '../generated/dashboard.ts';
import type { DashboardClient, DashboardSource } from './client';
import { loadTransientProduct } from './products.ts';
import { replySize, setReplySize } from './replySizes.ts';

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
  /** Hours held that are not wanted, so recording a wanted hour evicts in constant time. */
  private unwanted = 0;
  readonly capacity: number;
  constructor(capacity: number) { this.capacity = Math.max(0, Math.floor(capacity)); }

  /** Hours held. */
  get size(): number { return this.hours.size; }

  /** The hours currently wanted: never dropped until no longer wanted. */
  want(hours: Iterable<number>): void {
    this.wanted = new Set(hours);
    this.touch(this.wanted);
    this.unwanted = 0;
    for (const hour of this.hours.keys()) if (!this.wanted.has(hour)) this.unwanted++;
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
    let entry = this.hours.get(hour);
    if (!entry) {
      entry = { answered: new Set<string>(), values: new Map<string, V>() };
      if (!this.wanted.has(hour)) this.unwanted++;
    }
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
    if (this.unwanted <= this.capacity) return;
    for (const hour of [...this.hours.keys()]) {
      if (this.unwanted <= this.capacity) break;
      if (this.wanted.has(hour)) continue;
      this.hours.delete(hour);
      this.unwanted--;
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

  clear(): void { this.hours.clear(); this.wanted.clear(); this.unwanted = 0; }
}

/** The track and each wanted pair once. Time-ordered facts ignore list reordering;
 * artwork also tracks displayed order so queued work can follow a changed view. */
export function factsKey(trackKey: string, wanted: readonly NameAt[], order: FactRequestOrder = 'time'): string {
  const pairs = [...new Set(wanted.map(item => JSON.stringify([item.hour, item.tag])))];
  if (order === 'time') pairs.sort();
  return JSON.stringify([trackKey, pairs]);
}

/** The hours `wanted` names, earliest first, at most `maxHours` of them. */
export function wantedHours(wanted: readonly NameAt[], maxHours: number): number[] {
  return [...new Set(wanted.map(item => item.hour))].sort((a, b) => a - b).slice(0, maxHours);
}

/** Artwork may request list order so the visible rows load before later rows. */
export type FactRequestOrder = 'time' | 'wanted';

/** The requests `wanted` still needs, in chunks of at most `perRequest`.
 * Hours remain subject to the caller's admission; ordering only schedules them. */
export function planRequests<V>(store: HourStore<V>, wanted: readonly NameAt[], maxHours: number, perRequest: number,
  order: FactRequestOrder = 'time'): [number, string[]][] {
  const hours = new Set(wantedHours(wanted, maxHours));
  const byHour = new Map<number, Set<string>>();
  for (const item of wanted) {
    if (!hours.has(item.hour) || store.answered(item.hour, item.tag)) continue;
    const tags = byHour.get(item.hour) ?? new Set<string>();
    tags.add(item.tag);
    byHour.set(item.hour, tags);
  }
  const plan: [number, string[]][] = [];
  const orderedHours = [...byHour.keys()];
  if (order === 'time') orderedHours.sort((a, b) => a - b);
  for (const hour of orderedHours) {
    const tags = [...byHour.get(hour)!];
    if (order === 'time') tags.sort();
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

/** Hours to read, each with its countries. */
export type FactPlan = readonly (readonly [number, readonly string[]])[];

/** Reads a plan, telling each chunk in plan order; null reports failure, with whether
 * it may retry automatically. `failed` names countries of an answered chunk whose part
 * of the read failed, each with whether it may retry; the others are answered. */
export type FactRead<V> = (plan: FactPlan, signal: AbortSignal,
  onChunk: (hour: number, tags: readonly string[], found: ReadonlyMap<string, V> | null, retryable?: boolean,
    failed?: ReadonlyMap<string, boolean>) => void) => Promise<void>;

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

/** Time for the reads: now, a callback after a delay (cancellable), and optionally
 * one before the next frame is drawn (without it, changes are told at once). */
export interface FactClock {
  now: () => number;
  after: (ms: number, run: () => void) => () => void;
  frame?: (run: () => void) => void;
}
const REAL_CLOCK: FactClock = {
  now: () => (typeof performance !== 'undefined' ? performance.now() : Date.now()),
  after: (ms, run) => { const timer = setTimeout(run, ms); return () => clearTimeout(timer); },
  frame: typeof requestAnimationFrame === 'function' ? run => { requestAnimationFrame(run); } : undefined,
};

/** The reads for one store: one request per hour and chunk of countries, or, batched,
 * one request for everything queued; at most [`MAX_READING`] at once, the rest queued.
 * The queue is always what is wanted now, so a sweep across hours asks only for the
 * hour it stops at.
 *
 * - A read none of whose hours is still wanted is cancelled, freeing its place; a
 *   batched read, once none of the countries it has still to tell is wanted.
 * - A read of an hour still wanted goes on whatever countries join or leave (a
 *   top-N race's last place changing every few frames), and its answer is kept; a
 *   country being read is not asked for again.
 * - Only a read that completed decides a fact: a country its answer lacks is read
 *   with nothing. A read that failed (the desktop out of room, a transport error)
 *   decides nothing: it is tried again while its fact is wanted, after a wait that
 *   doubles each time ([`retryDelay`]). Product errors wait for `retry` or new content.
 * - Each read that ends starts the next queued, so nothing waits for a re-render.
 * - Reads ending in the same frame are told as one change, so many small replies
 *   arriving together update their consumers once.
 *
 * `cancel` (another track, nothing wanted, the widget off screen or gone) stops all. */
export class FactRequests<V> {
  /** Reads going, by id, with how many of their chunks have been told. */
  private readonly reading = new Map<number, { chunks: FactPlan; told: number; batched: boolean; controller: AbortController }>();
  /** The countries they read, by `nameKey`. */
  private readonly keys = new Set<string>();
  /** Failures in a row, and when each may be tried again, by `nameKey`. */
  private readonly failures = new Map<string, { count: number; at: number }>();
  private wantedKeys = new Set<string>();
  private wanted: readonly NameAt[] = [];
  private maxHours = MAX_NAME_HOURS;
  private perRequest = 1;
  private order: FactRequestOrder = 'time';
  private batched = false;
  /** Exact pending chunks survive successful replies. Demand and retry changes
   * rebuild the plan; a completed chunk only frees a slot for the next one. */
  private queue: [number, string[]][] = [];
  private queuedIndex = 0;
  private nextId = 0;
  /** The wait for the next retry, and when it is due. */
  private waiting: { at: number; stop: () => void } | null = null;
  /** Whether a change is to be told at the next frame. */
  private telling = false;
  private readonly store: HourStore<V>;
  private readonly read: FactRead<V>;
  private readonly changed: () => void;
  private readonly clock: FactClock;

  /** `changed` is told, at most once a frame, whenever reads complete or fail. */
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
   * starts what it needs that is neither read nor being read, as places free up;
   * `batched` reads everything queued in one request. Returns the requests started now. */
  want(wanted: readonly NameAt[], maxHours: number, perRequest: number, order: FactRequestOrder = 'time', batched = false): number {
    this.wanted = wanted;
    this.maxHours = maxHours;
    this.perRequest = perRequest;
    this.order = order;
    this.batched = batched;
    const hours = new Set(wantedHours(wanted, maxHours));
    this.wantedKeys = new Set(wanted.filter(item => hours.has(item.hour)).map(item => nameKey(item.hour, item.tag)));
    this.store.want(hours);
    // Failures belong to exact demand: another country keeping the hour active
    // must not retain a departed country's refusals.
    for (const key of this.failures.keys()) if (!this.wantedKeys.has(key)) this.failures.delete(key);
    for (const [id, read] of [...this.reading]) {
      const untold = read.chunks.slice(read.told);
      const wanted = read.batched ? untold.some(([hour, tags]) => tags.some(tag => this.wantedKeys.has(nameKey(hour, tag))))
        : untold.some(([hour]) => hours.has(hour));
      if (!wanted) this.stop(id);
    }
    return this.pump(true);
  }

  /** Tries again at once what failed, without waiting. */
  retry(): number {
    this.failures.clear();
    return this.pump(true);
  }

  /** Stops every read going; what they would have answered is asked for again when wanted. */
  cancel(): void {
    for (const id of [...this.reading.keys()]) this.stop(id);
    this.failures.clear();
    this.waiting?.stop();
    this.waiting = null;
    this.wanted = [];
    this.queue = [];
    this.queuedIndex = 0;
    this.wantedKeys.clear();
  }

  private pump(rebuild = false): number {
    if (rebuild) {
      const now = this.clock.now();
      let next = Infinity;
      const missing = this.wanted.filter(item => {
        const key = nameKey(item.hour, item.tag);
        if (this.keys.has(key)) return false;
        const failed = this.failures.get(key);
        if (failed && failed.at > now) { next = Math.min(next, failed.at); return false; }
        return true;
      });
      this.queue = planRequests(this.store, missing, this.maxHours, this.perRequest, this.order);
      this.queuedIndex = 0;
      // A failed read still wanted is tried again when its wait is over.
      if (next < Infinity && (!this.waiting || this.waiting.at > next)) {
        this.waiting?.stop();
        const at = next;
        this.waiting = { at, stop: this.clock.after(Math.max(0, at - now), () => {
          if (this.waiting?.at === at) this.waiting = null;
          this.pump(true);
        }) };
      }
    }
    let started = 0;
    while (this.queuedIndex < this.queue.length && this.reading.size < MAX_READING) {
      const chunks: [number, string[]][] = [];
      while (this.queuedIndex < this.queue.length && (this.batched || !chunks.length)) {
        const [hour, planned] = this.queue[this.queuedIndex++];
        const tags = planned.filter(tag => !this.keys.has(nameKey(hour, tag)) && !this.store.answered(hour, tag));
        if (tags.length) chunks.push([hour, tags]);
      }
      if (!chunks.length) continue;
      this.start(chunks, this.batched);
      started++;
    }
    return started;
  }

  private start(chunks: FactPlan, batched: boolean): void {
    const id = this.nextId++;
    const controller = new AbortController();
    const read = { chunks, told: 0, batched, controller };
    this.reading.set(id, read);
    for (const [hour, tags] of chunks) for (const tag of tags) this.keys.add(nameKey(hour, tag));
    let failed = false;
    // Product/configuration errors wait for retry; transient errors back off.
    const fail = (hour: number, tags: readonly string[], retryable: boolean) => {
      failed = true;
      const at = this.clock.now();
      for (const tag of tags) {
        const key = nameKey(hour, tag);
        if (!this.wantedKeys.has(key)) continue;
        const count = (this.failures.get(key)?.count ?? 0) + 1;
        this.failures.set(key, { count, at: retryable ? at + retryDelay(count) : Infinity });
      }
    };
    const done = (found: ReadonlyMap<string, V> | null, retryable = true, unanswered?: ReadonlyMap<string, boolean>) => {
      if (controller.signal.aborted || this.reading.get(id) !== read) return;
      const [hour, tags] = chunks[read.told++];
      for (const tag of tags) this.keys.delete(nameKey(hour, tag));
      const last = read.told === chunks.length;
      if (last) this.reading.delete(id);
      if (found) {
        // A completed read decides every country it asked for: one its answer lacks
        // is read with nothing.
        const answered = unanswered?.size ? tags.filter(tag => !unanswered.has(tag)) : tags;
        for (const tag of answered) this.failures.delete(nameKey(hour, tag));
        this.store.record(hour, answered, found);
        if (unanswered?.size) for (const again of [true, false]) fail(hour, tags.filter(tag => unanswered.get(tag) === again), again);
      } else fail(hour, tags, retryable);
      this.tell();
      if (last) this.pump(failed);
    };
    // A read that ends without telling a chunk failed it.
    const end = () => { while (this.reading.get(id) === read && !controller.signal.aborted) done(null); };
    void this.read(chunks, controller.signal, (_hour, _tags, found, retryable, unanswered) => done(found, retryable, unanswered)).then(end, end);
  }

  private tell(): void {
    if (!this.clock.frame) { this.changed(); return; }
    if (this.telling) return;
    this.telling = true;
    this.clock.frame(() => { this.telling = false; this.changed(); });
  }

  private stop(id: number): void {
    const read = this.reading.get(id);
    if (!read) return;
    read.controller.abort();
    this.reading.delete(id);
    for (const [hour, tags] of read.chunks.slice(read.told)) for (const tag of tags) this.keys.delete(nameKey(hour, tag));
  }
}

/** The recording a read is of; `content` is the content revision it was asked under. */
export interface FactSource { sourceId: string; revision: string; branchId: string; content?: number }

/** What a reply gives: at once, or once its values are prepared (such as encoded artwork). */
export type FactsFound<V> = ReadonlyMap<string, V> | Promise<ReadonlyMap<string, V>>;

/** How one kind of fact is read: its product, its parameters for an hour's
 * countries, and the value per country its reply gives. */
export interface FactSpec<V, Found extends FactsFound<V> = ReadonlyMap<string, V>> {
  product: string;
  /** Caller scope when the same product keys facts within a country. */
  identity?: string;
  perRequest: number;
  /** Preserve displayed list order for artwork; other dated facts use time order. */
  requestOrder?: FactRequestOrder;
  params: (branchId: string, hour: number, tags: readonly string[]) => unknown;
  extract: (data: unknown, params?: unknown, source?: FactSource) => Found;
}

/** A product read for a whole plan in one request, for facts wanted at many hours at once. */
export interface BatchFactSpec<V> {
  product: string;
  /** Caller scope when the same product keys facts within a country. */
  identity?: string;
  batch: true;
  params: (branchId: string, plan: FactPlan) => unknown;
  /** Values by hour, then country; an hour absent was not read. */
  extract: (data: unknown, plan: FactPlan) => ReadonlyMap<number, ReadonlyMap<string, V>>;
  /** How a plan with one hour is read instead: a single hour's read, on its quicker path. */
  single?: FactSpec<V, FactsFound<V>>;
  /** The start of the refusal of a reply too large for one request: its plan is read again in halves. */
  oversize?: string;
  /** How large a plan's request is, as its parameters select; a size that fit is
   * learned and later plans sliced in it. Without it, the hour and country pairs. */
  size?: (plan: FactPlan) => number;
}

/** Reads a plan in one request on the transient path, telling every chunk in plan
 * order. An hour the reply could not read, or a reply that cannot be understood,
 * fails without retrying; a failed request fails every chunk. */
export async function readFactBatch<V>(client: DashboardClient, source: FactSource, spec: BatchFactSpec<V>, plan: FactPlan, signal: AbortSignal,
  onChunk: Parameters<FactRead<V>>[2], recording?: DashboardSource): Promise<void> {
  const scope = JSON.stringify([spec.product, source.revision, source.branchId]);
  // After a refusal, a recording's later plans start at the largest request one reply held.
  const fits = replySize(client, source.sourceId, scope)?.fits ?? 0;
  const size = spec.size ?? pairs;
  if (fits > 0 && size(plan) > fits) {
    for (const slice of slices(plan, fits, size)) {
      if (signal.aborted) return;
      await readBatch(client, source, spec, slice, signal, onChunk, recording, scope);
    }
    return;
  }
  await readBatch(client, source, spec, plan, signal, onChunk, recording, scope);
}

const pairs = (plan: FactPlan) => plan.reduce((sum, [, tags]) => sum + tags.length, 0);

/** `plan` in order, in the longest runs whose `size` is at most `most` (one chunk at
 * least). A size never shrinks as a run grows, so each run is found by doubling, then halving. */
function slices(plan: FactPlan, most: number, size: (plan: FactPlan) => number): FactPlan[] {
  const out: FactPlan[] = [];
  for (let start = 0; start < plan.length;) {
    const left = plan.length - start;
    const fits = (count: number) => size(plan.slice(start, start + count)) <= most;
    let low = 1, high = 1;
    while (high < left && fits(Math.min(left, high * 2))) high = Math.min(left, high * 2);
    low = high;
    high = Math.min(left, high * 2);
    while (low < high) {
      const middle = (low + high + 1) >> 1;
      if (fits(middle)) low = middle; else high = middle - 1;
    }
    out.push(plan.slice(start, start + low));
    start += low;
  }
  return out;
}

/** `readFactBatch` of one plan, keeping what fits for `scope` once a refusal has split one. */
async function readBatch<V>(client: DashboardClient, source: FactSource, spec: BatchFactSpec<V>, plan: FactPlan, signal: AbortSignal,
  onChunk: Parameters<FactRead<V>>[2], recording: DashboardSource | undefined, scope: string): Promise<void> {
  if (spec.single && new Set(plan.map(([hour]) => hour)).size === 1) {
    for (const [hour, tags] of plan) await readChunk(client, source, spec.single, hour, tags, signal, onChunk, recording);
    return;
  }
  let data: unknown;
  try {
    data = await loadTransientProduct<unknown>(client, { sourceId: source.sourceId, revision: source.revision, product: spec.product,
      params: spec.params(source.branchId, plan) }, signal, recording);
  } catch (error) {
    if (signal.aborted) return;
    const message = error instanceof Error ? error.message : String(error);
    if (spec.oversize !== undefined && plan.length > 1 && message.startsWith(spec.oversize)) {
      // A refusal at the size kept lowers it: half is the size tried next.
      const size = (spec.size ?? pairs)(plan), held = replySize(client, source.sourceId, scope);
      setReplySize(client, source.sourceId, scope, { fits: Math.min(held?.fits ?? Infinity, size >> 1), refused: Math.min(held?.refused ?? Infinity, size) });
      const half = plan.length >> 1;
      await readBatch(client, source, spec, plan.slice(0, half), signal, onChunk, recording, scope);
      if (!signal.aborted) await readBatch(client, source, spec, plan.slice(half), signal, onChunk, recording, scope);
      return;
    }
    const retryable = transientError(error);
    for (const [hour, tags] of plan) onChunk(hour, tags, null, retryable);
    return;
  }
  if (signal.aborted) return;
  const held = replySize(client, source.sourceId, scope);
  if (held) {
    const size = (spec.size ?? pairs)(plan);
    // A reply as large as one refused now fits: the recording's replies shrank.
    setReplySize(client, source.sourceId, scope, size < held.refused ? { fits: Math.max(held.fits, size), refused: held.refused } : { fits: size, refused: Infinity });
  }
  let found: ReadonlyMap<number, ReadonlyMap<string, V>> | null = null;
  try {
    found = spec.extract(data, plan);
  } catch {
    // A reply that cannot be understood is a failure, never a refusal to try again.
  }
  for (const [hour, tags] of plan) onChunk(hour, tags, found?.get(hour) ?? null, false);
}

/** One hour's countries read with `spec`, in as many requests as it allows, told as
 * one chunk: the countries of a failed request fail, each as its own request may retry,
 * and the others keep their answers. */
async function readChunk<V>(client: DashboardClient, source: FactSource, spec: FactSpec<V, FactsFound<V>>, hour: number, tags: readonly string[],
  signal: AbortSignal, onChunk: Parameters<FactRead<V>>[2], recording?: DashboardSource): Promise<void> {
  const size = Math.max(1, spec.perRequest);
  const pieces: [number, string[]][] = [];
  for (let start = 0; start < tags.length; start += size) pieces.push([hour, tags.slice(start, start + size)]);
  const found = new Map<string, V>();
  // Each failed country, and whether its request may be tried again.
  const failed = new Map<string, boolean>();
  await readFacts(client, source, spec, pieces, signal, (_hour, part, values, again = true) => {
    if (values) for (const [tag, value] of values) found.set(tag, value);
    else for (const tag of part) failed.set(tag, again);
  }, recording);
  if (signal.aborted) return;
  const alike = new Set(failed.values());
  if (!failed.size) onChunk(hour, tags, found);
  else if (failed.size === tags.length && alike.size === 1) onChunk(hour, tags, null, [...alike][0]);
  else onChunk(hour, tags, found, undefined, failed);
}

/** Reads a plan on the transient path, one request at a time, reporting each
 * chunk's values (null when its read failed: a gap or an error). Stops when aborted. */
export async function readFacts<V>(client: DashboardClient, source: FactSource, spec: FactSpec<V, FactsFound<V>>,
  plan: FactPlan, signal: AbortSignal, onChunk: Parameters<FactRead<V>>[2], recording?: DashboardSource): Promise<void> {
  for (const [hour, tags] of plan) {
    let found: ReadonlyMap<string, V> | null = null;
    let retryable = false;
    try {
      const params = spec.params(source.branchId, hour, tags);
      const data = await loadTransientProduct<unknown>(client, { sourceId: source.sourceId, revision: source.revision, product: spec.product, params }, signal, recording);
      found = await spec.extract(data, params, source);
    } catch (error) {
      retryable = transientError(error);
    }
    if (signal.aborted) return;
    onChunk(hour, tags, found, retryable);
  }
}
