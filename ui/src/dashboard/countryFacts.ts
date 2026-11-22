import type { ParamsFor } from './regionTime';
import { asFailure, type Failure } from './queryError.ts';
import { TRANSIENT_ERRORS } from '../generated/dashboard.ts';
import type { DashboardClient, DashboardSource } from './client';
import { loadTransientProduct } from './products.ts';
import { replySize, setReplySize } from './replySizes.ts';
import { sharedFacts } from './sharedFacts.ts';
import type { FactFailure } from './sharedFacts';
import { inFrame } from './frameUpdates.ts';
import { askTimeline } from './countryTimeline.ts';
import type { HeldLookup } from './countryTimeline';
import { busyWait } from '../shell/busy.ts';
import { forwardRequest, type DeclaredProduct } from './request.ts';

export interface NameAt { hour: number; tag: string }

export const nameKey = (hour: number, tag: string) => JSON.stringify([hour, tag]);

export const FAILED_FACT = new Error('Fact request failed');
export function isFailedFact(value: unknown): value is Failure {
  return value instanceof Error;
}

export const MAX_NAME_HOURS = 64;

export class HourStore<V> {
  private hours = new Map<number, { answered: Set<string>; values: Map<string, V> }>();
  private wanted = new Set<number>();
  private unwanted = 0;
  private capacity: number;
  constructor(capacity: number) { this.capacity = Math.max(0, Math.floor(capacity)); }

  get size(): number { return this.hours.size; }

  want(hours: Iterable<number>): void {
    this.wanted = new Set(hours);
    this.capacity = Math.max(this.capacity, this.wanted.size);
    this.touch(this.wanted);
    this.unwanted = 0;
    for (const hour of this.hours.keys()) if (!this.wanted.has(hour)) this.unwanted++;
    this.evict();
  }

  answered(hour: number, tag: string): boolean { return this.hours.get(hour)?.answered.has(tag) ?? false; }

  get(hour: number, tag: string): V | undefined { return this.hours.get(hour)?.values.get(tag); }

  touch(hours: Iterable<number>): void {
    for (const hour of hours) {
      const entry = this.hours.get(hour);
      if (entry) { this.hours.delete(hour); this.hours.set(hour, entry); }
    }
  }

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

  pick(wanted: readonly NameAt[]): Map<string, V> {
    const picked = new Map<string, V>();
    for (const item of wanted) {
      const value = this.get(item.hour, item.tag);
      if (value !== undefined) picked.set(nameKey(item.hour, item.tag), value);
    }
    return picked;
  }

  pickRead(wanted: readonly NameAt[]): Map<string, V | null> {
    const picked = new Map<string, V | null>();
    for (const item of wanted) {
      if (this.answered(item.hour, item.tag)) picked.set(nameKey(item.hour, item.tag), this.get(item.hour, item.tag) ?? null);
    }
    return picked;
  }

  clear(): void { this.hours.clear(); this.wanted.clear(); this.unwanted = 0; }
}

export function factsKey(trackKey: string, wanted: readonly NameAt[], order: FactRequestOrder = 'time'): string {
  const pairs = [...new Set(wanted.map(item => JSON.stringify([item.hour, item.tag])))];
  if (order === 'time') pairs.sort();
  return JSON.stringify([trackKey, pairs]);
}

export function wantedHours(wanted: readonly NameAt[], maxHours: number): number[] {
  return [...new Set(wanted.map(item => item.hour))].sort((a, b) => a - b).slice(0, maxHours);
}

export type FactRequestOrder = 'time' | 'wanted';

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

export function transientError(error: unknown): boolean {
  if (typeof DOMException !== 'undefined' && error instanceof DOMException && error.name === 'AbortError') return true;
  if (error instanceof TypeError) return true;
  const message = asFailure(error).message;
  return TRANSIENT_ERRORS.some(refusal => message.startsWith(refusal));
}

export type FactPlan = readonly (readonly [number, readonly string[]])[];

export type FactRead<V> = (plan: FactPlan, signal: AbortSignal,
  onChunk: (hour: number, tags: readonly string[], found: ReadonlyMap<string, V> | null, retryable?: boolean,
    failed?: ReadonlyMap<string, FactFailure>, failure?: Failure) => void) => Promise<void>;

export const MAX_READING = 2;
export const RETRY_FIRST_MS = 250;
export const RETRY_MOST_MS = 8000;

export const retryDelay = (failures: number) => Math.min(RETRY_MOST_MS, RETRY_FIRST_MS * 2 ** Math.max(0, failures - 1));

export interface FactClock {
  now: () => number;
  after: (ms: number, run: () => void) => () => void;
  frame?: (run: () => void) => void;
}
const REAL_CLOCK: FactClock = {
  now: () => (typeof performance !== 'undefined' ? performance.now() : Date.now()),
  after: (ms, run) => busyWait(run, ms),
  frame: typeof requestAnimationFrame === 'function' ? inFrame : undefined,
};

export class FactRequests<V> {
  private readonly reading = new Map<number, { chunks: FactPlan; told: number; batched: boolean; controller: AbortController }>();
  private readonly keys = new Set<string>();
  private readonly failures = new Map<string, { count: number; at: number; error: Failure }>();
  private wantedKeys = new Set<string>();
  private wanted: readonly NameAt[] = [];
  private maxHours = MAX_NAME_HOURS;
  private perRequest = 1;
  private order: FactRequestOrder = 'time';
  private batched = false;
  private queue: [number, string[]][] = [];
  private queuedIndex = 0;
  private nextId = 0;
  private waiting: { at: number; stop: () => void } | null = null;
  private telling = false;
  private pumping = false;
  private again: boolean | null = null;
  private readonly store: HourStore<V>;
  private readonly read: FactRead<V>;
  private readonly changed: () => void;
  private readonly clock: FactClock;

  constructor(store: HourStore<V>, read: FactRead<V>, changed: () => void, clock: FactClock = REAL_CLOCK) {
    this.store = store;
    this.read = read;
    this.changed = changed;
    this.clock = clock;
  }

  get pending(): ReadonlySet<string> { return this.keys; }

  get running(): number { return this.reading.size; }

  pickRead(wanted: readonly NameAt[]): Map<string, V | null | Failure> {
    const picked: Map<string, V | null | Failure> = this.store.pickRead(wanted);
    for (const item of wanted) {
      const key = nameKey(item.hour, item.tag);
      const failed = this.failures.get(key);
      if (failed) picked.set(key, failed.error);
    }
    return picked;
  }

  want(wanted: readonly NameAt[], maxHours: number, perRequest: number, order: FactRequestOrder = 'time', batched = false): number {
    this.wanted = wanted;
    this.maxHours = maxHours;
    this.perRequest = perRequest;
    this.order = order;
    this.batched = batched;
    const hours = new Set(wantedHours(wanted, maxHours));
    this.wantedKeys = new Set(wanted.filter(item => hours.has(item.hour)).map(item => nameKey(item.hour, item.tag)));
    this.store.want(hours);
    for (const key of this.failures.keys()) if (!this.wantedKeys.has(key)) this.failures.delete(key);
    for (const [id, read] of [...this.reading]) {
      const untold = read.chunks.slice(read.told);
      const wanted = read.batched ? untold.some(([hour, tags]) => tags.some(tag => this.wantedKeys.has(nameKey(hour, tag))))
        : untold.some(([hour]) => hours.has(hour));
      if (!wanted) this.stop(id);
    }
    return this.pump(true);
  }

  retry(): number {
    this.failures.clear();
    return this.pump(true);
  }

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
    if (this.pumping) { this.again = !!this.again || rebuild; return 0; }
    this.pumping = true;
    let started = 0;
    try {
      for (let pass: boolean | null = rebuild; pass !== null; pass = this.again) {
        this.again = null;
        started += this.pumpOnce(pass);
      }
    } finally { this.pumping = false; }
    return started;
  }

  private pumpOnce(rebuild: boolean): number {
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
    const fail = (hour: number, tags: readonly string[], retryable: boolean, error: Failure) => {
      failed = true;
      const at = this.clock.now();
      for (const tag of tags) {
        const key = nameKey(hour, tag);
        if (!this.wantedKeys.has(key)) continue;
        const count = (this.failures.get(key)?.count ?? 0) + 1;
        this.failures.set(key, { count, at: retryable ? at + retryDelay(count) : Infinity, error });
      }
    };
    const done = (found: ReadonlyMap<string, V> | null, retryable = true, unanswered?: ReadonlyMap<string, FactFailure>, error: Failure = FAILED_FACT) => {
      if (controller.signal.aborted || this.reading.get(id) !== read) return;
      const [hour, tags] = chunks[read.told++];
      for (const tag of tags) this.keys.delete(nameKey(hour, tag));
      const last = read.told === chunks.length;
      if (last) this.reading.delete(id);
      if (found) {
        const answered = unanswered?.size ? tags.filter(tag => !unanswered.has(tag)) : tags;
        for (const tag of answered) this.failures.delete(nameKey(hour, tag));
        this.store.record(hour, answered, found);
        for (const tag of tags) {
          const failure = unanswered?.get(tag);
          if (failure) fail(hour, [tag], failure.retryable, failure.error);
        }
      } else fail(hour, tags, retryable, error);
      this.tell();
      if (last) this.pump(failed);
    };
    const end = (error?: unknown) => { while (this.reading.get(id) === read && !controller.signal.aborted) done(null, true, undefined, error === undefined ? FAILED_FACT : asFailure(error)); };
    void this.read(chunks, controller.signal, (_hour, _tags, found, retryable, unanswered, error) => done(found, retryable, unanswered, error)).then(end, end);
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

export type LabelParams = Readonly<Record<string, unknown>>;
const labelParams = (params: LabelParams) => params as ParamsFor<DeclaredProduct>;

export interface FactSource { sourceId: string; revision: string; branchId: string; content?: number }

export type FactsFound<V> = ReadonlyMap<string, V> | Promise<ReadonlyMap<string, V>>;

export interface FactSpec<V, Found extends FactsFound<V> = ReadonlyMap<string, V>> {
  product: DeclaredProduct;
  identity?: string;
  perRequest: number;
  requestOrder?: FactRequestOrder;
  shared?: boolean;
  params: (branchId: string, hour: number, tags: readonly string[]) => LabelParams;
  extract: (data: unknown, params?: unknown, source?: FactSource) => Found;
  held?: HeldLookup<V>;
}

export interface BatchFactSpec<V> {
  product: DeclaredProduct;
  identity?: string;
  batch: true;
  params: (branchId: string, plan: FactPlan) => LabelParams;
  extract: (data: unknown, plan: FactPlan) => ReadonlyMap<number, ReadonlyMap<string, V>>;
  single?: FactSpec<V, FactsFound<V>>;
  oversize?: string;
  size?: (plan: FactPlan) => number;
  held?: HeldLookup<V>;
}

export async function readFactBatch<V>(client: DashboardClient, source: FactSource, spec: BatchFactSpec<V>, plan: FactPlan, signal: AbortSignal,
  onChunk: Parameters<FactRead<V>>[2], recording?: DashboardSource): Promise<void> {
  const scope = JSON.stringify([spec.product, source.revision, source.branchId]);
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

async function readBatch<V>(client: DashboardClient, source: FactSource, spec: BatchFactSpec<V>, plan: FactPlan, signal: AbortSignal,
  onChunk: Parameters<FactRead<V>>[2], recording: DashboardSource | undefined, scope: string): Promise<void> {
  if (spec.single && new Set(plan.map(([hour]) => hour)).size === 1) {
    for (const [hour, tags] of plan) await readChunk(client, source, spec.single, hour, tags, signal, onChunk, recording);
    return;
  }
  let data: unknown;
  try {
    data = await loadTransientProduct<unknown>(client, forwardRequest({ id: source.sourceId, revision: source.revision }, spec.product,
      labelParams(spec.params(source.branchId, plan))), signal, recording);
  } catch (error) {
    if (signal.aborted) return;
    const message = asFailure(error).message;
    if (spec.oversize !== undefined && plan.length > 1 && message.startsWith(spec.oversize)) {
      const size = (spec.size ?? pairs)(plan), held = replySize(client, source.sourceId, scope);
      setReplySize(client, source.sourceId, scope, { fits: Math.min(held?.fits ?? Infinity, size >> 1), refused: Math.min(held?.refused ?? Infinity, size) });
      const half = plan.length >> 1;
      await readBatch(client, source, spec, plan.slice(0, half), signal, onChunk, recording, scope);
      if (!signal.aborted) await readBatch(client, source, spec, plan.slice(half), signal, onChunk, recording, scope);
      return;
    }
    const retryable = transientError(error);
    for (const [hour, tags] of plan) onChunk(hour, tags, null, retryable, undefined, asFailure(error));
    return;
  }
  if (signal.aborted) return;
  const held = replySize(client, source.sourceId, scope);
  if (held) {
    const size = (spec.size ?? pairs)(plan);
    setReplySize(client, source.sourceId, scope, size < held.refused ? { fits: Math.max(held.fits, size), refused: held.refused } : { fits: size, refused: Infinity });
  }
  let found: ReadonlyMap<number, ReadonlyMap<string, V>> | null = null;
  let failure: Failure | undefined;
  try {
    found = spec.extract(data, plan);
  } catch (error) { failure = asFailure(error); }
  for (const [hour, tags] of plan) {
    const values = found?.get(hour) ?? null;
    onChunk(hour, tags, values, false, undefined, values ? undefined : failure ?? new Error('Fact reply lacks the requested hour'));
  }
}

async function readChunk<V>(client: DashboardClient, source: FactSource, spec: FactSpec<V, FactsFound<V>>, hour: number, tags: readonly string[],
  signal: AbortSignal, onChunk: Parameters<FactRead<V>>[2], recording?: DashboardSource): Promise<void> {
  const size = Math.max(1, spec.perRequest);
  const pieces: [number, string[]][] = [];
  for (let start = 0; start < tags.length; start += size) pieces.push([hour, tags.slice(start, start + size)]);
  const found = new Map<string, V>();
  const failed = new Map<string, FactFailure>();
  await readFacts(client, source, spec, pieces, signal, (_hour, part, values, again = true, partly, error = FAILED_FACT) => {
    if (values) {
      for (const [tag, value] of values) found.set(tag, value);
      for (const [tag, retry] of partly ?? []) failed.set(tag, retry);
    } else {
      const failure = { retryable: again, error };
      for (const tag of part) failed.set(tag, failure);
    }
  }, recording);
  if (signal.aborted) return;
  const alike = new Set(failed.values());
  if (!failed.size) onChunk(hour, tags, found);
  else if (failed.size === tags.length && alike.size === 1) onChunk(hour, tags, null, [...alike][0].retryable, undefined, [...alike][0].error);
  else onChunk(hour, tags, found, undefined, failed);
}

export async function readFacts<V>(client: DashboardClient, source: FactSource, spec: FactSpec<V, FactsFound<V>>,
  plan: FactPlan, signal: AbortSignal, onChunk: Parameters<FactRead<V>>[2], recording?: DashboardSource): Promise<void> {
  const held = (hour: number, tags: readonly string[]) => {
    if (!spec.held || !recording) return undefined;
    const timeline = askTimeline(client, source, recording, hour);
    return timeline && spec.held(timeline, hour, tags);
  };
  if (!spec.shared) {
    for (const [hour, tags] of plan) {
      const known = held(hour, tags);
      if (known) { onChunk(hour, tags, known); continue; }
      let found: ReadonlyMap<string, V> | null = null;
      let retryable = false;
      let failure: Failure | undefined;
      try {
        const params = spec.params(source.branchId, hour, tags);
        const data = await loadTransientProduct<unknown>(client, forwardRequest({ id: source.sourceId, revision: source.revision }, spec.product, labelParams(params)), signal, recording);
        found = await spec.extract(data, params, source);
      } catch (error) {
        retryable = transientError(error);
        failure = asFailure(error);
      }
      if (signal.aborted) return;
      onChunk(hour, tags, found, retryable, undefined, failure);
    }
    return;
  }
  const scope = JSON.stringify([source.revision, source.branchId, source.content ?? null, !!recording]);
  const facts = sharedFacts<V>(client, source.sourceId, scope, spec, () => async (hour, tags, asked) => {
    try {
      const params = spec.params(source.branchId, hour, tags);
      const data = await loadTransientProduct<unknown>(client, forwardRequest({ id: source.sourceId, revision: source.revision }, spec.product, labelParams(params)), asked, recording);
      return await spec.extract(data, params, source);
    } catch (error) {
      return { failed: { retryable: transientError(error), error: asFailure(error) } };
    }
  });
  for (const [hour, tags] of plan) {
    const known = held(hour, tags);
    if (known) { onChunk(hour, tags, known); continue; }
    const read = await facts.read(hour, tags, signal);
    if (!read || signal.aborted) return;
    const alike = new Set(read.failed.values());
    if (!read.failed.size) onChunk(hour, tags, read.found);
    else if (read.failed.size === tags.length && alike.size === 1) onChunk(hour, tags, null, [...alike][0].retryable, undefined, [...alike][0].error);
    else onChunk(hour, tags, read.found, undefined, read.failed);
  }
}
