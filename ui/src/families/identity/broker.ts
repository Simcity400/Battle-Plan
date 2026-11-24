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
  EquipmentDesignsReply,
  PlaceNamesReply,
  StrategicRegionsReply,
  UnitLeadersReply,
} from '../../generated/dashboard';
import { busyWait } from '../../shell/busy.ts';
import { IdentityIndex, drawnFlagKey } from './index.ts';
import {
  identityKeyString,
  identityScopeKey,
  type CountryIdentityKey,
  type DrawnFlagEntry,
  type EntityIdentityKey,
  type EntityRunEntry,
  type HourSpan,
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
  loadDesigns(scope: IdentityScope, owner: string, signal: AbortSignal, recording?: DashboardSource): Promise<EquipmentDesignsReply>;
  loadPlaceNames(scope: IdentityScope, hour: number, states: readonly number[], provinces: readonly number[], signal: AbortSignal, recording?: DashboardSource): Promise<PlaceNamesReply>;
  loadLeaders(scope: IdentityScope, owner: string, hour: number, signal: AbortSignal, recording?: DashboardSource): Promise<UnitLeadersReply>;
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
    loadDesigns: (scope, owner, signal, recording) => loadProduct<EquipmentDesignsReply>(
      client,
      productRequest({ sourceId: scope.sourceId, revision: scope.revision, product: 'equipment-designs', params: { branchId: scope.branchId, country: countryFromKey(owner) } }),
      signal,
      undefined,
      recording,
    ),
    loadPlaceNames: (scope, hour, states, provinces, signal, recording) => loadTransientProduct<PlaceNamesReply>(
      client,
      productRequest({ sourceId: scope.sourceId, revision: scope.revision, product: 'place-names', params: {
        branchId: scope.branchId,
        land: false,
        places: provinces.map(provinceId => ({ provinceId, hour: hour as GatedHour })),
        states: states.map(stateId => ({ stateId, hour: hour as GatedHour })),
      } }),
      signal,
      recording,
    ),
    loadLeaders: (scope, owner, hour, signal, recording) => loadTransientProduct<UnitLeadersReply>(
      client,
      productRequest({ sourceId: scope.sourceId, revision: scope.revision, product: 'unit-leaders', params: {
        branchId: scope.branchId,
        hour: hour as GatedHour,
        selection: owner ? { kind: 'countries', countries: [countryFromKey(owner)] } : { kind: 'all' },
        leaders: 'all',
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

const REGION_ABSENT = 'Not a strategic region of this recording';
const DESIGN_ABSENT = 'Not one of the country’s saved designs at this hour';
const DESIGNS_NOT_RECORDED = 'The country’s designs were not recorded at this hour';
const LEADER_ABSENT = 'Not a leader of this country at this hour';
const LEADER_NAME_MISSING = 'Leader name not recorded';
const PLACE_NOT_ANSWERED = 'Place name not answered';

type EntityRequest =
  | { readonly kind: 'regions'; readonly id: string }
  | { readonly kind: 'designs'; readonly id: string; readonly owner: string }
  | { readonly kind: 'places'; readonly id: string; readonly hour: number }
  | { readonly kind: 'leaders'; readonly id: string; readonly owner: string; readonly hour: number };

interface WantedEntities {
  readonly request: EntityRequest;
  readonly keys: Map<string, EntityIdentityKey>;
}

interface EntityTask {
  readonly request: EntityRequest;
  readonly keys: ReadonlyMap<string, EntityIdentityKey>;
  readonly controller: AbortController;
}

function entityRequestOf(key: EntityIdentityKey, at: number): EntityRequest {
  switch (key.kind) {
    case 'region':
      return { kind: 'regions', id: 'regions' };
    case 'design':
      return { kind: 'designs', id: `designs\u0000${key.owner}`, owner: key.owner };
    case 'state':
    case 'province':
      return { kind: 'places', id: `places\u0000${at}`, hour: at };
    case 'leader':
      return { kind: 'leaders', id: `leaders\u0000${key.owner}\u0000${at}`, owner: key.owner, hour: at };
  }
}

function numericId(id: number | string): number | null {
  const value = typeof id === 'number' ? id : Number(id);
  return Number.isSafeInteger(value) ? value : null;
}

function designRuns(
  lives: readonly HourSpan[],
  known: readonly HourSpan[],
  bounds: HourInterval,
  named: Omit<EntityRunEntry, 'firstHour' | 'lastHour'>,
): EntityRunEntry[] {
  const runs: EntityRunEntry[] = [];
  const sortedKnown = [...known].sort((a, b) => a.firstHour - b.firstHour);
  const sortedLives = [...lives].sort((a, b) => a.firstHour - b.firstHour);
  let cursor = bounds[0];
  for (const span of sortedKnown) {
    if (span.firstHour > cursor) runs.push({ firstHour: cursor, lastHour: span.firstHour - 1, name: null, nameReason: DESIGNS_NOT_RECORDED });
    let at = Math.max(cursor, span.firstHour);
    for (const life of sortedLives) {
      const first = Math.max(life.firstHour, at);
      const last = Math.min(life.lastHour, span.lastHour);
      if (last < first) continue;
      if (first > at) runs.push({ firstHour: at, lastHour: first - 1, absent: true, reason: DESIGN_ABSENT, name: null });
      runs.push({ firstHour: first, lastHour: last, ...named });
      at = last + 1;
    }
    if (at <= span.lastHour) runs.push({ firstHour: at, lastHour: span.lastHour, absent: true, reason: DESIGN_ABSENT, name: null });
    cursor = Math.max(cursor, span.lastHour + 1);
  }
  if (cursor <= bounds[1]) runs.push({ firstHour: cursor, lastHour: bounds[1], name: null, nameReason: DESIGNS_NOT_RECORDED });
  return runs;
}

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

  factsController: AbortController | null;
  factsRetryTimer: (() => void) | null;
  factsFailures: number;

  identitiesController: AbortController | null;
  groupsController: AbortController | null;

  readonly styleRunning: Map<number, StyleTask>;
  readonly styleInflightKeys: Set<string>;
  readonly styleRetryAt: Map<string, { readonly count: number; readonly at: number; readonly error: Failure }>;
  styleRetryTimer: (() => void) | null;

  readonly flagRunning: Map<number, FlagTask>;
  readonly flagInflightKeys: Set<string>;
  readonly flagFailures: Map<string, FlagFailure>;
  flagRetryTimer: (() => void) | null;

  readonly entityRunning: Map<string, EntityTask>;
  readonly entityRetryAt: Map<string, { readonly count: number; readonly at: number }>;
  readonly entityLoaded: Set<string>;
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
        factsController: null,
        factsRetryTimer: null,
        factsFailures: 0,
        identitiesController: null,
        groupsController: null,
        styleRunning: new Map(),
        styleInflightKeys: new Set(),
        styleRetryAt: new Map(),
        styleRetryTimer: null,
        flagRunning: new Map(),
        flagInflightKeys: new Set(),
        flagFailures: new Map(),
        flagRetryTimer: null,
        entityRunning: new Map(),
        entityRetryAt: new Map(),
        entityLoaded: new Set(),
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
    const wanted = new Map<string, CountryFlagChoice>();
    for (const choice of choices) {
      const key = drawnFlagKey(size, choice);
      if (state.index.getDrawnFlag(size, choice) || state.flagInflightKeys.has(key)) continue;
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
    const wantedFlagsBySize = new Map<CountryFlagSize, Map<string, CountryFlagChoice>>();
    const wantedEntities = new Map<string, WantedEntities>();

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
            const flagKey = drawnFlagKey(size, resolved.flagChoice.value);
            if (!state.index.getDrawnFlag(size, resolved.flagChoice.value) && !state.flagInflightKeys.has(flagKey)) {
              let map = wantedFlagsBySize.get(size);
              if (!map) wantedFlagsBySize.set(size, map = new Map());
              map.set(flagKey, resolved.flagChoice.value);
            }
          }
        } else if (ask.key.kind === 'faction') {
          wantGroups = true;
          wantFacts = true;
        } else if (state.index.resolve(ask.key, at).kind === 'pending') {
          const request = entityRequestOf(ask.key, at);
          let wanted = wantedEntities.get(request.id);
          if (!wanted) wantedEntities.set(request.id, wanted = { request, keys: new Map() });
          wanted.keys.set(identityKeyString(ask.key), ask.key);
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

    this.pumpEntities(state, wantedEntities);
  }

  private pumpEntities(state: ScopeState, wanted: Map<string, WantedEntities>): void {
    for (const [id, task] of [...state.entityRunning]) {
      if (wanted.has(id) || task.request.kind === 'regions' || task.request.kind === 'designs') continue;
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
        case 'designs':
          if (state.index.resolve(key, bounds[0]).kind === 'pending') {
            state.index.setEntityRuns(key, [{ firstHour: bounds[0], lastHour: bounds[1], absent: true, reason: DESIGN_ABSENT, name: null }]);
          }
          break;
        case 'places':
          if (state.index.resolve(key, request.hour).kind === 'pending') {
            state.index.addEntityRun(key, { firstHour: request.hour, lastHour: request.hour, name: null, nameReason: PLACE_NOT_ANSWERED });
          }
          break;
        case 'leaders':
          if (state.index.resolve(key, request.hour).kind === 'pending') {
            state.index.addEntityRun(key, { firstHour: request.hour, lastHour: request.hour, absent: true, reason: LEADER_ABSENT, name: null });
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
      case 'designs':
        return this.transport.loadDesigns(state.scope, request.owner, signal, state.recording).then(reply => {
          const bounds = index.bounds();
          const icons = new Map(reply.icons.map(icon => [icon.key, icon.image]));
          for (const design of reply.designs) {
            const named = design.name;
            index.setEntityRuns({ kind: 'design', owner: request.owner, variant: design.id }, designRuns(design.lives, reply.known, bounds, {
              name: named.known ? named.name : null,
              ...(named.nameUnknown ? { nameReason: named.nameUnknown } : {}),
              subtitle: named.typeName,
              artKey: named.icon === null ? null : icons.get(named.icon) ?? null,
              ...(named.iconUnknown ? { artReason: named.iconUnknown } : {}),
              warnings: reply.warnings,
            }));
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
        return this.transport.loadPlaceNames(state.scope, request.hour, states, provinces, signal, state.recording).then(reply => {
          for (const place of reply.states) {
            index.addEntityRun({ kind: 'state', id: place.stateId }, {
              firstHour: place.hour, lastHour: place.hour, name: place.name, ...(place.missing ? { nameReason: place.missing } : {}), warnings: reply.warnings,
            });
          }
          for (const place of reply.places) {
            index.addEntityRun({ kind: 'province', id: place.provinceId }, {
              firstHour: place.hour, lastHour: place.hour, name: place.name, ...(place.missing ? { nameReason: place.missing } : {}), warnings: reply.warnings,
            });
          }
        });
      }
      case 'leaders':
        return this.transport.loadLeaders(state.scope, request.owner, request.hour, signal, state.recording).then(reply => {
          const hour = request.hour;
          const run = (named: Omit<EntityRunEntry, 'firstHour' | 'lastHour'>): EntityRunEntry => ({ firstHour: hour, lastHour: hour, ...named, warnings: reply.warnings });
          if (reply.missing !== null) {
            for (const key of keys.values()) index.addEntityRun(key, run({ name: null, nameReason: reply.missing }));
            return;
          }
          for (const row of reply.leaders) {
            index.addEntityRun({ kind: 'leader', owner: request.owner, id: row.reference }, run({ name: row.name, ...(row.name === null ? { nameReason: LEADER_NAME_MISSING } : {}) }));
          }
          for (const held of reply.withheld) {
            index.addEntityRun({ kind: 'leader', owner: request.owner, id: held.reference }, run({ name: null, nameReason: held.reason }));
          }
        });
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
        if (request.kind === 'regions' || request.kind === 'designs') state.entityLoaded.add(request.id);
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
        for (const key of keys.values()) state.index.setEntityFailure(key, failure);
        this.notify(state);
        this.reconcile(state);
      },
    );
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
      if (state.index.getDrawnFlag(size, choice) || state.flagInflightKeys.has(key)) continue;
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
        const unavailable = new Map(reply.unavailable.map(item => [item.flag, item.reason]));
        batch.forEach(([key, choice], index) => {
          state.flagInflightKeys.delete(key);
          state.flagFailures.delete(key);
          const image = images.get(index);
          const reason = unavailable.get(index);
          const entry: DrawnFlagEntry = image
            ? { url: image.png, image }
            : { reason: reason ?? 'Flag unavailable' };
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
