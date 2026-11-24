import { asFailure, type Failure } from '../dashboard/queryError.ts';
import {
  buildPlaybackBoundedRequest,
  buildStaticRequest,
  FamilySourceBroker,
  playbackIdleInterval,
  readsWithoutFollow,
  resolvePlaybackProvenInterval,
  type BrokerSlotHandle,
  type PlaybackBoundedOptions,
  type StaticRequestSpec,
} from './broker.ts';
import type { CompiledFollowSubscription, SourceEntry, SourcePlan, SourceTimeShape } from './plan.ts';
import type { FrozenGate } from './sources.ts';

export type Interval = readonly [lo: number, hi: number];

const EVERY_HOUR: Interval = [-Infinity, Infinity];
const NO_ENTRIES: readonly SourceEntry[] = [];

function boundedReadable(read: SourceTimeShape): boolean {
  return read.shape === 'point' || read.shape === 'range' || read.shape === 'branch-to-hour' || read.shape === 'trailing-window' || read.shape === 'paged-rows';
}

export function unitEntries(
  plan: SourcePlan | null,
  following: boolean,
  subscriptions: readonly CompiledFollowSubscription[],
): readonly SourceEntry[] {
  if (!plan || plan.refused || plan.unavailable) return NO_ENTRIES;
  const out = new Map<string, SourceEntry>();
  if (following) {
    const followed = new Set(subscriptions.map(sub => sub.name));
    for (const entry of plan.entries) {
      if (!out.has(entry.name) && !followed.has(entry.name) && readsWithoutFollow(entry.read)) out.set(entry.name, entry);
    }
  } else {
    const substitute = plan.staticEntry;
    if (substitute && boundedReadable(substitute.read)) out.set(substitute.name, substitute);
    for (const entry of plan.entries) {
      if (out.has(entry.name)) continue;
      if (substitute !== undefined ? readsWithoutFollow(entry.read) : boundedReadable(entry.read)) out.set(entry.name, entry);
    }
  }
  return out.size ? [...out.values()] : NO_ENTRIES;
}

export interface ReadUnit {
  readonly name: string;
  readonly entry: SourceEntry;
  readonly shift: number;
  readonly asked: number;
  readonly anchor: number;
  readonly spec: StaticRequestSpec | null;
  readonly key: string | null;
  readonly idle: Interval;
  readonly end: number;
}

export type UnitResult =
  | { readonly key: string; readonly data: unknown }
  | { readonly key: string; readonly failure: Failure };

export interface PinnedAsk {
  readonly timeMode: Parameters<typeof buildStaticRequest>[1]['timeMode'];
  readonly branch: { readonly firstHour: number; readonly lastHour: number };
  readonly window: { readonly firstHour: number; readonly lastHour: number } | null;
  readonly pinHour: number | null;
  readonly gate: FrozenGate;
  readonly countriesKnown?: (first: number, last: number) => boolean;
}

export type UnitAsk =
  | {
      readonly following: true;
      readonly trackOffset: number;
      readonly hour: number;
      readonly playback: (entry: SourceEntry) => Omit<PlaybackBoundedOptions, 'currentHour'>;
      readonly content: number;
    }
  | {
      readonly following: false;
      readonly trackOffset: number;
      readonly displayHour: number;
      readonly pinned: PinnedAsk;
      readonly content: number;
    };

function shifted(interval: Interval, by: number): Interval {
  return by === 0 ? interval : [interval[0] - by, interval[1] - by];
}

function unitKey(spec: StaticRequestSpec | null, content: number): string | null {
  return spec ? `${content}|${JSON.stringify(spec.request)}` : null;
}

export function askUnits(entries: readonly SourceEntry[], ask: UnitAsk): readonly ReadUnit[] {
  return entries.map(entry => {
    const shift = entry.offset - ask.trackOffset;
    if (ask.following) {
      const asked = ask.hour;
      const options = { ...ask.playback(entry), currentHour: asked + shift };
      const spec = buildPlaybackBoundedRequest(entry, options);
      return {
        name: entry.name,
        entry,
        shift,
        asked,
        anchor: spec ? spec.anchorHour : asked + shift,
        spec,
        key: unitKey(spec, ask.content),
        idle: spec ? EVERY_HOUR : shifted(playbackIdleInterval(entry, options), shift),
        end: options.effectiveEndHour,
      };
    }
    const pinned = ask.pinned;
    const pinHour = pinned.pinHour === null ? null : shift === 0 ? pinned.pinHour : pinned.gate.recorded(pinned.pinHour + shift);
    const window = pinned.window && shift !== 0
      ? { firstHour: pinned.window.firstHour + shift, lastHour: pinned.window.lastHour + shift }
      : pinned.window;
    const spec = buildStaticRequest(entry, { ...pinned, window, pinHour: pinHour as Parameters<typeof buildStaticRequest>[1]['pinHour'] });
    return {
      name: entry.name,
      entry,
      shift,
      asked: ask.displayHour,
      anchor: pinHour ?? ask.displayHour + shift,
      spec,
      key: unitKey(spec, ask.content),
      idle: EVERY_HOUR,
      end: Infinity,
    };
  });
}

export type UnitEvidence =
  | { readonly kind: 'pending' }
  | { readonly kind: 'failed'; readonly failure: Failure }
  | { readonly kind: 'held'; readonly data: unknown; readonly proof: Interval };

export function unitEvidence(
  unit: ReadUnit,
  result: UnitResult | undefined,
  following: boolean,
): UnitEvidence {
  if (!unit.spec) return { kind: 'held', data: undefined, proof: unit.idle };
  if (!result || result.key !== unit.key) return { kind: 'pending' };
  if ('failure' in result) return { kind: 'failed', failure: result.failure };
  if (!following) return { kind: 'held', data: result.data, proof: EVERY_HOUR };
  const proof = resolvePlaybackProvenInterval(unit.entry.read, result.data, { anchorHour: unit.anchor, span: unit.spec.span }, unit.end);
  return { kind: 'held', data: result.data, proof: shifted(proof, unit.shift) };
}

export interface UnitBundle {
  readonly hour: number;
  readonly span: Interval | null;
  readonly proof: Interval | null;
  readonly panels: ReadonlyMap<string, unknown>;
  readonly shifts: Readonly<Record<string, number>> | null;
}

export type Settlement =
  | { readonly kind: 'none' }
  | { readonly kind: 'outside' }
  | { readonly kind: 'failed'; readonly name: string; readonly failure: Failure }
  | { readonly kind: 'hold' }
  | { readonly kind: 'complete'; readonly bundle: UnitBundle };

export interface SettleInput {
  readonly following: boolean;
  readonly moving: boolean;
  readonly hour: number;
  readonly subscribed: boolean;
}

function answeredHour(unit: ReadUnit, data: unknown): number {
  const spec = unit.spec!;
  const read = unit.entry.read;
  if (spec.span) return spec.span[1] - unit.shift;
  if (read.shape === 'point' && !(read.channel === 'panel' && !read.exactProduct)) {
    const hour = (data as { hour?: unknown } | null)?.hour;
    if (typeof hour === 'number') return hour - unit.shift;
  }
  return unit.anchor - unit.shift;
}

export function settleUnits(
  units: readonly ReadUnit[],
  results: ReadonlyMap<string, UnitResult>,
  input: SettleInput,
): Settlement {
  if (!units.length) return { kind: 'none' };
  if (!input.subscribed && !units.some(unit => unit.spec)) return { kind: 'outside' };
  let lo = -Infinity;
  let hi = Infinity;
  let spanLo = Infinity;
  let spanHi = -Infinity;
  let answered = -Infinity;
  let pending = false;
  const panels = new Map<string, unknown>();
  let shifts: Record<string, number> | null = null;
  for (const unit of units) {
    const evidence = unitEvidence(unit, results.get(unit.name), input.following);
    if (evidence.kind === 'failed') return { kind: 'failed', name: unit.name, failure: evidence.failure };
    if (evidence.kind === 'pending') {
      pending = true;
      continue;
    }
    lo = Math.max(lo, evidence.proof[0]);
    hi = Math.min(hi, evidence.proof[1]);
    if (!unit.spec) continue;
    panels.set(unit.name, evidence.data);
    if (unit.shift !== 0) (shifts ??= {})[unit.name] = unit.shift;
    if (unit.spec.span) {
      spanLo = Math.min(spanLo, unit.spec.span[0] - unit.shift);
      spanHi = Math.max(spanHi, unit.spec.span[1] - unit.shift);
    }
    answered = Math.max(answered, answeredHour(unit, evidence.data));
  }
  if (pending) return { kind: 'hold' };
  const span: Interval | null = spanLo <= spanHi ? [spanLo, spanHi] : null;
  if (!input.following) {
    return { kind: 'complete', bundle: { hour: Number.isFinite(answered) ? answered : input.hour, span, proof: null, panels, shifts } };
  }
  if (lo > hi || lo > input.hour || (!input.moving && input.hour > hi)) return { kind: 'hold' };
  const hour = Number.isFinite(lo) ? lo : Math.min(input.hour, hi);
  return { kind: 'complete', bundle: { hour, span, proof: [lo, hi], panels, shifts } };
}

export interface ShownBundle<B, F> {
  readonly sig: string;
  readonly bundle: B;
  readonly frozen: F;
}

export function showBundle<B, F>(
  last: ShownBundle<B, F> | null,
  bundle: B | null,
  held: boolean,
  sig: string,
  current: F,
): { readonly last: ShownBundle<B, F> | null; readonly shown: ShownBundle<B, F> | null } {
  if (bundle === null) return { last, shown: null };
  if (held && last?.bundle === bundle && last.sig === sig) return { last, shown: last };
  const shown = last?.bundle === bundle && last.frozen === current && last.sig === sig ? last : { sig, bundle, frozen: current };
  return { last: shown, shown };
}

function jointProof(
  units: readonly ReadUnit[],
  results: ReadonlyMap<string, UnitResult>,
): Interval | null {
  let lo = -Infinity;
  let hi = Infinity;
  for (const unit of units) {
    const evidence = unitEvidence(unit, results.get(unit.name), true);
    if (evidence.kind !== 'held') return null;
    lo = Math.max(lo, evidence.proof[0]);
    hi = Math.min(hi, evidence.proof[1]);
  }
  return [lo, hi];
}

const NO_RESULTS: ReadonlyMap<string, UnitResult> = new Map();

export class UnitReads {
  private readonly broker = new FamilySourceBroker();
  private readonly flights = new Map<string, { readonly key: string; readonly handle: BrokerSlotHandle }>();
  private readonly restart = new Set<string>();
  private held: ReadonlyMap<string, UnitResult> = NO_RESULTS;
  private synced: { readonly units: readonly ReadUnit[]; readonly active: boolean } = { units: [], active: false };
  private readonly load: (unit: ReadUnit, signal: AbortSignal, restart: boolean) => Promise<unknown>;
  private readonly changed: () => void;
  private readonly peek: ((unit: ReadUnit) => unknown) | undefined;

  constructor(
    load: (unit: ReadUnit, signal: AbortSignal, restart: boolean) => Promise<unknown>,
    changed: () => void,
    peek?: (unit: ReadUnit) => unknown,
  ) {
    this.load = load;
    this.changed = changed;
    this.peek = peek;
  }

  get results(): ReadonlyMap<string, UnitResult> {
    return this.held;
  }

  get inFlight(): number {
    return this.flights.size;
  }

  take(units: readonly ReadUnit[]): ReadonlyMap<string, UnitResult> {
    if (!this.peek) return this.held;
    let next: Map<string, UnitResult> | null = null;
    for (const unit of units) {
      if (!unit.key || this.held.get(unit.name)?.key === unit.key) continue;
      const data = this.peek(unit);
      if (data !== undefined) (next ??= new Map(this.held)).set(unit.name, { key: unit.key, data });
    }
    if (next) this.held = next;
    return this.held;
  }

  sync(units: readonly ReadUnit[], active: boolean, retryFailed = false): void {
    this.synced = { units, active };
    const wanted = new Set<string>();
    if (active) {
      for (const unit of units) {
        if (!unit.key) continue;
        wanted.add(unit.name);
        const flight = this.flights.get(unit.name);
        const result = this.held.get(unit.name);
        if (result?.key === unit.key) {
          if (!retryFailed || !('failure' in result)) {
            if (flight) this.stop(unit.name);
            continue;
          }
          const next = new Map(this.held);
          next.delete(unit.name);
          this.held = next;
          this.restart.add(unit.name);
        } else if (flight?.key === unit.key) continue;
        this.start(unit);
      }
    }
    for (const name of [...this.flights.keys()]) if (!wanted.has(name)) this.stop(name);
    if (!active) return;
    const names = new Set(units.map(unit => unit.name));
    let pruned: Map<string, UnitResult> | null = null;
    for (const name of this.held.keys()) if (!names.has(name)) (pruned ??= new Map(this.held)).delete(name);
    if (pruned) this.held = pruned;
  }

  retry(): void {
    let next: Map<string, UnitResult> | null = null;
    for (const [name, result] of this.held) {
      if (!('failure' in result)) continue;
      (next ??= new Map(this.held)).delete(name);
      this.restart.add(name);
    }
    if (!next) return;
    this.held = next;
    this.sync(this.synced.units, this.synced.active);
    this.changed();
  }

  dispose(): void {
    this.flights.clear();
    this.broker.dispose();
  }

  private start(unit: ReadUnit): void {
    const { name } = unit;
    const key = unit.key!;
    const restart = this.restart.delete(name);
    const settle = (result: UnitResult) => {
      if (this.flights.get(name)?.key !== key) return;
      this.flights.delete(name);
      this.broker.cancelSlot(name);
      this.held = new Map(this.held).set(name, result);
      this.changed();
    };
    const handle = this.broker.runSlot(
      name,
      key,
      signal => this.load(unit, signal, restart),
      data => settle({ key, data }),
      error => settle({ key, failure: asFailure(error) }),
    );
    this.flights.set(name, { key, handle });
  }

  private stop(name: string): void {
    this.flights.get(name)?.handle.cancel();
    this.flights.delete(name);
  }
}

export interface TargetClock {
  now(): number;
  playing(): boolean;
  seek(): number;
  subscribe(changed: () => void): () => void;
  watch(interval: Interval, left: () => void): () => void;
}

interface TargetCommon {
  readonly entries: readonly SourceEntry[];
  readonly active: boolean;
  readonly content: number;
  readonly trackOffset: number;
}

export type TargetConfig = TargetCommon & (
  | { readonly following: true; readonly subscribed: boolean; readonly playback: (entry: SourceEntry) => Omit<PlaybackBoundedOptions, 'currentHour'> }
  | { readonly following: false; readonly displayHour: number; readonly pinned: PinnedAsk }
);

export interface TargetBundle {
  readonly panels: ReadonlyMap<string, unknown>;
  readonly shifts: Readonly<Record<string, number>> | null;
  readonly span: Interval | null;
  readonly hour: number;
  readonly follows: boolean;
}

export interface TargetView {
  readonly kind: Settlement['kind'];
  readonly failure: { readonly name: string; readonly failure: Failure } | null;
  readonly bundle: TargetBundle | null;
  readonly held: boolean;
}

export interface TargetProof {
  readonly hour: number;
  readonly proof: Interval;
}

const NOTHING: TargetView = { kind: 'none', failure: null, bundle: null, held: false };

function sameConfig(a: TargetConfig, b: TargetConfig): boolean {
  if (a.entries !== b.entries || a.active !== b.active || a.content !== b.content || a.trackOffset !== b.trackOffset) return false;
  if (a.following && b.following) return a.subscribed === b.subscribed && a.playback === b.playback;
  if (!a.following && !b.following) return a.displayHour === b.displayHour && a.pinned === b.pinned;
  return false;
}

function sameReplies(a: ReadonlyMap<string, unknown>, b: ReadonlyMap<string, unknown>): boolean {
  if (a.size !== b.size) return false;
  for (const [name, data] of b) if (!a.has(name) || a.get(name) !== data) return false;
  return true;
}

export class ReadTarget {
  private readonly clock: TargetClock;
  private readonly reads: UnitReads;
  private readonly listeners = new Set<() => void>();
  private readonly proofListeners = new Set<() => void>();
  private config: TargetConfig | null = null;
  private hour = NaN;
  private units: readonly ReadUnit[] = [];
  private last: TargetBundle | null = null;
  private lastProof: TargetProof | null = null;
  private shownProof: TargetProof | null = null;
  private current: TargetView = NOTHING;
  private armed: Interval | null = null;
  private stopWatch: (() => void) | null = null;
  private stopClock: (() => void) | null = null;
  private seekSeen = NaN;
  private settling = false;
  private disposed = false;

  constructor(
    clock: TargetClock,
    load: (unit: ReadUnit, signal: AbortSignal, restart: boolean) => Promise<unknown>,
    peek?: (unit: ReadUnit) => unknown,
  ) {
    this.clock = clock;
    this.reads = new UnitReads(load, () => this.settle(), peek);
  }

  get target(): number {
    return this.hour;
  }

  get inFlight(): number {
    return this.reads.inFlight;
  }

  readonly view = (): TargetView => this.current;

  readonly proof = (): TargetProof | null => this.shownProof;

  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };

  readonly subscribeProof = (listener: () => void): (() => void) => {
    this.proofListeners.add(listener);
    return () => { this.proofListeners.delete(listener); };
  };

  configure(config: TargetConfig): void {
    if (this.disposed) return;
    const was = this.config;
    if (was && sameConfig(was, config)) return;
    this.config = config;
    if (!was || was.entries !== config.entries || was.following !== config.following) {
      this.last = null;
      this.lastProof = null;
    }
    if (config.following && !this.stopClock) {
      this.seekSeen = this.clock.seek();
      this.stopClock = this.clock.subscribe(() => this.anchored());
    } else if (!config.following && this.stopClock) {
      this.stopClock();
      this.stopClock = null;
    }
    if (config.following) this.retarget(this.clock.now(), false, false);
    else this.retarget(config.displayHour, config.displayHour !== this.hour, false);
  }

  retry(): void {
    this.reads.retry();
  }

  dispose(): void {
    this.disposed = true;
    this.disarm();
    this.stopClock?.();
    this.stopClock = null;
    this.reads.dispose();
    this.listeners.clear();
    this.proofListeners.clear();
  }

  private anchored(): void {
    const seek = this.clock.seek();
    if (seek !== this.seekSeen) {
      this.seekSeen = seek;
      const now = this.clock.now();
      this.retarget(now, true, true);
      return;
    }
    this.settle();
  }

  private retarget(hour: number, retryFailed: boolean, keep: boolean): void {
    const config = this.config!;
    this.hour = hour;
    if (!config.entries.length) this.units = [];
    else if (!config.following) {
      this.units = askUnits(config.entries, { following: false, trackOffset: config.trackOffset, displayHour: config.displayHour, pinned: config.pinned, content: config.content });
    } else {
      const asked = askUnits(config.entries, { following: true, trackOffset: config.trackOffset, hour, playback: config.playback, content: config.content });
      this.units = keep ? this.keepProven(asked, hour) : asked;
    }
    this.reads.sync(this.units, config.active, retryFailed);
    this.settle();
  }

  private keepProven(asked: readonly ReadUnit[], hour: number): readonly ReadUnit[] {
    const before = new Map(this.units.map(unit => [unit.name, unit]));
    const results = this.reads.results;
    return asked.map(unit => {
      const prev = before.get(unit.name);
      if (!prev || prev.entry !== unit.entry || prev.key === unit.key) return unit;
      const evidence = unitEvidence(prev, results.get(prev.name), true);
      return evidence.kind === 'held' && evidence.proof[0] <= hour && hour <= evidence.proof[1] ? prev : unit;
    });
  }

  private settle(): void {
    const config = this.config;
    if (!config || this.disposed || this.settling) return;
    this.settling = true;
    try {
      const results = this.reads.take(this.units);
      const following = config.following;
      const moving = following && this.clock.playing();
      const now = following ? this.clock.now() : config.displayHour;
      const settlement = settleUnits(this.units, results, {
        following,
        moving,
        hour: now,
        subscribed: following && config.subscribed,
      });
      let bundle: TargetBundle | null = null;
      let failure: TargetView['failure'] = null;
      if (settlement.kind === 'complete') {
        const made = settlement.bundle;
        const kept = this.last && sameReplies(this.last.panels, made.panels) && (following || this.last.hour === made.hour) ? this.last : null;
        bundle = kept ?? { panels: made.panels, shifts: made.shifts, span: made.span, hour: made.hour, follows: following };
        this.last = bundle;
        this.lastProof = made.proof ? { hour: made.hour, proof: made.proof } : null;
      } else if (settlement.kind === 'failed') {
        failure = this.current.failure?.name === settlement.name && this.current.failure.failure === settlement.failure
          ? this.current.failure
          : { name: settlement.name, failure: settlement.failure };
      }
      const holding = bundle === null && (settlement.kind === 'hold' || settlement.kind === 'failed') ? this.held() : null;
      const shown = bundle ?? holding;
      this.showProof(shown ? this.lastProof : null);
      const kind = settlement.kind !== 'failed' && shown ? 'complete' : settlement.kind;
      this.publish({ kind, failure, bundle: shown, held: holding !== null && !moving });
      if (following && config.active) this.arm(results, now, settlement.kind === 'failed');
      else this.disarm();
    } finally {
      this.settling = false;
    }
  }

  private held(): TargetBundle | null {
    const last = this.last;
    if (!last) return null;
    const dated = this.lastProof ? this.lastProof.hour : last.hour;
    return dated > this.hour ? null : last;
  }

  private arm(results: ReadonlyMap<string, UnitResult>, now: number, failed: boolean): void {
    let watch: Interval | null;
    if (failed) watch = [this.hour, this.hour];
    else {
      watch = jointProof(this.units, results);
      if (watch === null) {
        this.disarm();
        return;
      }
      if (watch[0] > watch[1] || this.hour < watch[0] || this.hour > watch[1]) watch = [this.hour, this.hour];
    }
    if (now < watch[0] || now > watch[1]) {
      this.disarm();
      queueMicrotask(() => this.leave());
      return;
    }
    if (watch[0] === -Infinity && watch[1] === Infinity) {
      this.disarm();
      return;
    }
    if (this.armed && this.armed[0] === watch[0] && this.armed[1] === watch[1]) return;
    this.disarm();
    this.armed = watch;
    this.stopWatch = this.clock.watch(watch, () => this.leave());
  }

  private leave(): void {
    if (this.disposed || !this.config?.following) return;
    const now = this.clock.now();
    if (now === this.hour) return;
    this.disarm();
    this.retarget(now, false, true);
  }

  private disarm(): void {
    this.stopWatch?.();
    this.stopWatch = null;
    this.armed = null;
  }

  private showProof(proof: TargetProof | null): void {
    const was = this.shownProof;
    if (was === proof || (was && proof && was.hour === proof.hour && was.proof[0] === proof.proof[0] && was.proof[1] === proof.proof[1])) return;
    this.shownProof = proof;
    for (const listener of [...this.proofListeners]) listener();
  }

  private publish(next: TargetView): void {
    const was = this.current;
    if (was.kind === next.kind && was.failure === next.failure && was.bundle === next.bundle && was.held === next.held) return;
    this.current = next;
    for (const listener of [...this.listeners]) listener();
  }
}
