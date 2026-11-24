import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import { validateCountryFacts } from '../../dashboard/countryTimeline.ts';
import { transientError, retryDelay } from '../../dashboard/countryFacts.ts';
import { loadProduct, loadTransientProduct } from '../../dashboard/products.ts';
import { PerClient } from '../../dashboard/prepare.ts';
import { asFailure, type Failure } from '../../dashboard/queryError.ts';
import type { GatedHour } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request.ts';
import type {
  CountryFactsReply,
  CountryFlagChoice,
  CountryFlagReply,
  CountryFlagSize,
  CountryGroupReply,
  CountryIdentitiesReply,
  CountryStyleReply,
} from '../../generated/dashboard';
import { busyWait } from '../../shell/busy.ts';
import { IdentityIndex } from './index.ts';
import {
  flagChoiceKey,
  identityScopeKey,
  type CountryIdentityKey,
  type DrawnFlagEntry,
  type HourInterval,
  type IdentityAsk,
  type IdentityKey,
  type IdentityResult,
  type IdentityScope,
  type IdentitySnapshot,
} from './types.ts';

export interface IdentityTransport {
  loadFacts(scope: IdentityScope, signal: AbortSignal, recording?: DashboardSource): Promise<CountryFactsReply>;
  loadIdentities(scope: IdentityScope, signal: AbortSignal, recording?: DashboardSource): Promise<CountryIdentitiesReply>;
  loadGroups(scope: IdentityScope, signal: AbortSignal, recording?: DashboardSource): Promise<CountryGroupReply>;
  loadStyles(scope: IdentityScope, hour: number, tags: readonly string[], signal: AbortSignal, recording?: DashboardSource): Promise<CountryStyleReply>;
  loadFlags(scope: IdentityScope, size: CountryFlagSize, choices: readonly CountryFlagChoice[], signal: AbortSignal, recording?: DashboardSource): Promise<CountryFlagReply>;
}

export function clientIdentityTransport(client: DashboardClient): IdentityTransport {
  return {
    loadFacts: async (scope, signal, recording) => validateCountryFacts(
      await loadTransientProduct<unknown>(
        client,
        productRequest({ sourceId: scope.sourceId, revision: scope.revision, product: 'country-facts', params: { branchId: scope.branchId } }),
        signal,
        recording,
      ),
    ),
    loadIdentities: (scope, signal, recording) => loadProduct<CountryIdentitiesReply>(
      client,
      productRequest({ sourceId: scope.sourceId, revision: scope.revision, product: 'country-identities', params: { branchId: scope.branchId } }),
      signal,
      undefined,
      recording,
    ),
    loadGroups: (scope, signal, recording) => loadProduct<CountryGroupReply>(
      client,
      productRequest({ sourceId: scope.sourceId, revision: scope.revision, product: 'country-groups', params: { branchId: scope.branchId } }),
      signal,
      undefined,
      recording,
    ),
    loadStyles: (scope, hour, tags, signal, recording) => loadTransientProduct<CountryStyleReply>(
      client,
      productRequest({ sourceId: scope.sourceId, revision: scope.revision, product: 'country-styles', params: { branchId: scope.branchId, hour: hour as GatedHour, tags: [...tags] } }),
      signal,
      recording,
    ),
    loadFlags: (scope, size, choices, signal, recording) => loadTransientProduct<CountryFlagReply>(
      client,
      productRequest({ sourceId: scope.sourceId, revision: scope.revision, product: 'country-flags', params: { flags: [...choices], size } }),
      signal,
      recording,
    ),
  };
}

export const STYLE_BATCH_MAX = 256;
export const STYLE_MAX_CONCURRENT = 2;
export const FLAG_BATCH_MAX = 32;
export const FLAG_MAX_CONCURRENT = 2;

interface DemandEntry {
  readonly asks: readonly IdentityAsk[];
  readonly flagSize?: CountryFlagSize;
  readonly needFlags?: boolean;
  readonly needIdentities?: boolean;
  readonly needGroups?: boolean;
}

interface StyleTask {
  readonly id: number;
  readonly hour: number;
  readonly tags: readonly string[];
  readonly controller: AbortController;
}

interface FlagTask {
  readonly id: number;
  readonly size: CountryFlagSize;
  readonly batch: readonly (readonly [string, CountryFlagChoice])[];
  readonly controller: AbortController;
}

interface ScopeState {
  readonly key: string;
  readonly scope: IdentityScope;
  readonly index: IdentityIndex;
  recording?: DashboardSource;
  readonly watchers: Set<() => void>;
  readonly demands: Map<object, DemandEntry>;

  factsController: AbortController | null;
  factsRetryTimer: (() => void) | null;
  factsFailures: number;

  identitiesController: AbortController | null;
  groupsController: AbortController | null;

  readonly styleRunning: Map<number, StyleTask>;
  readonly styleInflightKeys: Set<string>;
  readonly styleRetryAt: Map<string, { readonly count: number; readonly at: number; readonly error: Failure }>;
  styleRetryTimer: (() => void) | null;

  readonly flagEntries: Map<string, DrawnFlagEntry>;
  readonly flagRunning: Map<number, FlagTask>;
  readonly flagInflightKeys: Set<string>;
  readonly flagFailures: Map<string, { readonly count: number; readonly at: number; readonly error: Failure }>;
  flagRetryTimer: (() => void) | null;

  nextTaskId: number;
  version: number;
  snapshotCache: { readonly version: number; readonly value: IdentitySnapshot } | null;
}

export interface BrokerClock {
  now(): number;
  after(ms: number, run: () => void): () => void;
}

const DEFAULT_CLOCK: BrokerClock = {
  now: () => (typeof performance !== 'undefined' ? performance.now() : Date.now()),
  after: (ms, run) => busyWait(run, ms),
};

export interface IdentityBrokerOptions {
  readonly transport: IdentityTransport;
  readonly clock?: BrokerClock;
  readonly enableStyleFallback?: boolean;
}

export class IdentityBroker {
  private readonly transport: IdentityTransport;
  private readonly clock: BrokerClock;
  private readonly enableStyleFallback: boolean;
  private readonly scopes = new Map<string, ScopeState>();

  constructor(options: IdentityBrokerOptions) {
    this.transport = options.transport;
    this.clock = options.clock ?? DEFAULT_CLOCK;
    this.enableStyleFallback = options.enableStyleFallback ?? true;
  }

  private getState(scope: IdentityScope, recording?: DashboardSource, branchBounds?: HourInterval): ScopeState {
    const key = identityScopeKey(scope);
    let state = this.scopes.get(key);
    if (!state) {
      state = {
        key,
        scope,
        index: new IdentityIndex(scope, branchBounds),
        recording,
        watchers: new Set(),
        demands: new Map(),
        factsController: null,
        factsRetryTimer: null,
        factsFailures: 0,
        identitiesController: null,
        groupsController: null,
        styleRunning: new Map(),
        styleInflightKeys: new Set(),
        styleRetryAt: new Map(),
        styleRetryTimer: null,
        flagEntries: new Map(),
        flagRunning: new Map(),
        flagInflightKeys: new Set(),
        flagFailures: new Map(),
        flagRetryTimer: null,
        nextTaskId: 1,
        version: 0,
        snapshotCache: null,
      };
      this.scopes.set(key, state);
    } else {
      if (recording) state.recording = recording;
      if (branchBounds) state.index.setBranchBounds(branchBounds);
    }
    return state;
  }

  indexOf(scope: IdentityScope, recording?: DashboardSource, branchBounds?: HourInterval): IdentityIndex {
    return this.getState(scope, recording, branchBounds).index;
  }

  versionOf(scope: IdentityScope): number {
    return this.scopes.get(identityScopeKey(scope))?.version ?? 0;
  }

  snapshotOf(scope: IdentityScope, recording?: DashboardSource, branchBounds?: HourInterval): IdentitySnapshot {
    const state = this.getState(scope, recording, branchBounds);
    if (state.snapshotCache && state.snapshotCache.version === state.version) {
      return state.snapshotCache.value;
    }
    const value = state.index.exportSnapshot();
    state.snapshotCache = { version: state.version, value };
    return value;
  }

  resolve<K extends IdentityKey>(scope: IdentityScope, key: K, hour: number, recording?: DashboardSource, branchBounds?: HourInterval): IdentityResult<K> {
    return this.getState(scope, recording, branchBounds).index.resolve(key, hour);
  }

  flagFor(scope: IdentityScope, size: CountryFlagSize, choice: CountryFlagChoice): DrawnFlagEntry | undefined {
    const state = this.scopes.get(identityScopeKey(scope));
    if (!state) return undefined;
    return state.flagEntries.get(`${size}\u0000${flagChoiceKey(choice)}`);
  }

  subscribe(scope: IdentityScope, listener: () => void, recording?: DashboardSource, branchBounds?: HourInterval): () => void {
    const state = this.getState(scope, recording, branchBounds);
    state.watchers.add(listener);
    return () => { state.watchers.delete(listener); };
  }

  setDemand(
    owner: object,
    scope: IdentityScope,
    demand: DemandEntry | null,
    recording?: DashboardSource,
    branchBounds?: HourInterval,
  ): void {
    const key = identityScopeKey(scope);
    const existing = this.scopes.get(key);
    if (!demand) {
      if (!existing) return;
      if (existing.demands.delete(owner)) this.reconcile(existing);
      return;
    }
    const state = this.getState(scope, recording, branchBounds);
    state.demands.set(owner, demand);
    this.reconcile(state);
  }

  requestFlagChoices(
    scope: IdentityScope,
    size: CountryFlagSize,
    choices: Iterable<CountryFlagChoice>,
    recording?: DashboardSource,
  ): void {
    const state = this.getState(scope, recording);
    const wanted = new Map<string, CountryFlagChoice>();
    for (const choice of choices) {
      const choiceId = flagChoiceKey(choice);
      const key = `${size}\u0000${choiceId}`;
      if (state.flagEntries.has(key) || state.flagInflightKeys.has(key)) continue;
      wanted.set(key, choice);
    }
    if (wanted.size) this.pumpFlags(state, size, wanted);
  }

  retry(scope: IdentityScope): void {
    const state = this.scopes.get(identityScopeKey(scope));
    if (!state) return;
    state.factsRetryTimer?.();
    state.factsRetryTimer = null;
    state.factsFailures = 0;
    state.index.clearFactsFailure();
    state.styleRetryTimer?.();
    state.styleRetryTimer = null;
    for (const pairKey of state.styleRetryAt.keys()) {
      const sep = pairKey.indexOf('\u0000');
      state.index.clearStyleFailure(pairKey.slice(sep + 1), Number(pairKey.slice(0, sep)));
    }
    state.styleRetryAt.clear();
    state.flagRetryTimer?.();
    state.flagRetryTimer = null;
    for (const key of state.flagFailures.keys()) state.flagEntries.delete(key);
    state.flagFailures.clear();
    this.notify(state);
    this.reconcile(state);
  }

  forgetSource(sourceId: string): void {
    for (const [key, state] of [...this.scopes]) {
      if (state.scope.sourceId !== sourceId) continue;
      this.abortScope(state);
      this.scopes.delete(key);
    }
  }

  flushContent(sourceId: string, activeContent: number): void {
    for (const [key, state] of [...this.scopes]) {
      if (state.scope.sourceId !== sourceId || state.scope.content === activeContent) continue;
      this.abortScope(state);
      this.scopes.delete(key);
    }
  }

  dispose(): void {
    for (const state of this.scopes.values()) this.abortScope(state);
    this.scopes.clear();
  }

  private abortScope(state: ScopeState): void {
    state.factsController?.abort();
    state.factsController = null;
    state.factsRetryTimer?.();
    state.factsRetryTimer = null;
    state.identitiesController?.abort();
    state.identitiesController = null;
    state.groupsController?.abort();
    state.groupsController = null;
    state.styleRetryTimer?.();
    state.styleRetryTimer = null;
    for (const task of state.styleRunning.values()) task.controller.abort();
    state.styleRunning.clear();
    state.styleInflightKeys.clear();
    state.flagRetryTimer?.();
    state.flagRetryTimer = null;
    for (const task of state.flagRunning.values()) task.controller.abort();
    state.flagRunning.clear();
    state.flagInflightKeys.clear();
  }

  private notify(state: ScopeState): void {
    state.version++;
    state.snapshotCache = null;
    for (const listener of [...state.watchers]) listener();
  }

  private reconcile(state: ScopeState): void {
    let wantFacts = false;
    let wantIdentities = false;
    let wantGroups = false;
    const neededStylesByHour = new Map<number, Set<string>>();
    const wantedStyleKeys = new Set<string>();
    const wantedFlagsBySize = new Map<CountryFlagSize, Map<string, CountryFlagChoice>>();

    for (const demand of state.demands.values()) {
      if (demand.needIdentities) wantIdentities = true;
      if (demand.needGroups) wantGroups = true;
      for (const ask of demand.asks) {
        const at = Math.floor(ask.hour);
        if (!Number.isFinite(at)) continue;
        if (ask.key.kind === 'country') {
          wantFacts = true;
          if (ask.key.by === 'slot' || ask.key.by === 'holder') wantIdentities = true;
          const resolved = state.index.resolve(ask.key, at);
          if (resolved.kind === 'pending') {
            const tag = this.tagForAsk(state, ask.key, at);
            if (tag !== null && this.enableStyleFallback && !state.index.hasFactsAt(at)) {
              const pairKey = `${at}\u0000${tag}`;
              wantedStyleKeys.add(pairKey);
              let set = neededStylesByHour.get(at);
              if (!set) neededStylesByHour.set(at, set = new Set());
              set.add(tag);
            }
          } else if (resolved.kind === 'known' && demand.needFlags && resolved.flagChoice.kind === 'known') {
            const size: CountryFlagSize = demand.flagSize ?? 'panel';
            const choiceId = flagChoiceKey(resolved.flagChoice.value);
            const flagKey = `${size}\u0000${choiceId}`;
            if (!state.flagEntries.has(flagKey) && !state.flagInflightKeys.has(flagKey)) {
              let map = wantedFlagsBySize.get(size);
              if (!map) wantedFlagsBySize.set(size, map = new Map());
              map.set(flagKey, resolved.flagChoice.value);
            }
          }
        } else if (ask.key.kind === 'faction') {
          wantGroups = true;
          wantFacts = true;
        }
      }
    }

    if (wantFacts && !state.index.hasFacts() && !state.factsController && !state.factsRetryTimer) {
      this.startFacts(state);
    }
    if (wantIdentities && !state.index.hasIdentities() && !state.identitiesController) {
      this.startIdentities(state);
    }
    if (wantGroups && !state.index.hasGroups() && !state.groupsController) {
      this.startGroups(state);
    }

    for (const [id, task] of [...state.styleRunning]) {
      const stillWanted = task.tags.some(tag => wantedStyleKeys.has(`${task.hour}\u0000${tag}`));
      if (!stillWanted || state.index.hasFactsAt(task.hour)) {
        task.controller.abort();
        state.styleRunning.delete(id);
        for (const tag of task.tags) state.styleInflightKeys.delete(`${task.hour}\u0000${tag}`);
      }
    }

    if (neededStylesByHour.size && !state.index.hasFacts()) {
      this.pumpStyles(state, neededStylesByHour);
    }

    for (const [size, wanted] of wantedFlagsBySize) {
      this.pumpFlags(state, size, wanted);
    }
  }

  private tagForAsk(state: ScopeState, key: CountryIdentityKey, at: number): string | null {
    if (key.by === 'tag') return key.tag;
    const held = state.index.slotHolderAt(key.slot, at);
    if (!held) return null;
    if (key.by === 'holder' && held.countryId !== key.countryId) return null;
    return held.tag;
  }

  private startFacts(state: ScopeState): void {
    const controller = new AbortController();
    state.factsController = controller;
    void this.transport.loadFacts(state.scope, controller.signal, state.recording).then(
      reply => {
        if (state.factsController !== controller || controller.signal.aborted) return;
        state.factsController = null;
        state.factsFailures = 0;
        state.index.setFacts(reply, true);
        for (const [id, task] of [...state.styleRunning]) {
          task.controller.abort();
          state.styleRunning.delete(id);
          for (const tag of task.tags) state.styleInflightKeys.delete(`${task.hour}\u0000${tag}`);
        }
        this.notify(state);
        this.reconcile(state);
      },
      error => {
        if (state.factsController !== controller || controller.signal.aborted) return;
        state.factsController = null;
        const failure = asFailure(error);
        state.factsFailures++;
        state.index.setFactsFailure(failure);
        if (transientError(error)) {
          state.factsRetryTimer = this.clock.after(retryDelay(state.factsFailures), () => {
            state.factsRetryTimer = null;
            state.index.clearFactsFailure();
            this.reconcile(state);
          });
        }
        this.notify(state);
      },
    );
  }

  private startIdentities(state: ScopeState): void {
    const controller = new AbortController();
    state.identitiesController = controller;
    void this.transport.loadIdentities(state.scope, controller.signal, state.recording).then(
      reply => {
        if (state.identitiesController !== controller || controller.signal.aborted) return;
        state.identitiesController = null;
        state.index.setIdentities(reply);
        this.notify(state);
        this.reconcile(state);
      },
      error => {
        if (state.identitiesController !== controller || controller.signal.aborted) return;
        state.identitiesController = null;
        state.index.setIdentitiesFailure(asFailure(error));
        this.notify(state);
      },
    );
  }

  private startGroups(state: ScopeState): void {
    const controller = new AbortController();
    state.groupsController = controller;
    void this.transport.loadGroups(state.scope, controller.signal, state.recording).then(
      reply => {
        if (state.groupsController !== controller || controller.signal.aborted) return;
        state.groupsController = null;
        state.index.setGroups(reply);
        this.notify(state);
        this.reconcile(state);
      },
      error => {
        if (state.groupsController !== controller || controller.signal.aborted) return;
        state.groupsController = null;
        state.index.setGroupsFailure(asFailure(error));
        this.notify(state);
      },
    );
  }

  private pumpStyles(state: ScopeState, neededByHour: Map<number, Set<string>>): void {
    const now = this.clock.now();
    let soonestRetry = Infinity;
    const hours = [...neededByHour.keys()].sort((a, b) => a - b);
    for (const hour of hours) {
      if (state.styleRunning.size >= STYLE_MAX_CONCURRENT) break;
      const tags = [...neededByHour.get(hour)!].filter(tag => {
        const pairKey = `${hour}\u0000${tag}`;
        if (state.styleInflightKeys.has(pairKey)) return false;
        const failed = state.styleRetryAt.get(pairKey);
        if (failed && failed.at > now) {
          soonestRetry = Math.min(soonestRetry, failed.at);
          return false;
        }
        return true;
      }).sort();
      for (let start = 0; start < tags.length && state.styleRunning.size < STYLE_MAX_CONCURRENT; start += STYLE_BATCH_MAX) {
        const chunk = tags.slice(start, start + STYLE_BATCH_MAX);
        this.startStyleBatch(state, hour, chunk);
      }
    }
    if (soonestRetry < Infinity && !state.styleRetryTimer) {
      state.styleRetryTimer = this.clock.after(Math.max(0, soonestRetry - now), () => {
        state.styleRetryTimer = null;
        this.reconcile(state);
      });
    }
  }

  private startStyleBatch(state: ScopeState, hour: number, tags: readonly string[]): void {
    const id = state.nextTaskId++;
    const controller = new AbortController();
    const task: StyleTask = { id, hour, tags, controller };
    state.styleRunning.set(id, task);
    for (const tag of tags) state.styleInflightKeys.add(`${hour}\u0000${tag}`);
    void this.transport.loadStyles(state.scope, hour, tags, controller.signal, state.recording).then(
      reply => {
        if (state.styleRunning.get(id) !== task || controller.signal.aborted) return;
        state.styleRunning.delete(id);
        for (const tag of tags) {
          const pairKey = `${hour}\u0000${tag}`;
          state.styleInflightKeys.delete(pairKey);
          state.styleRetryAt.delete(pairKey);
        }
        state.index.ingestStylesReply(reply);
        this.notify(state);
        this.reconcile(state);
      },
      error => {
        if (state.styleRunning.get(id) !== task || controller.signal.aborted) return;
        state.styleRunning.delete(id);
        const failure = asFailure(error);
        const retryable = transientError(error);
        const now = this.clock.now();
        for (const tag of tags) {
          const pairKey = `${hour}\u0000${tag}`;
          state.styleInflightKeys.delete(pairKey);
          const count = (state.styleRetryAt.get(pairKey)?.count ?? 0) + 1;
          state.styleRetryAt.set(pairKey, { count, at: retryable ? now + retryDelay(count) : Infinity, error: failure });
          state.index.setStyleFailure(tag, hour, failure);
        }
        this.notify(state);
        this.reconcile(state);
      },
    );
  }

  private pumpFlags(state: ScopeState, size: CountryFlagSize, wanted: Map<string, CountryFlagChoice>): void {
    const now = this.clock.now();
    let soonestRetry = Infinity;
    const ready: [string, CountryFlagChoice][] = [];
    for (const [key, choice] of wanted) {
      if (state.flagEntries.has(key) || state.flagInflightKeys.has(key)) continue;
      const failed = state.flagFailures.get(key);
      if (failed && failed.at > now) {
        soonestRetry = Math.min(soonestRetry, failed.at);
        continue;
      }
      ready.push([key, choice]);
    }
    for (let start = 0; start < ready.length && state.flagRunning.size < FLAG_MAX_CONCURRENT; start += FLAG_BATCH_MAX) {
      const batch = ready.slice(start, start + FLAG_BATCH_MAX);
      this.startFlagBatch(state, size, batch);
    }
    if (soonestRetry < Infinity && !state.flagRetryTimer) {
      state.flagRetryTimer = this.clock.after(Math.max(0, soonestRetry - now), () => {
        state.flagRetryTimer = null;
        for (const [key, info] of [...state.flagFailures]) {
          if (info.at <= this.clock.now()) state.flagEntries.delete(key);
        }
        this.reconcile(state);
      });
    }
  }

  private startFlagBatch(state: ScopeState, size: CountryFlagSize, batch: readonly (readonly [string, CountryFlagChoice])[]): void {
    const id = state.nextTaskId++;
    const controller = new AbortController();
    const task: FlagTask = { id, size, batch, controller };
    state.flagRunning.set(id, task);
    for (const [key] of batch) state.flagInflightKeys.add(key);
    void this.transport.loadFlags(state.scope, size, batch.map(([, choice]) => choice), controller.signal, state.recording).then(
      reply => {
        if (state.flagRunning.get(id) !== task || controller.signal.aborted) return;
        state.flagRunning.delete(id);
        const images = new Map(reply.flags.map(image => [image.flag, image]));
        const unavailable = new Map(reply.unavailable.map(item => [item.flag, item.reason]));
        batch.forEach(([key, choice], index) => {
          state.flagInflightKeys.delete(key);
          state.flagFailures.delete(key);
          const image = images.get(index);
          const reason = unavailable.get(index);
          const entry: DrawnFlagEntry = image
            ? { url: image.png, image }
            : { reason: reason ?? 'Flag unavailable' };
          state.flagEntries.set(key, entry);
          state.index.setDrawnFlag(choice, entry);
        });
        this.notify(state);
        this.reconcile(state);
      },
      error => {
        if (state.flagRunning.get(id) !== task || controller.signal.aborted) return;
        state.flagRunning.delete(id);
        const failure = asFailure(error);
        const retryable = transientError(error);
        const now = this.clock.now();
        for (const [key, choice] of batch) {
          state.flagInflightKeys.delete(key);
          const count = (state.flagFailures.get(key)?.count ?? 0) + 1;
          state.flagFailures.set(key, { count, at: retryable ? now + retryDelay(count) : Infinity, error: failure });
          const entry: DrawnFlagEntry = { failed: failure };
          state.flagEntries.set(key, entry);
          state.index.setDrawnFlag(choice, entry);
        }
        this.notify(state);
        this.reconcile(state);
      },
    );
  }
}

const brokers = new PerClient<IdentityBroker>();

export function identityBrokerFor(client: DashboardClient): IdentityBroker {
  let broker = brokers.get(client);
  if (!broker) {
    broker = new IdentityBroker({ transport: clientIdentityTransport(client) });
    brokers.set(client, broker);
  }
  return broker;
}

export function forgetIdentitySource(client: DashboardClient, sourceId: string): void {
  brokers.get(client)?.forgetSource(sourceId);
}
