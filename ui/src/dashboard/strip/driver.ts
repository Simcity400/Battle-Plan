import { createContext } from 'react';
import { engine, type FrameEngine } from '../../engine/frame.ts';
import { clientIdentityTransport, identityBrokerFor, type IdentityFlight, type StripIdentityCosts, type StripIdentitySource } from '../../families/identity/broker.ts';
import { identityKeyString, identityScopeKey, type IdentityAsk, type IdentitySnapshot } from '../../families/identity/types.ts';
import { busyKinds, inBackground, onForegroundRequest } from '../../shell/busy.ts';
import { perfLog } from '../../shell/perf.ts';
import type { StripReply } from '../../generated/dashboard.ts';
import { validateProductReply, type DashboardClient } from '../client.ts';
import type { PlaybackController } from '../playback.ts';
import { baseClient, prepareGate, type PrepareGate } from '../prepare.ts';
import { RefusedError } from '../queryError.ts';
import { productRequest, type ProductRequest } from '../request.ts';
import type { StripTileClaim, StripTileRead } from '../tiles/cache.ts';
import { tileCache } from '../tiles/view.ts';
import { readStripHeader } from './decode.ts';
import type { StripDensity, StripLattice } from './lattice.ts';
import { stripCoverageAt, type StripFrame, type StripKeyframe, type StripScenePlan, type StripTrack } from './scrub.ts';
import { EMPTY_STRIP_RECEIPT, type StripReceipt, type StripUpdate } from './store.ts';

export interface StripWidget {
  readonly key: string;
  readonly supported: boolean;
  readonly fixed?: boolean;
  readonly tiles?: (claim: StripTileClaim, read: StripTileRead) => Promise<void>;
  readonly identity?: StripIdentitySource;
  readonly track?: StripTrack;
  keyframe(hour: number): StripKeyframe;
  scrub?(page: number, frame: StripFrame | null): void;
  store(build: number, update: StripUpdate): Promise<StripReceipt>;
  settled(): boolean;
}

type Slot = { name: string; hours: Set<number>; held: StripReceipt; coarseNext: number; fineNext: Map<number, number> };
type HeldWidget = { input: StripWidget; build: number; bytes: number; coarse: number | null; slots: Slot[]; building: StripDensity; claim?: StripTileClaim };
type IdentityLease = ReturnType<ReturnType<typeof identityBrokerFor>['holdStripIdentities']>;
type IdentityLevel = { scopes: Map<string, IdentityLease>; ready: boolean };
type Status = 'not ready' | 'coarse' | 'ready' | 'will not follow';
export const STRIP_PAGE_BUDGET_BYTES = 64 * 1024 * 1024;
const BUILD_TIMELINE_STRIPS = false;
const YIELD_FRAME_FACTOR = 2;
const MAX_YIELD_FRAMES = 32;
const DENSITIES = [16, 8, 4] as const;
let nextBuild = 0;
let nextPage = 0;

const slotAt = (slot: Slot, selected: ReadonlySet<number>, density: StripDensity) => {
  const first = slot.hours.keys().next().value;
  const expected = selected.size + Number(first !== undefined && !selected.has(first));
  return (density === 1 ? slot.held.coarseKeys : slot.held.keys) === expected;
};

function pageReadiness(held: ReadonlyMap<string, HeldWidget>, mounted: number, selected: ReadonlySet<number>, density: StripDensity,
  budget: number, identityBytes: number, identitiesReady: boolean) {
  let bytes = identityBytes;
  let complete = identitiesReady && held.size === mounted;
  for (const widget of held.values()) {
    bytes += widget.bytes;
    complete &&= (!widget.input.tiles || widget.claim?.active === true) && widget.slots.every(slot => slotAt(slot, selected, density));
  }
  return { bytes, state: bytes > budget ? 'over budget' : complete ? 'ready' : 'incomplete' } as const;
}

function* fineChains(hours: Iterable<number>, coarse: ReadonlySet<number>, fine: ReadonlySet<number>) {
  let chain: number[] = [];
  for (const hour of hours) {
    if (coarse.has(hour)) {
      chain.push(hour);
      if (chain.length > 2) yield chain;
      chain = [hour];
    } else if (fine.has(hour)) chain.push(hour);
  }
  if (chain.length > 1) yield chain;
}

export class StripPage {
  readonly lattice: StripLattice;
  private readonly clock: PlaybackController;
  private readonly client: DashboardClient;
  private readonly budget: number;
  private readonly gate: PrepareGate;
  private readonly name?: string;
  private readonly frames: FrameEngine;
  private readonly id = ++nextPage;
  private holdDrag: (() => void) | undefined;
  status: Status = 'not ready';
  density: StripDensity = 1;
  private readonly widgets = new Map<string, StripWidget>();
  private readonly held = new Map<string, HeldWidget>();
  private readonly identities = new Map<StripDensity, IdentityLevel>();
  private readonly identityWindows = new Map<string, { bytes: number; claims: Map<IdentityLease, StripIdentityCosts> }>();
  private identityBytes = 0;
  private readonly products = new Set<string>();
  private readonly counts = { groupsAsked: 0, groupsCancelled: 0, groupsResumed: 0, groupsGaveWay: 0, anchorsAsked: 0, anchorsCommitted: 0,
    yieldedToForeign: 0, foreignYieldFrames: 0,
    keyframesAsked: 0, asksDiscarded: 0, releaseCount: 0, releasedBytes: 0, startedWhilePlaying: 0, startedWhileSeeking: 0, startedWhileForeground: 0,
    tilesAsked: 0, tilesCancelled: 0, tilesStartedWhileScrubbing: 0,
    identityAsked: 0, identityCancelled: 0, identityResumed: 0, identityScans: 0, identityRounds: 0 };
  private build: AbortController | null = null;
  private scheduled = false;
  private running = Promise.resolve();
  private enabled = false;

  constructor(lattice: StripLattice, clock: PlaybackController, client: DashboardClient,
    { budget = STRIP_PAGE_BUDGET_BYTES, gate = prepareGate(client), name, frames = engine }:
    { budget?: number; gate?: PrepareGate; name?: string; frames?: FrameEngine } = {}) {
    this.lattice = lattice;
    this.clock = clock;
    this.client = client;
    this.budget = budget;
    this.gate = gate;
    this.name = name;
    this.frames = frames;
  }

  start() {
    this.enabled = true;
    this.changed();
    const stopScrub = this.publishScrub();
    return () => { stopScrub(); this.enabled = false; this.changed(); };
  }

  register(id: string, widget: StripWidget) {
    this.widgets.set(id, widget);
    this.changed();
    return () => { if (this.widgets.get(id) === widget) { this.widgets.delete(id); this.changed(); } };
  }

  rowWindowChanged(before: { first: number; count: number } | null | undefined, next: typeof before) {
    if (before?.first !== next?.first || before?.count !== next?.count) this.changed(true);
  }

  private changed(keepIdentities = false) {
    this.status = 'not ready';
    this.holdDrag?.();
    this.build?.abort();
    if (!keepIdentities) for (const density of this.identities.keys()) this.releaseIdentities(density);
    if (this.scheduled) return;
    this.scheduled = true;
    queueMicrotask(() => {
      this.scheduled = false;
      const build = this.build = new AbortController();
      this.running = this.running.then(() => this.run(build.signal));
    });
  }

  private publishScrub() {
    let active = false, held = false, loggedMixedGap = false, loggedTiles = false;
    const posted = new Set<NonNullable<StripWidget['scrub']>>();
    const clear = () => {
      for (const send of posted) send(this.id, null);
      posted.clear();
    };
    const hasTiles = () => [...this.widgets.values()].some(widget => widget.tiles);
    this.holdDrag = () => {
      if (!active) return;
      held = true;
      clear();
      if (!loggedTiles && hasTiles()) {
        perfLog?.strip({ page: this.name, status: 'drag held', position: this.clock.getScrubPosition(), reason: 'tile widgets follow from 3C-ii' });
        loggedTiles = true;
      }
    };
    const ready = () => this.status === 'coarse' || this.status === 'ready';
    let task: ReturnType<FrameEngine['every']> | undefined;
    const update = () => {
      if (this.clock.getScrubPosition() === null) {
        active = false;
        task?.stop();
        task = undefined;
        clear();
      } else if (!active) {
        active = true;
        held = false;
        loggedMixedGap = false;
        loggedTiles = false;
        if (!ready() || hasTiles()) this.holdDrag?.();
        if (held || ![...this.widgets.values()].some(widget => widget.scrub)) return;
        task = this.frames.every('data', ({ index }) => {
          const position = this.clock.getScrubPosition();
          if (held || !ready() || position === null) { this.holdDrag?.(); return false; }
          const density = this.density;
          const target = this.lattice.scrub(position, density);
          if (!target) {
            perfLog?.strip({ page: this.name, status: 'drag held', position, builtEnd: this.lattice.last });
            this.holdDrag?.();
            return false;
          }
          if (target.gap && [...this.widgets.values()].some(widget => widget.track && stripCoverageAt(widget.track, target.hour).hour !== null)) {
            if (!loggedMixedGap) {
              perfLog?.strip({ page: this.name, status: 'drag held', position, reason: 'A widget recorded the reference gap' });
              loggedMixedGap = true;
            }
            clear();
            return true;
          }
          const frame: StripFrame = { ...target, page: this.id, frame: index, density, ...(perfLog ? { measure: true } : {}) };
          for (const widget of this.widgets.values()) if (widget.scrub) posted.add(widget.scrub);
          for (const send of posted) send(this.id, frame);
          return true;
        });
      }
    };
    const stop = this.clock.subscribeScrub(update);
    update();
    return () => { stop(); task?.stop(); clear(); this.holdDrag = undefined; };
  }

  private accountIdentity(scope: string, id: string, lease: IdentityLease, costs?: StripIdentityCosts) {
    const key = JSON.stringify([scope, id]);
    let window = this.identityWindows.get(key);
    if (!window) this.identityWindows.set(key, window = { bytes: 0, claims: new Map() });
    if (costs) window.claims.set(lease, costs); else window.claims.delete(lease);
    const parts = new Map<string, number>();
    for (const claim of window.claims.values()) for (const [part, bytes] of claim) parts.set(part, Math.max(parts.get(part) ?? 0, bytes));
    const bytes = [...parts.values()].reduce((sum, size) => sum + size, 0);
    this.identityBytes += bytes - window.bytes;
    window.bytes = bytes;
    if (!window.claims.size) this.identityWindows.delete(key);
  }

  private releaseIdentities(density: StripDensity, levels = this.identities) {
    const held = levels.get(density);
    levels.delete(density);
    for (const [scope, lease] of held?.scopes ?? []) {
      lease.release();
      for (const window of lease.windows.keys()) this.accountIdentity(scope, window, lease);
    }
  }

  private async run(signal: AbortSignal) {
    if (signal.aborted) return;
    const widgets = this.enabled && [...this.widgets.values()].every(w => w.supported) ? new Map(this.widgets) : new Map<string, StripWidget>();
    const cache = tileCache(this.client);
    const levels = new Map<StripDensity, Set<number>>(([1, ...DENSITIES] as const).map(density => [density, new Set(this.lattice.hours(density))]));
    let candidate: StripDensity = 1;
    let finest: number = this.density !== 1 && this.identities.get(this.density)?.ready ? this.density : 16;
    const retiring = new Map<StripDensity, IdentityLevel>();
    const readiness = (density = candidate) => pageReadiness(this.held, widgets.size, levels.get(density)!, density,
      this.budget, this.identityBytes, this.identities.get(density)?.ready === true);
    const received = () => {
      const level = readiness();
      if (!signal.aborted && level.state === 'over budget') this.status = 'not ready';
      return level.state;
    };
    const report = (detail?: unknown, lost = false) => {
      if (signal.aborted && !lost) return;
      perfLog?.strip({ ...this.counts, page: this.name, status: this.status, density: this.density,
        builtEnd: this.lattice.last, products: [...this.products], bytes: readiness().bytes, identityBytes: this.identityBytes, budgetBytes: this.budget, maxLateAsksPerWorker: 1, detail,
        tileBytes: cache.stripBytes, tileCacheBytes: cache.bytes, tileBudgetBytes: cache.cap,
        supportedWidgets: [...this.widgets].filter(([id, widget]) => widget.supported && (!widget.tiles || this.held.get(id)?.claim?.active)).length,
        mountedWidgets: this.widgets.size,
        widgets: [...this.held].map(([widget, held]) => ({ widget, bytes: held.bytes,
          keyframes: held.slots.length ? Math.min(...held.slots.map(slot => slot.held.keys)) : 0 })) });
    };
    const drop = async (widget: HeldWidget, fine: boolean) => {
      const before = widget.bytes;
      if (!fine) widget.claim?.release();
      const held = widget.input.store(widget.build, { kind: 'drop', fine });
      widget.bytes = (await (fine ? held : held.catch(() => EMPTY_STRIP_RECEIPT))).bytes;
      for (const slot of widget.slots) {
        slot.fineNext.clear();
        slot.held = fine ? { ...slot.held, keys: slot.held.coarseKeys } : EMPTY_STRIP_RECEIPT;
      }
      widget.building = 1;
      if (before > widget.bytes) { this.counts.releaseCount++; this.counts.releasedBytes += before - widget.bytes; }
      received();
    };
    const dropFine = () => {
      for (const density of this.identities.keys()) if (density !== 1) this.releaseIdentities(density);
      return Promise.all([...this.held.values()].filter(widget => widget.building !== 1).map(widget => drop(widget, true)));
    };
    const fits = async (state = readiness().state) => {
      signal.throwIfAborted();
      if (state !== 'over budget') return true;
      if (retiring.size) {
        for (const density of retiring.keys()) this.releaseIdentities(density, retiring);
        if (readiness().state !== 'over budget') return true;
      }
      if (candidate !== 1) return false;
      for (const widget of this.held.values()) if (widget.building !== 1) finest = Math.min(finest, widget.building / 2);
      await dropFine();
      signal.throwIfAborted();
      return readiness().state !== 'over budget';
    };
    for (const [id, widget] of this.held) if (widgets.get(id)?.key !== widget.input.key) {
      await drop(widget, false);
      this.held.delete(id);
      if (signal.aborted) return;
      report();
    }
    if (signal.aborted || !widgets.size) {
      if (this.enabled && this.widgets.size) report('A mounted widget cannot follow the timeline');
      return;
    }
    for (const [density, level] of this.identities) retiring.set(density, level);
    this.identities.clear();
    const playing = () => this.clock.getSnapshot().playing;
    const seeking = () => this.clock.getScrubPosition() !== null || [...widgets.values()].some(w => !w.fixed && !w.settled());
    const foreground = () => busyKinds().requests > 0;
    const idle = () => !playing() && !foreground() && !seeking();
    let flight: AbortController | null = null;
    const stopClock = this.clock.onPublish((before, next) => {
      if (next.playing || before.seekSequence !== next.seekSequence || before.hour !== next.hour) flight?.abort();
    });
    const stopScrub = this.clock.subscribeScrub(() => { if (this.clock.getScrubPosition() !== null) flight?.abort(); });
    const stopForeground = onForegroundRequest(() => flight?.abort());
    const update = async (widget: HeldWidget, slot: Slot, op: Exclude<StripUpdate, { kind: 'drop' }>) => {
      slot.held = await widget.input.store(widget.build, op);
      widget.bytes = slot.held.bytes;
      return received();
    };
    const ask = async <T,>(request: ProductRequest | null, load: (signal: AbortSignal) => Promise<T>,
      start: (resuming: boolean) => void, cancel: () => void, service = true): Promise<T> => {
      let resuming = false;
      for (;;) {
        signal.throwIfAborted();
        const attempt: { signal?: AbortSignal } = {};
        try {
          const response = await this.gate.ask(request, async () => {
            flight = new AbortController();
            attempt.signal = AbortSignal.any([signal, flight.signal]);
            start(resuming);
            if (service) {
              this.counts.startedWhilePlaying += Number(playing());
              this.counts.startedWhileSeeking += Number(seeking());
              this.counts.startedWhileForeground += Number(foreground());
            }
            return inBackground(() => load(attempt.signal!));
          }, signal, idle);
          attempt.signal!.throwIfAborted();
          return response;
        } catch (error) {
          if (!attempt.signal?.aborted) throw error;
          cancel();
          resuming = true;
          report();
        } finally { flight = null; }
      }
    };
    const requestAt = (widget: HeldWidget, slot: Slot, hour: number) => widget.input.keyframe(hour).asks.find(ask => ask.slot === slot.name)!.request!;
    const read = async <T>(request: ProductRequest, group?: { product: string; keys: number; anchors: number }) => {
      const response = await ask(request, signal => baseClient(this.client).product<T>(request, signal), resuming => {
        if (group) {
          this.counts.groupsAsked++;
          if (resuming) this.counts.groupsResumed++;
          this.counts.anchorsAsked += group.anchors;
          this.counts.keyframesAsked += group.keys - group.anchors;
        } else {
          this.counts.tilesAsked++;
          this.counts.tilesStartedWhileScrubbing += Number(this.clock.getScrubPosition() !== null);
        }
        this.products.add(group?.product ?? request.product);
      }, () => {
        if (group) {
          this.counts.groupsCancelled++;
          this.counts.asksDiscarded += group.keys;
        } else this.counts.tilesCancelled++;
      });
      return validateProductReply(response, request);
    };
    const identityFlight: IdentityFlight = load => ask(null, load, resuming => {
      this.counts.identityAsked++;
      if (resuming) this.counts.identityResumed++;
    }, () => { this.counts.identityCancelled++; });
    const prepareIdentities = async (density: StripDensity, selected: ReadonlySet<number>) => {
      const level: IdentityLevel = { scopes: new Map(), ready: false };
      this.identities.set(density, level);
      const groups = new Map<string, { source: StripIdentitySource; widgets: HeldWidget[] }>();
      for (const widget of this.held.values()) {
        const source = widget.input.identity;
        if (!source || widget.input.fixed || widget.input.tiles) continue;
        const key = identityScopeKey(source.scope);
        let group = groups.get(key);
        if (!group) groups.set(key, group = { source, widgets: [] });
        group.widgets.push(widget);
      }
      const fine = density !== 1;
      const install = (widgets: readonly HeldWidget[], stage?: IdentitySnapshot) =>
        ask(null, () => Promise.all(widgets.map(widget => widget.input.store(widget.build, {
          kind: 'identities', fine, ...(stage ? { snapshot: stage } : {}),
        }))), () => {}, () => {}, false);
      for (const [key, { source, widgets }] of groups) {
        signal.throwIfAborted();
        const lease = identityBrokerFor(this.client).holdStripIdentities(source, clientIdentityTransport(baseClient(this.client), true));
        level.scopes.set(key, lease);
        const account = (window: string, costs: StripIdentityCosts) => {
          this.accountIdentity(key, window, lease, costs);
          return fits(received());
        };
        let snapshot = (retiring.get(density)?.scopes.get(key) ?? (fine ? this.identities.get(1)?.scopes.get(key) : undefined))?.snapshot() ?? lease.snapshot();
        const wants = new Map<string, IdentityAsk>();
        let frames = widgets.flatMap(widget => [...selected].map(hour => ({ widget, hour })));
        for (;;) {
          this.counts.identityRounds++;
          await install(widgets, snapshot);
          const before = wants.size;
          const pending: typeof frames = [];
          for (const entry of frames) {
            const { widget, hour } = entry;
            const receipt = await ask(null, () => widget.input.store(widget.build, { kind: 'discover', fine, hour,
              measure: !!perfLog }), () => { this.counts.identityScans++; }, () => {}, false);
            if (!receipt.identities) throw new Error('A dashboard lane did not discover strip identities');
            for (const ask of receipt.identities) wants.set(`${identityKeyString(ask.key)}@${ask.hour}`, ask);
            if (receipt.unresolved) pending.push(entry);
          }
          if (wants.size !== before) {
            if (!await lease.prepare([...wants.values()], identityFlight, account)) return false;
            snapshot = lease.snapshot();
            frames = pending;
            continue;
          }
          if (pending.length) throw new Error('A dashboard strip identity was not answered');
          if (!wants.size) await install(widgets, lease.snapshot());
          break;
        }
      }
      if (groups.size) await install([...groups.values()].flatMap(group => group.widgets));
      this.releaseIdentities(density, retiring);
      level.ready = true;
      return true;
    };
    const send = async (widget: HeldWidget, slot: Slot, hours: number[], whole: boolean, anchors: number) => {
      const requests = hours.map(hour => requestAt(widget, slot, hour));
      const first = requests[0]!;
      const request = productRequest({ sourceId: first.sourceId, revision: first.revision, product: 'strip',
        params: { product: first.product, asks: requests.map(ask => ask.params), whole } });
      const response = await read<StripReply>(request, { product: first.product, keys: hours.length, anchors });
      const binary = atob(response.data.data), data = new Uint8Array(binary.length);
      for (let i = 0; i < binary.length; i++) data[i] = binary.charCodeAt(i);
      return data;
    };
    const chain = async (widget: HeldWidget, slot: Slot, hours: number[], fine: boolean, closesAtAnchor: boolean) => {
      const first = hours[0]!;
      let next = fine ? slot.fineNext.get(first) ?? 1 : slot.coarseNext, isolateUntil = 0, retryFrames = 1;
      while (next < hours.length) {
        const start = Math.max(0, next - 1);
        const end = next < isolateUntil ? next + 1 : Math.min(hours.length, start + 17);
        let state: ReturnType<typeof received>;
        try {
          const group = hours.slice(start, end), anchors = Number(next !== 0) + Number(closesAtAnchor && end === hours.length);
          const bytes = await send(widget, slot, group, next === 0, anchors);
          signal.throwIfAborted();
          const { whole, count } = readStripHeader(bytes);
          if (whole !== (next === 0) || count > group.length) throw new Error('Invalid dashboard strip prefix');
          const gaveWay = count < group.length;
          if (gaveWay) this.counts.groupsGaveWay++;
          const answered = start + count;
          if (answered === next) {
            if (idle()) {
              this.counts.yieldedToForeign++;
              let framesLeft = retryFrames;
              retryFrames = Math.min(MAX_YIELD_FRAMES, retryFrames * YIELD_FRAME_FACTOR);
              await new Promise<void>(resolve => {
                const task = this.frames.every('data', () => {
                  this.counts.foreignYieldFrames++;
                  if (--framesLeft > 0 && idle() && !signal.aborted) return true;
                  task.stop();
                  resolve();
                });
              });
            }
            report();
            continue;
          }
          state = await update(widget, slot, { kind: 'append', slot: slot.name, hours: group.slice(0, count), bytes, fine });
          retryFrames = 1;
          this.counts.anchorsCommitted += Number(next !== 0) + Number(closesAtAnchor && answered === hours.length);
          next = answered;
          if (fine) slot.fineNext.set(first, next); else slot.coarseNext = next;
          if (gaveWay) report();
          signal.throwIfAborted();
        } catch (error) {
          if (signal.aborted) throw error;
          if (end - next <= 1 || error instanceof RefusedError) throw new Error(`${requestAt(widget, slot, hours[next]!).product} at ${hours[next]}: ${error}`);
          isolateUntil = end;
          continue;
        }
        if (!await fits(state)) return false;
        signal.throwIfAborted();
      }
      return true;
    };
    const coarse = levels.get(1)!;
    try {
      for (const [id, source] of widgets) if (!this.held.has(id)) {
        await this.gate.wait(signal, idle);
        signal.throwIfAborted();
        const slots = new Map<string, Slot>();
        for (const hour of this.lattice.hours(16)) for (const { slot, request } of source.keyframe(hour).asks) {
          let entry = slots.get(slot);
          if (!entry) slots.set(slot, entry = { name: slot, hours: new Set(), held: EMPTY_STRIP_RECEIPT, coarseNext: 0, fineNext: new Map() });
          if (request) entry.hours.add(hour);
        }
        if (!slots.size && !source.fixed && !source.tiles) slots.set('', { name: '', hours: new Set(), held: EMPTY_STRIP_RECEIPT, coarseNext: 0, fineNext: new Map() });
        this.held.set(id, { input: source, build: ++nextBuild, bytes: 0, coarse: null, building: 1, slots: [...slots.values()] });
      }
      for (const widget of this.held.values()) if (widget.input.tiles && !widget.claim?.active) {
        await this.gate.wait(signal, idle);
        signal.throwIfAborted();
        const claim = cache.claim(() => {
          this.changed();
          report('A timeline tile claim yielded to the active viewport or recording', true);
        });
        try {
          await widget.input.tiles(claim, read);
          signal.throwIfAborted();
          widget.claim = claim;
        } finally {
          if (widget.claim !== claim) claim.release();
        }
      }
      const level = async (density: StripDensity) => {
        candidate = density;
        const selected = levels.get(density)!, fine = density !== 1;
        if (!await fits()) return false;
        for (const widget of this.held.values()) {
          signal.throwIfAborted();
          if (fine) widget.building = density;
          for (const slot of widget.slots) {
            if (!fine && slotAt(slot, selected, density)) continue;
            const anchors = new Set([...slot.hours].slice(0, 1).concat([...coarse].filter(hour => slot.hours.has(hour))));
            const chains = fine ? fineChains(slot.hours, anchors, selected) : [[...anchors]];
            for (const chainHours of chains) {
              if (!await chain(widget, slot, chainHours, fine, fine && anchors.has(chainHours.at(-1)!))) return false;
              signal.throwIfAborted();
            }
            const marks = [...selected].filter(hour => !slot.hours.has(hour) && (!fine || !coarse.has(hour)));
            if (marks.length && !await fits(await update(widget, slot, { kind: 'mark', slot: slot.name, hours: marks, fine }))) return false;
            signal.throwIfAborted();
          }
          if (widget.input.scrub && !widget.input.fixed && !widget.input.tiles) {
            const frames: [number, StripScenePlan][] = [];
            for (const hour of selected) {
              if (fine && coarse.has(hour)) continue;
              const scene = widget.input.keyframe(hour).scene;
              if (scene) frames.push([hour, scene]);
            }
            widget.bytes = (await widget.input.store(widget.build, { kind: 'scenes', page: this.id, fine, frames, track: widget.input.track })).bytes;
            if (!await fits(received())) return false;
          }
          if (!fine) widget.coarse ??= widget.bytes;
        }
        return await prepareIdentities(density, selected) && await fits();
      };
      const publish = (density: StripDensity) => {
        signal.throwIfAborted();
        if (readiness(density).state !== 'ready') throw new Error('A dashboard lane did not hold its strip level');
        this.density = density;
        this.status = density === 1 ? 'coarse' : 'ready';
        report();
      };
      if (!await level(1)) { report(`Coarse strips exceed ${this.budget} bytes`); throw new Error('Coarse strips exceed the page budget'); }
      publish(1);
      const coarseBytes = [...this.held.values()].reduce((sum, widget) => sum + (widget.coarse ?? 0), 0);
      const available = DENSITIES.filter(density => density <= finest);
      const start = available.findIndex(d => coarseBytes * levels.get(d)!.size <= this.budget * coarse.size);
      for (const density of available.slice(start === -1 ? -1 : start)) {
        await Promise.all([...this.held.values()].filter(widget => widget.building !== 1 && widget.building !== density).map(widget => drop(widget, true)));
        signal.throwIfAborted();
        if (await level(density)) { publish(density); break; }
        signal.throwIfAborted();
        report(`Fine strips at ${density} steps exceed ${this.budget} bytes`);
        await dropFine();
        publish(1);
      }
    } catch (error) {
      if (!signal.aborted) {
        this.status = 'will not follow'; report(String(error));
        for (const density of this.identities.keys()) this.releaseIdentities(density);
        await Promise.all([...this.held.values()].map(widget => drop(widget, false)));
        this.held.clear();
        report();
      }
    } finally {
      for (const density of retiring.keys()) this.releaseIdentities(density, retiring);
      stopClock(); stopScrub();
      stopForeground();
    }
  }
}

export const configuredStripPage = <T>(make: () => T): T | null => BUILD_TIMELINE_STRIPS ? make() : null;
export const StripPageContext = createContext<StripPage | null>(null);
