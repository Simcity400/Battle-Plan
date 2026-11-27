import { createContext } from 'react';
import { engine, type FrameEngine } from '../../engine/frame.ts';
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
import type { StripDensity, StripLattice } from './lattice.ts';
import { stripCoverageAt, type StripFrame, type StripKeyframe, type StripScenePlan, type StripTrack } from './scrub.ts';
import { EMPTY_STRIP_RECEIPT, type StripReceipt, type StripUpdate } from './store.ts';

export interface StripWidget {
  readonly key: string;
  readonly supported: boolean;
  readonly fixed?: boolean;
  readonly tiles?: (claim: StripTileClaim, read: StripTileRead) => Promise<void>;
  readonly track?: StripTrack;
  keyframe(hour: number): StripKeyframe;
  scrub?(page: number, frame: StripFrame | null): void;
  store(build: number, update: StripUpdate): Promise<StripReceipt>;
  settled(): boolean;
}

type Slot = { name: string; hours: Set<number>; held: StripReceipt; coarseNext: number; fineNext: Map<number, number> };
type HeldWidget = { input: StripWidget; build: number; bytes: number; coarse: number | null; slots: Slot[]; building: StripDensity; claim?: StripTileClaim };
type Status = 'not ready' | 'coarse' | 'ready' | 'will not follow';
export const STRIP_PAGE_BUDGET_BYTES = 64 * 1024 * 1024;
const BUILD_TIMELINE_STRIPS = false;
const DENSITIES = [16, 8, 4] as const;
let nextBuild = 0;
let nextPage = 0;

const slotAt = (slot: Slot, selected: ReadonlySet<number>, density: StripDensity) => {
  const first = slot.hours.keys().next().value;
  const expected = selected.size + Number(first !== undefined && !selected.has(first));
  return (density === 1 ? slot.held.coarseKeys : slot.held.keys) === expected;
};

function pageReadiness(held: ReadonlyMap<string, HeldWidget>, mounted: number, selected: ReadonlySet<number>, density: StripDensity, budget: number) {
  let bytes = 0;
  let complete = held.size === mounted;
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
  private readonly products = new Set<string>();
  private readonly counts = { groupsAsked: 0, groupsCancelled: 0, groupsResumed: 0, anchorsAsked: 0, anchorsCommitted: 0,
    keyframesAsked: 0, asksDiscarded: 0, releaseCount: 0, releasedBytes: 0, startedWhilePlaying: 0, startedWhileSeeking: 0, startedWhileForeground: 0,
    tilesAsked: 0, tilesCancelled: 0, tilesStartedWhileScrubbing: 0 };
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

  private changed() {
    this.status = 'not ready';
    this.holdDrag?.();
    this.build?.abort();
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

  private async run(signal: AbortSignal) {
    if (signal.aborted) return;
    const widgets = this.enabled && [...this.widgets.values()].every(w => w.supported) ? new Map(this.widgets) : new Map<string, StripWidget>();
    const cache = tileCache(this.client);
    const levels = new Map<StripDensity, Set<number>>(([1, ...DENSITIES] as const).map(density => [density, new Set(this.lattice.hours(density))]));
    let candidate: StripDensity = 1, finest = 16;
    const readiness = (density = candidate) => pageReadiness(this.held, widgets.size, levels.get(density)!, density, this.budget);
    const received = () => {
      const level = readiness();
      if (!signal.aborted && level.state === 'over budget') this.status = 'not ready';
      return level.state;
    };
    const report = (detail?: unknown, lost = false) => {
      if (signal.aborted && !lost) return;
      perfLog?.strip({ ...this.counts, page: this.name, status: this.status, density: this.density,
        builtEnd: this.lattice.last, products: [...this.products], bytes: readiness().bytes, budgetBytes: this.budget, maxLateAsksPerWorker: 1, detail,
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
    const dropFine = () => Promise.all([...this.held.values()].filter(widget => widget.building !== 1).map(widget => drop(widget, true)));
    const fits = async (state = readiness().state) => {
      signal.throwIfAborted();
      if (state !== 'over budget') return true;
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
    const requestAt = (widget: HeldWidget, slot: Slot, hour: number) => widget.input.keyframe(hour).asks.find(ask => ask.slot === slot.name)!.request!;
    const read = async <T>(request: ProductRequest, group?: { product: string; keys: number; anchors: number }) => {
      let resuming = false;
      for (;;) {
        signal.throwIfAborted();
        const attempt: { signal?: AbortSignal } = {};
        try {
          const response = await this.gate.ask(request, () => {
            flight = new AbortController();
            attempt.signal = AbortSignal.any([signal, flight.signal]);
            if (group) {
              this.counts.groupsAsked++;
              if (resuming) this.counts.groupsResumed++;
              this.counts.anchorsAsked += group.anchors;
              this.counts.keyframesAsked += group.keys - group.anchors;
            } else {
              this.counts.tilesAsked++;
              this.counts.tilesStartedWhileScrubbing += Number(this.clock.getScrubPosition() !== null);
            }
            this.counts.startedWhilePlaying += Number(playing());
            this.counts.startedWhileSeeking += Number(seeking());
            this.counts.startedWhileForeground += Number(foreground());
            this.products.add(group?.product ?? request.product);
            return inBackground(() => baseClient(this.client).product<T>(request, attempt.signal!));
          }, signal, idle);
          attempt.signal!.throwIfAborted();
          return validateProductReply(response, request);
        } catch (error) {
          if (!attempt.signal?.aborted) throw error;
          if (group) {
            this.counts.groupsCancelled++;
            this.counts.asksDiscarded += group.keys;
          } else this.counts.tilesCancelled++;
          resuming = true;
          report();
        } finally { flight = null; }
      }
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
      let next = fine ? slot.fineNext.get(first) ?? 1 : slot.coarseNext, isolateUntil = 0;
      while (next < hours.length) {
        const start = Math.max(0, next - 1);
        const end = next < isolateUntil ? next + 1 : Math.min(hours.length, start + 17);
        let state: ReturnType<typeof received>;
        try {
          const group = hours.slice(start, end), anchors = Number(next !== 0) + Number(closesAtAnchor && end === hours.length);
          const bytes = await send(widget, slot, group, next === 0, anchors);
          signal.throwIfAborted();
          state = await update(widget, slot, { kind: 'append', slot: slot.name, hours: group, bytes, fine });
          next = end;
          if (fine) slot.fineNext.set(first, next); else slot.coarseNext = next;
          this.counts.anchorsCommitted += anchors;
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
        return fits();
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
        await Promise.all([...this.held.values()].map(widget => drop(widget, false)));
        this.held.clear();
        report();
      }
    } finally {
      stopClock(); stopScrub();
      stopForeground();
    }
  }
}

export const configuredStripPage = <T>(make: () => T): T | null => BUILD_TIMELINE_STRIPS ? make() : null;
export const StripPageContext = createContext<StripPage | null>(null);
