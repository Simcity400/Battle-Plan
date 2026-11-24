import { asFailure, type Failure } from '../dashboard/queryError.ts';
import {
  buildPlaybackBoundedRequest,
  buildStaticRequest,
  FamilySourceBroker,
  playbackIdleInterval,
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

function readsWithoutFollow(read: SourceTimeShape): boolean {
  if (read.shape === 'range') return true;
  return (read.shape === 'branch-to-hour' || read.shape === 'trailing-window' || read.shape === 'point') && read.followQuery === null;
}

function boundedReadable(read: SourceTimeShape): boolean {
  return read.shape === 'point' || read.shape === 'range' || read.shape === 'branch-to-hour' || read.shape === 'trailing-window';
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
      readonly askedAt: (name: string) => number;
      readonly playback: Omit<PlaybackBoundedOptions, 'currentHour'>;
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
      const asked = ask.askedAt(entry.name);
      const options = { ...ask.playback, currentHour: asked + shift };
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
    };
  });
}

export type UnitEvidence =
  | { readonly kind: 'pending' }
  | { readonly kind: 'failed'; readonly failure: Failure; readonly watch: Interval }
  | { readonly kind: 'held'; readonly data: unknown; readonly proof: Interval };

export function unitEvidence(
  unit: ReadUnit,
  result: UnitResult | undefined,
  following: boolean,
  endHour: number,
): UnitEvidence {
  if (!unit.spec) return { kind: 'held', data: undefined, proof: unit.idle };
  if (!result || result.key !== unit.key) return { kind: 'pending' };
  if ('failure' in result) return { kind: 'failed', failure: result.failure, watch: following ? [unit.asked, unit.asked] : EVERY_HOUR };
  if (!following) return { kind: 'held', data: result.data, proof: EVERY_HOUR };
  const proof = resolvePlaybackProvenInterval(unit.entry.read, result.data, { anchorHour: unit.anchor, span: unit.spec.span }, endHour);
  return { kind: 'held', data: result.data, proof: shifted(proof, unit.shift) };
}

export interface UnitBundle {
  readonly hour: number;
  readonly span: Interval | null;
  readonly proof: Interval | null;
  readonly panels: ReadonlyMap<string, unknown>;
}

export type Settlement =
  | { readonly kind: 'none' }
  | { readonly kind: 'outside' }
  | { readonly kind: 'failed'; readonly name: string; readonly failure: Failure }
  | { readonly kind: 'hold' }
  | { readonly kind: 'complete'; readonly bundle: UnitBundle };

export interface SettleInput {
  readonly following: boolean;
  readonly hour: number;
  readonly subscribed: boolean;
  readonly endHour: number;
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
  for (const unit of units) {
    const evidence = unitEvidence(unit, results.get(unit.name), input.following, input.endHour);
    if (evidence.kind === 'failed') return { kind: 'failed', name: unit.name, failure: evidence.failure };
    if (evidence.kind === 'pending') {
      pending = true;
      continue;
    }
    lo = Math.max(lo, evidence.proof[0]);
    hi = Math.min(hi, evidence.proof[1]);
    if (!unit.spec) continue;
    panels.set(unit.name, evidence.data);
    if (unit.spec.span) {
      spanLo = Math.min(spanLo, unit.spec.span[0] - unit.shift);
      spanHi = Math.max(spanHi, unit.spec.span[1] - unit.shift);
    }
    answered = Math.max(answered, answeredHour(unit, evidence.data));
  }
  if (pending) return { kind: 'hold' };
  const span: Interval | null = spanLo <= spanHi ? [spanLo, spanHi] : null;
  if (!input.following) {
    return { kind: 'complete', bundle: { hour: Number.isFinite(answered) ? answered : input.hour, span, proof: null, panels } };
  }
  if (lo > hi || lo > input.hour) return { kind: 'hold' };
  const hour = Number.isFinite(lo) ? lo : Math.min(input.hour, hi);
  return { kind: 'complete', bundle: { hour, span, proof: [lo, hi], panels } };
}

export interface ShownBundle<B, F> {
  readonly sig: string;
  readonly bundle: B;
  readonly frozen: F;
}

export function showBundle<B, F>(
  last: ShownBundle<B, F> | null,
  settlement: Settlement,
  complete: B | null,
  sig: string,
  current: F,
): { readonly last: ShownBundle<B, F> | null; readonly shown: ShownBundle<B, F> | null; readonly held: boolean } {
  if (complete !== null) {
    const shown = last?.bundle === complete && last.frozen === current && last.sig === sig ? last : { sig, bundle: complete, frozen: current };
    return { last: shown, shown, held: false };
  }
  const kept = (settlement.kind === 'hold' || settlement.kind === 'failed') && last?.sig === sig ? last : null;
  return { last, shown: kept, held: kept !== null };
}

export function unitWatch(unit: ReadUnit, result: UnitResult | undefined, endHour: number): Interval | null {
  const evidence = unitEvidence(unit, result, true, endHour);
  if (evidence.kind === 'pending') return null;
  return evidence.kind === 'failed' ? evidence.watch : evidence.proof;
}

export function watchSpan(
  units: readonly ReadUnit[],
  results: ReadonlyMap<string, UnitResult>,
  endHour: number,
): Interval | null {
  let lo = -Infinity;
  let hi = Infinity;
  let any = false;
  for (const unit of units) {
    const watch = unitWatch(unit, results.get(unit.name), endHour);
    if (!watch) continue;
    any = true;
    lo = Math.max(lo, watch[0]);
    hi = Math.min(hi, watch[1]);
  }
  return any ? [lo, hi] : null;
}

export function reaskedUnits(
  units: readonly ReadUnit[],
  results: ReadonlyMap<string, UnitResult>,
  hour: number,
  endHour: number,
): readonly string[] {
  const out: string[] = [];
  for (const unit of units) {
    const watch = unitWatch(unit, results.get(unit.name), endHour);
    if (watch && (hour < watch[0] || hour > watch[1])) out.push(unit.name);
  }
  return out;
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

  sync(units: readonly ReadUnit[], active: boolean): void {
    this.synced = { units, active };
    const wanted = new Set<string>();
    if (active) {
      for (const unit of units) {
        if (!unit.key) continue;
        wanted.add(unit.name);
        const flight = this.flights.get(unit.name);
        if (this.held.get(unit.name)?.key === unit.key) {
          if (flight) this.stop(unit.name);
          continue;
        }
        if (flight?.key === unit.key) continue;
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
