import type { DashboardClient } from './client';
import type { Failure } from './queryError';
import { PerClient } from './prepare.ts';

export interface FactFailure { error: Failure; retryable: boolean }

export type Told<V> = ReadonlyMap<string, V> | { failed: FactFailure };

export type Ask<V> = (hour: number, tags: readonly string[], signal: AbortSignal) => Promise<Told<V>>;

export interface SharedRead<V> { found: Map<string, V>; failed: Map<string, FactFailure> }

interface Asking<V> {
  readonly tags: readonly string[];
  readonly controller: AbortController;
  told: Promise<Told<V>>;
  waiting: number;
  settled: boolean;
}

type Entry<V> = { value: V | undefined } | Asking<V>;
const asking = <V>(entry: Entry<V>): entry is Asking<V> => 'controller' in entry;

export const SHARED_FACT_HOURS = 64;

export class SharedFacts<V> {
  private readonly hours = new Map<number, Map<string, Entry<V>>>();
  private readonly ask: Ask<V>;
  constructor(ask: Ask<V>) { this.ask = ask; }

  async read(hour: number, tags: readonly string[], signal: AbortSignal): Promise<SharedRead<V> | null> {
    if (signal.aborted) return null;
    const known = this.hours.get(hour) ?? new Map<string, Entry<V>>();
    this.hours.delete(hour);
    this.hours.set(hour, known);
    const missing = tags.filter(tag => !known.has(tag));
    if (missing.length) this.start(hour, known, missing);
    const waits = new Map<string, Asking<V>>();
    const told = new Map<string, V | undefined>();
    for (const tag of tags) {
      const entry = known.get(tag)!;
      if (asking(entry)) waits.set(tag, entry);
      else told.set(tag, entry.value);
    }
    this.evict();
    const askings = [...new Set(waits.values())];
    for (const each of askings) each.waiting++;
    let results: Map<Asking<V>, Told<V>> | null;
    try {
      results = await untilAborted(Promise.all(askings.map(async each => [each, await each.told] as const))
        .then(pairs => new Map(pairs)), signal);
    } finally {
      for (const each of askings) {
        if (--each.waiting === 0 && !each.settled) this.abandon(hour, each);
      }
    }
    if (!results || signal.aborted) return null;
    const found = new Map<string, V>(), failed = new Map<string, FactFailure>();
    for (const tag of tags) {
      const each = waits.get(tag);
      const result = each ? results.get(each)! : undefined;
      if (result && 'failed' in result) { failed.set(tag, result.failed); continue; }
      const value = result ? result.get(tag) : told.get(tag);
      if (value !== undefined) found.set(tag, value);
    }
    return { found, failed };
  }

  private start(hour: number, known: Map<string, Entry<V>>, tags: readonly string[]): void {
    const controller = new AbortController();
    const each: Asking<V> = { tags, controller, told: this.ask(hour, tags, controller.signal), waiting: 0, settled: false };
    each.told = each.told.then(result => {
      each.settled = true;
      const held = this.hours.get(hour);
      for (const tag of tags) {
        if (held?.get(tag) !== each) continue;
        if ('failed' in result) held.delete(tag);
        else held.set(tag, { value: result.get(tag) });
      }
      this.evict();
      return result;
    });
    for (const tag of tags) known.set(tag, each);
  }

  private abandon(hour: number, each: Asking<V>): void {
    const held = this.hours.get(hour);
    for (const tag of each.tags) if (held?.get(tag) === each) held.delete(tag);
    each.controller.abort();
  }

  private evict(): void {
    if (this.hours.size <= SHARED_FACT_HOURS) return;
    for (const [hour, known] of this.hours) {
      if (this.hours.size <= SHARED_FACT_HOURS) break;
      if (![...known.values()].some(asking)) this.hours.delete(hour);
    }
  }
}

function untilAborted<T>(promise: Promise<T>, signal: AbortSignal): Promise<T | null> {
  if (signal.aborted) return Promise.resolve(null);
  return new Promise(resolve => {
    const stop = () => resolve(null);
    signal.addEventListener('abort', stop, { once: true });
    void promise.then(value => { signal.removeEventListener('abort', stop); resolve(value); });
  });
}

const shared = new PerClient<Map<string, WeakMap<object, Map<string, SharedFacts<unknown>>>>>();
const KEPT_SCOPES = 4;

export function sharedFacts<V>(client: DashboardClient, sourceId: string, scope: string, spec: object, ask: () => Ask<V>): SharedFacts<V> {
  let sources = shared.get(client);
  if (!sources) shared.set(client, sources = new Map());
  let specs = sources.get(sourceId);
  if (!specs) sources.set(sourceId, specs = new WeakMap());
  let scopes = specs.get(spec);
  if (!scopes) specs.set(spec, scopes = new Map());
  let facts = scopes.get(scope) as SharedFacts<V> | undefined;
  if (facts) scopes.delete(scope);
  else facts = new SharedFacts<V>(ask());
  scopes.set(scope, facts as SharedFacts<unknown>);
  for (const old of scopes.keys()) {
    if (scopes.size <= KEPT_SCOPES) break;
    scopes.delete(old);
  }
  return facts;
}

export function forgetFacts(client: DashboardClient, sourceId: string): void {
  shared.get(client)?.delete(sourceId);
}
