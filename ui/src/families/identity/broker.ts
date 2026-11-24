import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import { countryFromKey } from '../../dashboard/countryKey.ts';
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
  LeaderNameRunsReply,
  NameRun,
  PlaceNameRunsReply,
  StrategicRegionsReply,
} from '../../generated/dashboard';
import { busyWait } from '../../shell/busy.ts';
import { productArt } from '../art/types.ts';
import { BRANCH_READS, IdentityIndex, drawnFlagKey, type BranchRead } from './index.ts';
import {
  identityKeyString,
  identityScopeKey,
  type CountryIdentityKey,
  type DrawnFlagEntry,
  type EntityIdentityKey,
  type EntityRunEntry,
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
  loadRegions(scope: IdentityScope, signal: AbortSignal, recording?: DashboardSource): Promise<StrategicRegionsReply>;
  loadPlaceNameRuns(scope: IdentityScope, from: number, to: number, states: readonly number[], provinces: readonly number[], signal: AbortSignal, recording?: DashboardSource): Promise<PlaceNameRunsReply>;
  loadLeaderNameRuns(scope: IdentityScope, owner: string, from: number, to: number, references: readonly string[], signal: AbortSignal, recording?: DashboardSource): Promise<LeaderNameRunsReply>;
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
    loadRegions: (scope, signal, recording) => loadProduct<StrategicRegionsReply>(
      client,
      productRequest({ sourceId: scope.sourceId, revision: scope.revision, product: 'strategic-regions', params: { branchId: scope.branchId } }),
      signal,
      undefined,
      recording,
    ),
    loadPlaceNameRuns: (scope, from, to, states, provinces, signal, recording) => loadTransientProduct<PlaceNameRunsReply>(
      client,
      productRequest({ sourceId: scope.sourceId, revision: scope.revision, product: 'place-name-runs', params: {
        branchId: scope.branchId,
        land: false,
        from: from as GatedHour,
        to: to as GatedHour,
        states: [...states],
        provinces: [...provinces],
      } }),
      signal,
      recording,
    ),
    loadLeaderNameRuns: (scope, owner, from, to, references, signal, recording) => loadTransientProduct<LeaderNameRunsReply>(
      client,
      productRequest({ sourceId: scope.sourceId, revision: scope.revision, product: 'leader-name-runs', params: {
        branchId: scope.branchId,
        owner: countryFromKey(owner),
        references: [...references],
        from: from as GatedHour,
        to: to as GatedHour,
      } }),
      signal,
      recording,
    ),
  };
}

export const STYLE_BATCH_MAX = 256;
export const STYLE_MAX_CONCURRENT = 2;
export const FLAG_BATCH_MAX = 32;
export const FLAG_MAX_CONCURRENT = 2;
export const ENTITY_MAX_CONCURRENT = 2;

function drawn(entry: DrawnFlagEntry | undefined): boolean {
  return entry !== undefined && entry.failed === undefined;
}

const REGION_ABSENT = 'Not a strategic region of this recording';
const LEADER_ABSENT = 'Not a leader of this country at this hour';
const PLACE_ABSENT = 'No such place at this hour';
const PLACE_NOT_ANSWERED = 'Place name not answered';
const LEADER_NOT_ANSWERED = 'Leader not answered';
const PRESENCE_UNKNOWN = 'Not known at this hour';

function runEntry(run: NameRun, absent: string, warnings: readonly string[]): EntityRunEntry {
  switch (run.presence) {
    case 'absent':
      return { firstHour: run.firstHour, lastHour: run.lastHour, absent: true, reason: run.missing ?? absent, name: null };
    case 'unknown': {
      const reason = run.missing ?? PRESENCE_UNKNOWN;
      return { firstHour: run.firstHour, lastHour: run.lastHour, name: null, nameReason: reason, holderReason: reason, warnings };
    }
    case 'present':
      return { firstHour: run.firstHour, lastHour: run.lastHour, name: run.name, ...(run.missing ? { nameReason: run.missing } : {}), warnings };
  }
}

type EntityRequest =
  | { readonly kind: 'regions'; readonly id: string }
  | { readonly kind: 'places'; readonly id: string; readonly from: number; readonly to: number }
  | { readonly kind: 'leaders'; readonly id: string; readonly owner: string; readonly from: number; readonly to: number };

interface WantedEntities {
  readonly request: EntityRequest;
  readonly keys: Map<string, EntityIdentityKey>;
}

interface EntityTask {
  readonly request: EntityRequest;
  readonly keys: ReadonlyMap<string, EntityIdentityKey>;
  readonly controller: AbortController;
}

const ENTITY_WINDOW_HOURS = 720;

function windowOf(at: number, bounds: HourInterval): { readonly start: number; readonly from: number; readonly to: number } {
  const start = Math.floor(at / ENTITY_WINDOW_HOURS) * ENTITY_WINDOW_HOURS;
  return { start, from: Math.max(start, bounds[0]), to: Math.min(start + ENTITY_WINDOW_HOURS - 1, bounds[1]) };
}

function entityRequestOf(key: EntityIdentityKey, at: number, bounds: HourInterval): EntityRequest | null {
  switch (key.kind) {
    case 'region':
      return { kind: 'regions', id: 'regions' };
    case 'design':
      return null;
    case 'state':
    case 'province':
    {
      const { start, from, to } = windowOf(at, bounds);
      return { kind: 'places', id: `places\u0000${start}`, from, to };
    }
    case 'leader': {
      const { start, from, to } = windowOf(at, bounds);
      return { kind: 'leaders', id: `leaders\u0000${key.owner}\u0000${start}`, owner: key.owner, from, to };
    }
  }
}

function numericId(id: number | string): number | null {
  const value = typeof id === 'number' ? id : Number(id);
  return Number.isSafeInteger(value) ? value : null;
}


interface DemandEntry {
  readonly asks: readonly IdentityAsk[];
  readonly flagSize?: CountryFlagSize;
  readonly needFlags?: boolean;
  readonly needIdentities?: boolean;
  readonly needGroups?: boolean;
}

interface DemandWants {
  readonly revision: number;
  readonly facts: boolean;
  readonly identities: boolean;
  readonly groups: boolean;
  readonly styles: readonly (readonly [number, string])[];
  readonly flags: readonly (readonly [CountryFlagSize, string, CountryFlagChoice])[];
  readonly entities: readonly (readonly [EntityRequest, string, EntityIdentityKey])[];
  readonly windows: ReadonlySet<string>;
}

interface LoadedWindow {
  readonly request: Extract<EntityRequest, { readonly from: number }>;
  readonly keys: Map<string, EntityIdentityKey>;
  used: number;
}

export const MAX_ENTITY_WINDOWS = 16;

interface BranchReadState {
  controller: AbortController | null;
  retryTimer: (() => void) | null;
  failures: number;
}

function idleRead(): BranchReadState {
  return { controller: null, retryTimer: null, failures: 0 };
}

interface StyleTask {
  readonly id: number;
  readonly hour: number;
  readonly tags: readonly string[];
  readonly controller: AbortController;
}

interface FlagFailure {
  readonly count: number;
  readonly at: number;
  readonly error: Failure;
  readonly size: CountryFlagSize;
  readonly choice: CountryFlagChoice;
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
  readonly wants: WeakMap<DemandEntry, DemandWants>;

  readonly reads: Record<BranchRead, BranchReadState>;

  readonly styleRunning: Map<number, StyleTask>;
  readonly styleInflightKeys: Set<string>;
  readonly styleRetryAt: Map<string, { readonly count: number; readonly at: number; readonly error: Failure }>;
  styleRetryTimer: (() => void) | null;

  readonly flagRunning: Map<number, FlagTask>;
  readonly flagInflightKeys: Set<string>;
  readonly flagFailures: Map<string, FlagFailure>;
  readonly flagRequests: Map<CountryFlagSize, Map<string, CountryFlagChoice>>;
  readonly flagOutstanding: Map<CountryFlagSize, Map<string, CountryFlagChoice>>;
  flagWantsStamp: readonly DemandWants[];
  flagRetryTimer: (() => void) | null;

  readonly entityRunning: Map<string, EntityTask>;
  readonly entityRetryAt: Map<string, { readonly count: number; readonly at: number }>;
  readonly entityLoaded: Set<string>;
  readonly entityWindows: Map<string, LoadedWindow>;
  windowUse: number;
  entityRetryTimer: (() => void) | null;

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
        wants: new WeakMap(),
        reads: { facts: idleRead(), identities: idleRead(), groups: idleRead() },
        styleRunning: new Map(),
        styleInflightKeys: new Set(),
        styleRetryAt: new Map(),
        styleRetryTimer: null,
        flagRunning: new Map(),
        flagInflightKeys: new Set(),
        flagFailures: new Map(),
        flagRequests: new Map(),
        flagOutstanding: new Map(),
        flagWantsStamp: [],
        flagRetryTimer: null,
        entityRunning: new Map(),
        entityRetryAt: new Map(),
        entityLoaded: new Set(),
        entityWindows: new Map(),
        windowUse: 0,
        entityRetryTimer: null,
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
    return state.index.getDrawnFlag(size, choice);
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
    let requested = state.flagRequests.get(size);
    if (!requested) state.flagRequests.set(size, requested = new Map());
    let outstanding = state.flagOutstanding.get(size);
    if (!outstanding) state.flagOutstanding.set(size, outstanding = new Map());
    let added = false;
    for (const choice of choices) {
      if (drawn(state.index.getDrawnFlag(size, choice))) continue;
      const key = drawnFlagKey(size, choice);
      if (requested.has(key)) continue;
      requested.set(key, choice);
      outstanding.set(key, choice);
      added = true;
    }
    if (added) this.reconcile(state);
  }

  retry(scope: IdentityScope): void {
    const state = this.scopes.get(identityScopeKey(scope));
    if (!state) return;
    for (const read of BRANCH_READS) {
      const slot = state.reads[read];
      slot.retryTimer?.();
      slot.retryTimer = null;
      slot.failures = 0;
      if (state.index.readFailed(read)) state.index.setReadFailure(read, undefined);
    }
    state.styleRetryTimer?.();
    state.styleRetryTimer = null;
    for (const pairKey of state.styleRetryAt.keys()) {
      const sep = pairKey.indexOf('\u0000');
      state.index.clearStyleFailure(pairKey.slice(sep + 1), Number(pairKey.slice(0, sep)));
    }
    state.styleRetryAt.clear();
    state.flagRetryTimer?.();
    state.flagRetryTimer = null;
    for (const failure of state.flagFailures.values()) state.index.forgetDrawnFlag(failure.size, failure.choice);
    state.flagFailures.clear();
    state.entityRetryTimer?.();
    state.entityRetryTimer = null;
    state.entityRetryAt.clear();
    state.index.clearEntityFailures();
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
    for (const read of BRANCH_READS) {
      const slot = state.reads[read];
      slot.controller?.abort();
      slot.controller = null;
      slot.retryTimer?.();
      slot.retryTimer = null;
    }
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
    state.entityRetryTimer?.();
    state.entityRetryTimer = null;
    for (const task of state.entityRunning.values()) task.controller.abort();
    state.entityRunning.clear();
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
    const wantedEntities = new Map<string, WantedEntities>();
    const allWants = [...state.demands.values()].map(demand => this.wantsOf(state, demand));
    const flagsMoved = allWants.length !== state.flagWantsStamp.length || allWants.some((wants, at) => wants !== state.flagWantsStamp[at]);
    if (flagsMoved) {
      state.flagWantsStamp = allWants;
      state.flagOutstanding.clear();
      const add = (size: CountryFlagSize, key: string, choice: CountryFlagChoice) => {
        if (drawn(state.index.getDrawnFlag(size, choice))) return;
        let map = state.flagOutstanding.get(size);
        if (!map) state.flagOutstanding.set(size, map = new Map());
        map.set(key, choice);
      };
      for (const wants of allWants) for (const [size, flagKey, choice] of wants.flags) add(size, flagKey, choice);
      for (const [size, requested] of state.flagRequests) {
        for (const [key, choice] of requested) {
          if (drawn(state.index.getDrawnFlag(size, choice))) requested.delete(key);
          else add(size, key, choice);
        }
      }
    }

    for (const wants of allWants) {
      if (wants.facts) wantFacts = true;
      if (wants.identities) wantIdentities = true;
      if (wants.groups) wantGroups = true;
      for (const [at, tag] of wants.styles) {
        wantedStyleKeys.add(`${at}\u0000${tag}`);
        let set = neededStylesByHour.get(at);
        if (!set) neededStylesByHour.set(at, set = new Set());
        set.add(tag);
      }
      for (const [request, id, key] of wants.entities) {
        let wanted = wantedEntities.get(request.id);
        if (!wanted) wantedEntities.set(request.id, wanted = { request, keys: new Map() });
        wanted.keys.set(id, key);
      }
    }

    if (wantFacts) this.ensureRead(state, 'facts');
    if (wantIdentities) this.ensureRead(state, 'identities');
    if (wantGroups) this.ensureRead(state, 'groups');

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

    for (const [size, outstanding] of state.flagOutstanding) {
      if (outstanding.size) this.pumpFlags(state, size, outstanding);
    }

    this.pumpEntities(state, wantedEntities);
  }

  private pumpEntities(state: ScopeState, wanted: Map<string, WantedEntities>): void {
    for (const [id, task] of [...state.entityRunning]) {
      if (wanted.has(id) || task.request.kind === 'regions') continue;
      task.controller.abort();
      state.entityRunning.delete(id);
    }
    let settled = false;
    for (const [id, item] of [...wanted]) {
      if (!state.entityLoaded.has(id)) continue;
      this.settleUnanswered(state, item.request, item.keys.values());
      settled = true;
      wanted.delete(id);
    }
    if (settled) this.notify(state);
    const now = this.clock.now();
    let soonestRetry = Infinity;
    for (const [id, item] of wanted) {
      if (state.entityRunning.size >= ENTITY_MAX_CONCURRENT) break;
      if (state.entityRunning.has(id)) continue;
      const failed = state.entityRetryAt.get(id);
      if (failed && failed.at > now) {
        soonestRetry = Math.min(soonestRetry, failed.at);
        continue;
      }
      this.startEntityTask(state, item.request, item.keys);
    }
    if (soonestRetry < Infinity && !state.entityRetryTimer) {
      state.entityRetryTimer = this.clock.after(Math.max(0, soonestRetry - now), () => {
        state.entityRetryTimer = null;
        this.reconcile(state);
      });
    }
  }

  private settleUnanswered(state: ScopeState, request: EntityRequest, keys: Iterable<EntityIdentityKey>): void {
    const bounds = state.index.bounds();
    for (const key of keys) {
      switch (request.kind) {
        case 'regions':
          if (state.index.resolve(key, bounds[0]).kind === 'pending') {
            state.index.setEntityRuns(key, [{ firstHour: bounds[0], lastHour: bounds[1], absent: true, reason: REGION_ABSENT, name: null }]);
          }
          break;
        case 'places':
          if (state.index.resolve(key, request.from).kind === 'pending') {
            state.index.replaceEntityRuns(key, request.from, request.to, [{ firstHour: request.from, lastHour: request.to, name: null, nameReason: PLACE_NOT_ANSWERED, holderReason: PLACE_NOT_ANSWERED }]);
          }
          break;
        case 'leaders':
          if (state.index.resolve(key, request.from).kind === 'pending') {
            state.index.replaceEntityRuns(key, request.from, request.to, [{ firstHour: request.from, lastHour: request.to, name: null, nameReason: LEADER_NOT_ANSWERED, holderReason: LEADER_NOT_ANSWERED }]);
          }
          break;
      }
    }
  }

  private loadEntityRequest(state: ScopeState, request: EntityRequest, keys: ReadonlyMap<string, EntityIdentityKey>, signal: AbortSignal): Promise<void> {
    const index = state.index;
    switch (request.kind) {
      case 'regions':
        return this.transport.loadRegions(state.scope, signal, state.recording).then(reply => {
          const bounds = index.bounds();
          for (const region of reply.regions) {
            index.setEntityRuns({ kind: 'region', id: region.id }, [{
              firstHour: bounds[0],
              lastHour: bounds[1],
              name: region.name,
              ...(region.nameMissing ? { nameReason: region.nameMissing } : {}),
              warnings: reply.warnings,
            }]);
          }
        });
      case 'places': {
        const states: number[] = [];
        const provinces: number[] = [];
        for (const key of keys.values()) {
          if (key.kind !== 'state' && key.kind !== 'province') continue;
          const id = numericId(key.id);
          if (id !== null) (key.kind === 'state' ? states : provinces).push(id);
        }
        return this.transport.loadPlaceNameRuns(state.scope, request.from, request.to, states, provinces, signal, state.recording).then(reply => {
          const entry = (run: NameRun) => runEntry(run, PLACE_ABSENT, reply.warnings);
          index.replaceEntityUnknown(['state', 'province'], reply.from, reply.to, reply.unknown);
          for (const place of reply.states) index.replaceEntityRuns({ kind: 'state', id: place.id }, reply.from, reply.to, place.runs.map(entry));
          for (const place of reply.provinces) index.replaceEntityRuns({ kind: 'province', id: place.id }, reply.from, reply.to, place.runs.map(entry));
        });
      }
      case 'leaders':
      {
        const references: string[] = [];
        for (const key of keys.values()) if (key.kind === 'leader') references.push(String(key.id));
        return this.transport.loadLeaderNameRuns(state.scope, request.owner, request.from, request.to, references, signal, state.recording).then(reply => {
          const entry = (run: NameRun) => runEntry(run, LEADER_ABSENT, reply.warnings);
          index.replaceEntityUnknown(['leader'], reply.from, reply.to, reply.unknown);
          for (const leader of reply.leaders) {
            index.replaceEntityRuns({ kind: 'leader', owner: request.owner, id: leader.reference }, reply.from, reply.to, leader.runs.map(entry));
          }
        });
      }
    }
  }

  private startEntityTask(state: ScopeState, request: EntityRequest, keys: ReadonlyMap<string, EntityIdentityKey>): void {
    const controller = new AbortController();
    const task: EntityTask = { request, keys, controller };
    state.entityRunning.set(request.id, task);
    void this.loadEntityRequest(state, request, keys, controller.signal).then(
      () => {
        if (state.entityRunning.get(request.id) !== task || controller.signal.aborted) return;
        state.entityRunning.delete(request.id);
        state.entityRetryAt.delete(request.id);
        if (request.kind === 'regions') state.entityLoaded.add(request.id);
        else this.keepWindow(state, request, keys);
        this.settleUnanswered(state, request, keys.values());
        this.notify(state);
        this.reconcile(state);
      },
      error => {
        if (state.entityRunning.get(request.id) !== task || controller.signal.aborted) return;
        state.entityRunning.delete(request.id);
        const failure = asFailure(error);
        const count = (state.entityRetryAt.get(request.id)?.count ?? 0) + 1;
        state.entityRetryAt.set(request.id, { count, at: transientError(error) ? this.clock.now() + retryDelay(count) : Infinity });
        const [from, to] = request.kind === 'regions' ? state.index.bounds() : [request.from, request.to];
        for (const key of keys.values()) state.index.setEntityFailure(key, from, to, failure);
        this.notify(state);
        this.reconcile(state);
      },
    );
  }

  private keepWindow(state: ScopeState, request: Extract<EntityRequest, { readonly from: number }>, keys: ReadonlyMap<string, EntityIdentityKey>): void {
    let held = state.entityWindows.get(request.id);
    if (!held) state.entityWindows.set(request.id, held = { request, keys: new Map(), used: 0 });
    for (const [id, key] of keys) held.keys.set(id, key);
    held.used = ++state.windowUse;
    if (state.entityWindows.size <= MAX_ENTITY_WINDOWS) return;
    const pinned = new Set<string>();
    for (const demand of state.demands.values()) for (const id of this.wantsOf(state, demand).windows) pinned.add(id);
    const oldest = [...state.entityWindows.values()].filter(window => !pinned.has(window.request.id)).sort((a, b) => a.used - b.used);
    for (const window of oldest) {
      if (state.entityWindows.size <= MAX_ENTITY_WINDOWS) break;
      state.entityWindows.delete(window.request.id);
      state.index.dropEntityRuns(window.keys.values(), window.request.from, window.request.to);
    }
  }

  private wantsOf(state: ScopeState, demand: DemandEntry): DemandWants {
    const revision = state.index.resolveRevision();
    const held = state.wants.get(demand);
    if (held && held.revision === revision) return held;
    let facts = false;
    let identities = demand.needIdentities ?? false;
    let groups = demand.needGroups ?? false;
    const styles: [number, string][] = [];
    const flags: [CountryFlagSize, string, CountryFlagChoice][] = [];
    const entities: [EntityRequest, string, EntityIdentityKey][] = [];
    const windows = new Set<string>();
    const size: CountryFlagSize = demand.flagSize ?? 'panel';
    for (const ask of demand.asks) {
      const at = Math.floor(ask.hour);
      if (!Number.isFinite(at)) continue;
      if (ask.key.kind === 'country') {
        facts = true;
        identities = true;
        const resolved = state.index.resolve(ask.key, at);
        if (resolved.kind === 'pending') {
          const tag = this.tagForAsk(state, ask.key, at);
          if (tag !== null && this.enableStyleFallback && !state.index.hasFactsAt(at)) styles.push([at, tag]);
        } else if (resolved.kind === 'known' && demand.needFlags && resolved.flagChoice.kind === 'known') {
          flags.push([size, drawnFlagKey(size, resolved.flagChoice.value), resolved.flagChoice.value]);
        }
      } else if (ask.key.kind === 'faction') {
        groups = true;
        facts = true;
      } else {
        const request = entityRequestOf(ask.key, at, state.index.bounds());
        if (!request) continue;
        windows.add(request.id);
        const answered = state.index.resolve(ask.key, at).kind;
        if (answered === 'pending' || answered === 'failed') entities.push([request, identityKeyString(ask.key), ask.key]);
      }
    }
    const wants: DemandWants = { revision, facts, identities, groups, styles, flags, entities, windows };
    state.wants.set(demand, wants);
    return wants;
  }

  private tagForAsk(state: ScopeState, key: CountryIdentityKey, at: number): string | null {
    if (key.by === 'tag') return key.tag;
    const held = state.index.slotHolderAt(key.slot, at);
    if (!held) return null;
    if (key.by === 'holder' && held.countryId !== key.countryId) return null;
    return held.tag;
  }

  private ensureRead(state: ScopeState, read: BranchRead): void {
    const slot = state.reads[read];
    if (state.index.hasRead(read) || state.index.readFailed(read) || slot.controller || slot.retryTimer) return;
    const controller = new AbortController();
    slot.controller = controller;
    void this.loadRead(state, read, controller.signal).then(
      apply => {
        if (slot.controller !== controller || controller.signal.aborted) return;
        slot.controller = null;
        slot.failures = 0;
        apply();
        this.notify(state);
        this.reconcile(state);
      },
      error => {
        if (slot.controller !== controller || controller.signal.aborted) return;
        slot.controller = null;
        slot.failures++;
        state.index.setReadFailure(read, asFailure(error));
        if (transientError(error)) {
          slot.retryTimer = this.clock.after(retryDelay(slot.failures), () => {
            slot.retryTimer = null;
            state.index.setReadFailure(read, undefined);
            this.reconcile(state);
          });
        }
        this.notify(state);
      },
    );
  }

  private loadRead(state: ScopeState, read: BranchRead, signal: AbortSignal): Promise<() => void> {
    switch (read) {
      case 'facts':
        return this.transport.loadFacts(state.scope, signal, state.recording).then(reply => () => {
          state.index.setFacts(reply, true);
          for (const [id, task] of [...state.styleRunning]) {
            task.controller.abort();
            state.styleRunning.delete(id);
            for (const tag of task.tags) state.styleInflightKeys.delete(`${task.hour}\u0000${tag}`);
          }
        });
      case 'identities':
        return this.transport.loadIdentities(state.scope, signal, state.recording).then(reply => () => state.index.setIdentities(reply));
      case 'groups':
        return this.transport.loadGroups(state.scope, signal, state.recording).then(reply => () => state.index.setGroups(reply));
    }
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

  private pumpFlags(state: ScopeState, size: CountryFlagSize, outstanding: Map<string, CountryFlagChoice>): void {
    const now = this.clock.now();
    let soonestRetry = Infinity;
    const room = (FLAG_MAX_CONCURRENT - state.flagRunning.size) * FLAG_BATCH_MAX;
    const ready: [string, CountryFlagChoice][] = [];
    for (const [key, choice] of outstanding) {
      if (ready.length >= room) break;
      if (state.flagInflightKeys.has(key)) continue;
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
        for (const info of state.flagFailures.values()) {
          if (info.at <= this.clock.now()) state.index.forgetDrawnFlag(info.size, info.choice);
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
        const unavailable = new Map(reply.unavailable.map(item => [item.flag, item.resolution]));
        const outstanding = state.flagOutstanding.get(size);
        batch.forEach(([key, choice], index) => {
          state.flagInflightKeys.delete(key);
          state.flagFailures.delete(key);
          outstanding?.delete(key);
          state.flagRequests.get(size)?.delete(key);
          const image = images.get(index);
          const resolution = unavailable.get(index);
          const entry: DrawnFlagEntry = image
            ? { url: image.png, image }
            : resolution
              ? { resolution: productArt(resolution) }
              : { resolution: { status: 'failed', reason: 'The flag reply left this choice out', retryable: true } };
          state.index.setDrawnFlag(size, choice, entry);
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
          state.flagFailures.set(key, { count, at: retryable ? now + retryDelay(count) : Infinity, error: failure, size, choice });
          state.index.setDrawnFlag(size, choice, { failed: failure });
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
