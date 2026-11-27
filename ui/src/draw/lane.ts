import { Facts, changeWindow, followsChannel, joinComponents, samePicture, type Channel, type PanelRecord, type Picture, type Window } from '../dashboard/follow/facts.ts';
import { SAME, type Ack, type Bitmap, type Entry, type FailureOrigin, type FrameRequest, type Refusal } from '../dashboard/follow/page.ts';
import { parseAnswer, parseDatedPanelRecord, parseFollowedFrame, type FollowedAnswer } from '../dashboard/follow/message.ts';
import { patchAnswer } from '../dashboard/follow/tables.ts';
import { sectionOf } from '../dashboard/follow/liveCells.ts';
import { PayloadStore, type PayloadsMessage, type Revisions } from '../dashboard/follow/payloads.ts';
import { snapCanvasText } from './kit.ts';
import type { InstalledViewport } from '../families/plan.ts';
import { EMPTY_STRIP_RECEIPT, WidgetStrips, type StripUpdate } from '../dashboard/strip/store.ts';
import { stripCoverageAt, type StripFrame, type StripScenePlan, type StripWork } from '../dashboard/strip/scrub.ts';
import type { Outside } from '../dashboard/regionTime.ts';
import type { SceneProjection, SceneReaders } from '../families/scene.ts';
import { exactPart, exactStatics, type ExactPart, type ExactStatics } from '../families/exactScene.ts';
import { IdentityIndex } from '../families/identity/index.ts';
import { createSnapshotIdentityProvider } from '../families/identity/provider.ts';
import { identityKeyString, parseEntityKey, type IdentityAsk, type IdentitySnapshot } from '../families/identity/types.ts';
import type { RowWindow, TailValidator } from '../families/table/rowSource.ts';
import type { LaneSlotsPayload, SharedSurfaceSlot } from './laneMessages';

export interface WorkerCanvasInfo {
  readonly canvas: OffscreenCanvas;
  readonly opaque: boolean;
  readonly dirty?: boolean;
}

export interface SharedLaneCanvas {
  readonly canvas: OffscreenCanvas;
  readonly ctx: OffscreenCanvasRenderingContext2D;
  readonly rawSetTransform: OffscreenCanvasRenderingContext2D['setTransform'];
  readonly opaque: boolean;
  width: number;
  height: number;
  dpr: number;
  scrollTop: number;
  scrollLeft: number;
  originX: number;
  originY: number;
  surfaceColor: string;
  readonly activeSlot: {
    ox: number;
    oy: number;
    w: number;
    h: number;
    dpr: number;
    surfaceColor: string;
    widgetKey: object;
  };
  readonly slots: Map<number, SharedSurfaceSlot>;
  readonly dirtySlots: Set<number>;
}

export interface PanelValue { kind: 'panel'; bytes: Uint8Array | null; data: unknown; refused: string | null; stopped?: string }
export interface AnswerValue { kind: 'answer'; body?: Uint8Array; answer: FollowedAnswer }
export interface SeedValue { kind: 'seed'; values: readonly (number | string)[] }
export type Value = PanelValue | AnswerValue | SeedValue;

export interface Sub { id: number; channel: Channel; product?: string; route: number | null }

export interface Surface { width: number; height: number; dpr: number }
export interface Dated { h: number; c: number }

export interface Binding {
  generation: number; owner?: string; surfaces: readonly Surface[]; offset: number; pin: number | null; settings?: unknown; dated?: Dated;
}

export interface Supply {
  stamp?: number;
  statics?: unknown;
  identity?: number | null;
  dated?: Dated | null;
}
interface Resolved {
  stamp?: number;
  statics?: Readonly<Record<string, unknown>> | null;
  identity?: { readonly id: number; readonly snapshot: unknown } | null;
  dated?: Dated | null;
}
export interface Bitmapped { readonly width: number; readonly height: number }

function withoutDated(binding: Binding): Binding {
  const { dated: _, ...rest } = binding;
  return rest;
}

export interface Scene { widget: number; subs: readonly Sub[]; pictures: readonly Picture<Value>[]; model: unknown; owner?: string }

export interface PaintInput extends Scene {
  hour: number;
  seek: number;
  speed: number;
  binding: Binding;
  inspect: string | null;
  now: number;
  workerCanvas?: WorkerCanvasInfo;
  sharedSurface?: { readonly lane: SharedLaneCanvas; readonly slot: SharedSurfaceSlot; readonly dirty: boolean };
}

export interface Painted {
  surfaces: readonly ({ source: Bitmapped; content: string } | null)[];
  surfaceContent?: string;
  fields: Readonly<Record<string, { value: unknown; content: string }>>;
  motion: boolean;
  repainted?: boolean;
  ephemeral?: boolean;
  mode?: 'picture' | 'refused';
  reason?: string;
  nextChangeAt?: number;
  waiting?: true;
  viewport?: InstalledViewport;
  hit?: (point: readonly [number, number]) => string | null;
}

export interface LaneView {
  next?(scene: Scene, hour: number, state?: Map<string, unknown>): number | null;
  paint(input: PaintInput, state: Map<string, unknown>, content: () => string): Painted;
  onSpeed?(speed: number, state: Map<string, unknown>): void;
  unmount?(widget: number, state: Map<string, unknown>): void;
  prepare?(): Promise<void>;
  scrub?(scene: Scene, statics: (validateTail?: TailValidator, window?: RowWindow) => ScrubInput, state: Map<string, unknown>, readers?: StripReaders): ScrubScene;
}

type StripReaders = Required<Pick<SceneReaders, 'index' | 'identity'>>;
type LaneStrip = { build: number; held: WidgetStrips; identities?: { coarse?: StripReaders; fine?: StripReaders; active?: StripReaders; activeFine?: boolean } };

export type ScrubInput = { mode: 'picture'; statics: ExactStatics; options?: unknown } | { mode: 'missing' | 'waiting' } | { mode: 'refused'; reason: string };
export type ScrubScene = Exclude<ScrubInput, { mode: 'picture' }> | { mode: 'outside'; outside: Outside }
  | { mode: 'failed'; reason: string }
  | { mode: 'picture'; picture: SceneProjection<unknown> };
export type ScrubSample = StripWork & { widget: number; mode: ScrubScene['mode']; error?: string };

export interface Presented<B extends Bitmap> {
  lane: number;
  serial: number;
  resets: [number, number][];
  closed?: [widget: number, lo: number, hi: number][];
  withdrawn?: [widget: number, binding: number][];
  confirmed?: [number, 'followed' | 'panel', number, number, number][];
  entries: Entry<B>[];
}

export interface LaneHost<B extends Bitmap> {
  lane: number;
  lanes: number;
  views(kind: string): LaneView;
  crop(sources: readonly Bitmapped[]): Promise<B[]>;
  present(presentation: Presented<B>, transfer: B[]): void;
  release(bytes: number): void;
  fail(reason: string): void;
  failSub?(sub: number, reason: string): void;
  waitSub?(sub: number, waiting: boolean): void;
  failWidget?(widget: number, reason: string, stamp?: number): void;
  failLaneSurface?(): void;
  later(run: () => void): void;
  schedule?(run: () => void): void;
  hit?(widget: number, key: string | null, installedId?: number, owner?: string): void;
  served?(frame: { seek: number; target: number; exact: boolean; changed: number; ms: number; cancelled?: boolean; costs?: [widget: number, cost: number][] }): void;
  scrubbed?(frame: StripFrame, samples: readonly ScrubSample[]): void;
  budget?: number;
  earlyBuffer?: number;
  clock?: () => number;
  stretch?: (ms: number) => void;
}

interface Sent {
  id: number; owner?: string; request: number; seek: number; ephemeral: boolean; pictures: Picture<Value>[]; h: number; c: number; lo: number; hi: number; binding: number;
  contents: Record<string, string>; fieldKeys: string[]; surfaceCount: number; mode: Entry['mode']; reason?: string;
  open?: 'followed' | 'panel'; viewport?: InstalledViewport; inspect: string | null; hit?: Painted['hit']; supply: number;
  model: unknown; bindingSnapshot: Binding; subsSnapshot: readonly Sub[];
}

interface Supplied { stamp: number | undefined; statics: Readonly<Record<string, unknown>> | null; identity: { readonly id: number; readonly snapshot: unknown } | null }

interface Widget {
  id: number; order: number; subs: number[]; view: string; model: unknown; own: unknown; supplied: Supplied; supply: number; binding: Binding;
  installed: Sent | null; shown: Sent | null; inflight: Sent | null; sent: Record<string, string>; forceFull: boolean; motion: boolean; inspect: string | null;
  validWindow: Window | null;
  start: { version: string; hour: number } | null; state: Map<string, unknown>; cost: number; served: number; deferrals: number; halted: number | null; parked: Sent | null;
  withdrawn: number | null;
}

interface Held { kind: 'follow' | 'panels'; native: number; seek: number; hour: number; id: number; records: Uint8Array[]; size: number; completed?: number }

const charged = (record: Uint8Array) => {
  const view = new DataView(record.buffer, record.byteOffset, record.byteLength), length = view.getUint32(8, true);
  return 16 + 2 * Math.max(length, view.getUint32(12, true));
};

const recordsCharge = (bytes: Uint8Array) => {
  if (bytes.byteLength < 24) return 0;
  const data = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let total = 0;
  for (let i = 0, at = 24, count = data.getUint32(12, true); i < count; i++) {
    if (at + 16 > bytes.byteLength) break;
    const length = data.getUint32(at + 8, true);
    if (at + 16 + length > bytes.byteLength) break;
    total += charged(bytes.subarray(at, at + 16 + length));
    at += 16 + length;
  }
  return total;
};

const panelRecordsCharge = (bytes: Uint8Array) => {
  if (bytes.byteLength < 24) return 0;
  const data = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let total = 0;
  for (let i = 0, at = 24, count = data.getUint32(8, true); i < count; i++) {
    if (at + 16 > bytes.byteLength) break;
    const kind = data.getUint8(at + 8), length = data.getUint32(at + 12, true), size = ((16 + length + 7) & ~7);
    if (at + size > bytes.byteLength) break;
    if (kind !== 4) total += 2 * size;
    at += size;
  }
  return total;
};

const followRecordSubs = (bytes: Uint8Array): number[] => {
  if (bytes.byteLength < 24) return [];
  const data = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength), out: number[] = [];
  for (let i = 0, at = 24, count = data.getUint32(12, true); i < count; i++) {
    if (at + 16 > bytes.byteLength) break;
    const id = data.getUint32(at, true), length = data.getUint32(at + 8, true);
    if (at + 16 + length > bytes.byteLength) break;
    if (id < CELL) out.push(id);
    at += 16 + length;
  }
  return out;
};

const panelRecordSubs = (bytes: Uint8Array): number[] => {
  if (bytes.byteLength < 24) return [];
  const data = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength), out: number[] = [];
  for (let i = 0, at = 24, count = data.getUint32(8, true); i < count; i++) {
    if (at + 16 > bytes.byteLength) break;
    const id = data.getUint32(at, true), kind = data.getUint8(at + 8), length = data.getUint32(at + 12, true), size = ((16 + length + 7) & ~7);
    if (at + size > bytes.byteLength) break;
    if (kind !== 4 && id < CELL) out.push(id);
    at += size;
  }
  return out;
};

const isWholeFollowRecord = (record: Uint8Array) => record.byteLength > 16 && record[16] !== 3;

const refusalOf = (picture: Picture<Value>): string | null => {
  const value = picture.parts[0]!.value;
  if (value.kind === 'answer') return value.answer.kind === 'notRecorded' ? value.answer.reason : null;
  return value.kind === 'panel' ? value.refused : null;
};

const stopOf = (picture: Picture<Value>): string | null => {
  const value = picture.parts[0]!.value;
  return value.kind === 'panel' ? value.stopped ?? null : null;
};

const BUFFER = 8 << 20;
const HELD_COST = 32;
const STALE = 120;
const CELL = 0x8000_0000;
const LOST = 'Answers that arrived before this reading was registered overflowed the early buffer and were lost';
const EARLY_OVERFLOW = 'Early packets for a future seek overflowed the lane buffer and were lost';

type Selected = { pictures: Picture<Value>[]; window: Window; h: number; c: number };

const now = () => performance.now();

function cellsOf(data: unknown): { id: number; seedHour?: number; values?: Value }[] {
  const section = sectionOf(data);
  if (!section) return [];
  return section.groups.flatMap(group => group.follow === undefined ? []
    : [{ id: group.follow, ...group.seedHour !== undefined && group.values ? { seedHour: group.seedHour, values: { kind: 'seed' as const, values: group.values } } : {} }]);
}

export class Lane<B extends { readonly width: number; close(): void }> {
  strips?: Map<number, LaneStrip>;
  private stripReaders?: WeakMap<IdentitySnapshot, StripReaders>;
  scrubScenes?: { frame: StripFrame; widgets: Map<number, ScrubScene> };

  strip(widget: number, build: number, update: StripUpdate) {
    let current = this.strips?.get(widget);
    if (update.kind === 'drop' && current?.build !== build) return EMPTY_STRIP_RECEIPT;
    if (current && current.build > build) throw new Error('Superseded dashboard strip');
    if (update.kind === 'drop' && !update.fine) { this.strips?.delete(widget); return EMPTY_STRIP_RECEIPT; }
    if (!current || current.build !== build) (this.strips ??= new Map()).set(widget, current = { build, held: new WidgetStrips() });
    if (update.kind === 'identities') {
      const identities = current.identities ??= {};
      const stage = update.fine ? 'fine' : 'coarse';
      if (update.snapshot === undefined) {
        if (identities[stage]) {
          identities.active = identities[stage];
          identities.activeFine = update.fine;
          delete identities[stage];
        } else if (!identities.active || identities.activeFine !== update.fine) throw new Error('Dashboard strip identities were not staged');
      } else {
        const snapshot = typeof update.snapshot === 'object' ? update.snapshot : (this.store.resolve({
          revisions: { identitySnapshot: update.snapshot },
        }) as { identitySnapshot: IdentitySnapshot }).identitySnapshot;
        const cached = this.stripReaders ??= new WeakMap();
        let readers = cached.get(snapshot);
        if (!readers) {
          const index = IdentityIndex.fromSnapshot(snapshot, identities[stage]?.index ?? identities.active?.index);
          cached.set(snapshot, readers = { index, identity: createSnapshotIdentityProvider(index) });
        }
        identities[stage] = readers;
      }
    } else if (update.kind === 'discover') {
      const target = this.widgets.get(widget), readers = current.identities?.[update.fine ? 'fine' : 'coarse'];
      if (!target || !readers) throw new Error('Dashboard strip identity scan has no mounted widget or snapshot');
      const plan = current.held.scene(update.hour, update.fine);
      const asks = new Map<string, IdentityAsk>();
      let unresolved = false;
      const mark = update.measure ? `bp-strip-discover:${this.host.lane}:${widget}:${update.hour}` : null;
      if (mark) performance.mark(`${mark}:start`);
      try {
        const scene = plan && this.scrubPicture(target, current.held, update.hour, plan, undefined, { ...readers, identity: {
          resolve: (kind, key, hour) => {
            const parsed = parseEntityKey(kind, key);
            asks.set(`${identityKeyString(parsed)}@${hour}`, { key: parsed, hour });
            const result = readers.identity.resolve(kind, key, hour);
            unresolved ||= [result.name, result.holder, result.tag, result.colour, result.flagChoice]
              .some(field => field?.status === 'pending' || field?.status === 'failed');
            return result;
          },
        } });
        if (scene?.mode === 'failed') throw new Error(scene.reason);
        return { ...EMPTY_STRIP_RECEIPT, identities: [...asks.values()], unresolved };
      } finally {
        if (mark) { performance.mark(`${mark}:end`); performance.clearMarks(`${mark}:start`); performance.clearMarks(`${mark}:end`); }
      }
    } else if (update.kind === 'drop' && current.identities) {
      if (current.identities.activeFine) delete current.identities.active;
      delete current.identities.fine;
    }
    return current.held.update(update);
  }

  private scrubPicture(widget: Widget, held: WidgetStrips, hour: number, plan: StripScenePlan, work?: StripWork, readers?: StripReaders): ScrubScene | undefined {
    if ('outside' in plan) return { mode: 'outside', outside: plan.outside };
    const scene: Scene = { widget: widget.id, subs: [], pictures: [], model: widget.model, owner: widget.binding.owner };
    return this.host.views(widget.view).scrub?.(scene, (validate, window) => this.scrubStatics(held, hour, plan, work, validate, window), widget.state, readers);
  }

  private scrubStatics(held: WidgetStrips, hour: number, plan: Extract<StripScenePlan, { recorded: number }>,
    work?: StripWork, validate?: TailValidator, window?: RowWindow): ScrubInput {
    const parts: (ExactPart | null)[] = [];
    for (const part of plan.parts) {
      if (!part) { parts.push(null); continue; }
      const raws: unknown[] = [];
      for (const slot of part.slots) {
        const state = held.slots.get(slot)?.at(hour, work);
        if (state === undefined) return { mode: 'waiting' };
        if (state === null) break;
        if ('refused' in state) return { mode: 'refused', reason: state.refused };
        raws.push(structuredClone(state.answer));
      }
      parts.push(raws.length === part.slots.length ? exactPart(part, plan.recorded, raws, validate) : null);
    }
    const statics = exactStatics(plan.recorded, parts, plan.allowEmpty, window);
    return statics ? { mode: 'picture', statics, options: plan.options } : { mode: 'missing' };
  }

  scrub(page: number, frame: StripFrame | null) {
    if (!frame) {
      if (this.scrubScenes?.frame.page === page) this.scrubScenes = undefined;
      return;
    }
    const widgets = new Map<number, ScrubScene>();
    this.scrubScenes = { frame, widgets };
    const samples: ScrubSample[] = [];
    for (const [id, { held, identities }] of this.strips ?? []) {
      if (held.page !== page) continue;
      const widget = this.widgets.get(id);
      const coverage = frame.gap && held.track ? stripCoverageAt(held.track, frame.hour) : null;
      const gap = coverage ? (coverage.hour === null ? coverage.outside : null) : frame.gap;
      const plan = frame.gap ? (gap ? { outside: gap } : undefined) : held.scene(frame.hour, frame.density !== 1);
      if (!widget || !plan) continue;
      const work = frame.measure ? { leaves: 0, bytes: 0 } : undefined;
      const mark = work ? `bp-strip:${page}:${frame.frame}:${this.host.lane}:${id}` : null;
      if (mark) performance.mark(`${mark}:start`);
      try {
        const scrub = this.scrubPicture(widget, held, frame.hour, plan, work, identities?.active);
        if (scrub) widgets.set(id, scrub);
        if (work) samples.push({ widget: id, mode: scrub?.mode ?? 'waiting', ...work });
      } catch (error) {
        const reason = String(error);
        widgets.set(id, { mode: 'failed', reason });
        if (work) samples.push({ widget: id, mode: 'failed', error: reason, ...work });
      } finally {
        if (mark) {
          performance.mark(`${mark}:end`, { detail: work });
          performance.clearMarks(`${mark}:start`);
          performance.clearMarks(`${mark}:end`);
        }
      }
    }
    if (frame.measure) this.host.scrubbed?.(frame, samples);
  }
  readonly facts = new Facts<Value>();
  readonly stats = { frames: 0, entries: 0, motion: 0, crops: 0, refused: {} as Record<string, number>, visited: 0, pruned: 0, restoreFailures: 0 };
  private readonly subs = new Map<number, Sub>();
  private readonly widgets = new Map<number, Widget>();
  private readonly workerCanvases = new Map<number, WorkerCanvasInfo>();
  private readonly disabledWorkerCanvases = new Set<number>();
  private readonly dirtyWorkerCanvases = new Set<number>();
  private sharedSurface: SharedLaneCanvas | null = null;
  private readonly subWidgets = new Map<number, Set<number>>();
  private readonly routeWidgets = new Map<number, Set<number>>();
  private readonly dirtyWidgets = new Set<number>();
  private readonly motionWidgets = new Set<number>();
  private readonly ephemeralWidgets = new Set<number>();
  private readonly inflightWidgets = new Set<number>();
  private readonly windowedWidgets = new Set<number>();
  private windowSafeLo = -Infinity;
  private windowSafeHi = Infinity;
  private windowSafeDirty = false;
  private lastTarget = NaN;
  private lastNow = NaN;
  private lastSpeed: number | null = null;
  private readonly parkedWidgets = new Set<number>();
  private readonly inspectedWidgets = new Set<number>();
  private readonly nonOpenFollowed = new Set<number>();
  private readonly nonOpenPanel = new Set<number>();
  private readonly prunableNow = new Set<number>();
  private readonly prunableAt = new Map<number, number>();
  private minPruneHour = Infinity;
  private readonly limitedSubs = new Set<number>();
  private readonly failedSubs = new Set<number>();
  private readonly waitingSubs = new Set<number>();
  private readonly failedWidgets = new Set<number>();
  private readonly routes = new Map<number, number>();
  private readonly natives = new Map<number, number>();
  private readonly offsets = new Map<number, number>();
  private readonly resets = new Map<number, number>();
  private readonly unbound = new Map<number, Map<number, number>>();
  private readonly stamped = new Map<number, number>();
  private readonly applied = new Map<number, number>();
  private readonly told = new Map<number, number>();
  private readonly routeConfirmedFollowed = new Map<number, number>();
  private readonly toldConfirmedFollowed = new Map<number, number>();
  private readonly routeConfirmedPanel = new Map<number, number>();
  private readonly toldConfirmedPanel = new Map<number, number>();
  private readonly pendingClosed = new Map<number, readonly [lo: number, hi: number]>();
  private readonly pendingWithdrawn = new Map<number, number>();
  private held: Held[] = [];
  private early: { native: number; seek: number; bytes: Uint8Array; stamp?: number }[] = [];
  private earlyPanels: { native: number; seek: number; bytes: Uint8Array; completedOnly?: number }[] = [];
  private earlyBytes = 0;
  private readonly newestFutureSeek = new Map<number, number>();
  private readonly newestUnboundSeek = new Map<number, number>();
  private readonly earlyOverflow = new Map<number, number>();
  private readonly overflowedSubs = new Map<number, { seek: number; subs: Set<number> }>();
  private heldBytes = 0;
  private readonly heldIds = new Map<number, number>();
  private readonly lost = new Map<number, number>();
  private readonly gone = new Set<number>();
  private readonly failed = new Set<number>();
  private readonly presented = new Map<number, { widget: number; sent: Sent }>();
  private readonly store = new PayloadStore();
  private readonly earlySupply = new Map<number, Resolved>();
  private readonly acks: { installed: readonly Ack[]; refused: readonly Refusal[] }[] = [];
  private ackMs = 0;
  private request: FrameRequest | null = null;
  private pendingExact: FrameRequest | null = null;
  private lastExact: FrameRequest | null = null;
  private deferredExact = false;
  private serving = false;
  private working = false;
  private inHandover = false;
  private handoverToken = 0;
  private pacing = false;
  private requests = 0;
  private pageRequests = 0;
  private nextEntry = 1;
  private nextContent = 1;
  private nextOrder = 1;
  private freed = 0;
  private readonly host: LaneHost<B>;

  constructor(host: LaneHost<B>) { this.host = host; }

  retainedBytes(): number {
    let bytes = this.freed + this.facts.retainedBytes();
    for (const item of this.held) bytes += item.size;
    for (const item of this.early) bytes += recordsCharge(item.bytes);
    for (const item of this.earlyPanels) bytes += panelRecordsCharge(item.bytes);
    return bytes;
  }

  private time() { return this.host.clock?.() ?? now(); }
  private content = () => `${this.host.lane}:${this.nextContent++}`;

  private indexWidget(widget: Widget) {
    for (const sub of widget.subs) {
      let set = this.subWidgets.get(sub);
      if (!set) this.subWidgets.set(sub, set = new Set());
      set.add(widget.id);
    }
    const route = this.routeOf(widget);
    if (route !== null) {
      let set = this.routeWidgets.get(route);
      if (!set) this.routeWidgets.set(route, set = new Set());
      set.add(widget.id);
    }
    this.syncOpenTracking(widget);
  }

  private unindexWidget(widget: Widget) {
    for (const sub of widget.subs) {
      const set = this.subWidgets.get(sub);
      if (set) { set.delete(widget.id); if (!set.size) this.subWidgets.delete(sub); }
    }
    for (const set of this.routeWidgets.values()) set.delete(widget.id);
    this.dirtyWidgets.delete(widget.id);
    this.motionWidgets.delete(widget.id);
    this.ephemeralWidgets.delete(widget.id);
    this.inflightWidgets.delete(widget.id);
    if (this.windowedWidgets.delete(widget.id)) {
      if (!this.windowedWidgets.size) { this.windowSafeLo = -Infinity; this.windowSafeHi = Infinity; this.windowSafeDirty = false; }
      else this.windowSafeDirty = true;
    }
    this.parkedWidgets.delete(widget.id);
    this.inspectedWidgets.delete(widget.id);
    this.nonOpenFollowed.delete(widget.id);
    this.nonOpenPanel.delete(widget.id);
    this.pendingClosed.delete(widget.id);
    this.pendingWithdrawn.delete(widget.id);
  }

  private clampSafeRange(lo: number, hi: number, a: number, b: number, offset: number, cur: number): [number, number] {
    const b0 = a - offset, b1 = b - offset;
    if (cur < b0) hi = Math.min(hi, b0 - 1);
    else if (cur > b1) lo = Math.max(lo, b1 + 1);
    else { lo = Math.max(lo, b0); hi = Math.min(hi, b1); }
    return [lo, hi];
  }

  private syncWindowTracking(widget: Widget) {
    const active = widget.inflight ?? widget.installed;
    const w = widget.validWindow;
    const finiteActive = !!active && (Number.isFinite(active.lo) || Number.isFinite(active.hi));
    const finiteValid = !!w && (Number.isFinite(w[0]) || Number.isFinite(w[1]));
    if (widget.binding.pin === null && (finiteActive || finiteValid)) {
      this.windowedWidgets.add(widget.id);
      const cur = this.isDirectTarget(widget.id) ? this.lastNow : this.lastTarget;
      if (Number.isFinite(cur)) {
        let lo = this.windowSafeLo, hi = this.windowSafeHi;
        if (w) [lo, hi] = this.clampSafeRange(lo, hi, w[0], w[1], widget.binding.offset, cur);
        if (active) [lo, hi] = this.clampSafeRange(lo, hi, active.lo, active.hi, widget.binding.offset, cur);
        this.windowSafeLo = lo;
        this.windowSafeHi = hi;
      } else {
        this.windowSafeDirty = true;
      }
    } else {
      if (this.windowedWidgets.delete(widget.id)) {
        if (!this.windowedWidgets.size) { this.windowSafeLo = -Infinity; this.windowSafeHi = Infinity; this.windowSafeDirty = false; }
        else this.windowSafeDirty = true;
      }
    }
    if (widget.inflight) this.inflightWidgets.add(widget.id);
    else this.inflightWidgets.delete(widget.id);
    if (active?.ephemeral) this.ephemeralWidgets.add(widget.id);
    else this.ephemeralWidgets.delete(widget.id);
    if (widget.parked) this.parkedWidgets.add(widget.id);
    else this.parkedWidgets.delete(widget.id);
    this.syncOpenTracking(widget);
  }

  private schedulePrune(subId: number) {
    this.prunableNow.add(subId);
    this.prunableAt.delete(subId);
  }

  private syncOpenTracking(widget: Widget) {
    const active = widget.inflight ?? widget.installed;
    const usesFollowed = widget.subs.some(id => {
      const sub = this.subs.get(id);
      return sub?.channel === 'followed' || (sub?.channel === 'panel' && (this.facts.usesOf(id)?.size ?? 0) > 0);
    });
    if (usesFollowed && active?.open !== 'followed') this.nonOpenFollowed.add(widget.id);
    else this.nonOpenFollowed.delete(widget.id);
    const usesOpenSteppedPanel = widget.subs.some(id => this.facts.isOpenSteppedPanel(id));
    if (usesOpenSteppedPanel && active?.open !== 'panel') this.nonOpenPanel.add(widget.id);
    else this.nonOpenPanel.delete(widget.id);
  }

  private failSub(id: number, reason: string) {
    if (id >= CELL) {
      this.failCell(id, reason);
      return;
    }
    if (this.failedSubs.has(id)) return;
    this.failedSubs.add(id);
    this.limitedSubs.delete(id);
    this.lost.delete(id);
    this.forget(id);
    this.facts.fail(id);
    for (const cell of this.facts.drain()) {
      this.gone.add(cell);
      this.limitedSubs.delete(cell);
      this.forget(cell);
    }
    const widgets = this.subWidgets.get(id);
    if (widgets) for (const wid of widgets) {
      const widget = this.widgets.get(wid);
      if (widget) {
        widget.inflight = null;
        widget.parked = null;
        this.dirtyWidgets.delete(wid);
        this.motionWidgets.delete(wid);
        this.syncWindowTracking(widget);
      }
    }
    this.host.failSub?.(id, reason);
  }

  private failCell(cell: number, reason: string) {
    const users = this.facts.usersOf(cell);
    if (!users || !users.size) return;
    for (const owner of [...users]) this.failSub(owner, reason);
  }

  private failWidget(widget: Widget, reason: string, stamp: number | undefined) {
    if (this.failedWidgets.has(widget.id)) return;
    this.failedWidgets.add(widget.id);
    widget.halted = widget.binding.generation;
    widget.motion = false;
    widget.inflight = null;
    widget.parked = null;
    this.motionWidgets.delete(widget.id);
    this.dirtyWidgets.delete(widget.id);
    this.syncWindowTracking(widget);
    this.host.failWidget?.(widget.id, reason, stamp);
  }

  private markSub(id: number, closeOpen = false) {
    if (id < CELL) {
      this.schedulePrune(id);
      const set = this.subWidgets.get(id);
      if (set) for (const wid of set) {
        this.dirtyWidgets.add(wid);
        if (closeOpen) {
          this.noteClosed(wid, -Infinity, Infinity);
          const widget = this.widgets.get(wid);
          if (widget) this.closeOpen(widget);
        }
      }
    } else {
      const users = this.facts.usersOf(id);
      if (users) for (const ownerSub of users) this.markSub(ownerSub, closeOpen);
    }
  }

  private markRoute(route: number, channel?: 'followed' | 'panel') {
    const set = this.routeWidgets.get(route);
    if (!set) return;
    for (const wid of set) {
      const widget = this.widgets.get(wid);
      if (!widget) continue;
      if (channel === 'panel' && !widget.subs.some(id => this.subs.get(id)?.channel === 'panel')) continue;
      if (channel === 'followed' && !widget.subs.some(id => {
        const sub = this.subs.get(id);
        return sub?.channel === 'followed' || (sub?.channel === 'panel' && (this.facts.usesOf(id)?.size ?? 0) > 0);
      })) continue;
      this.dirtyWidgets.add(wid);
    }
  }

  follow(sub: number, channel: Channel, product?: string) {
    this.subs.set(sub, { id: sub, channel, product, route: null });
    if (channel === 'pinned') this.facts.follow(sub, -1, 'pinned');
  }

  private retireRoute(route: number) {
    const native = this.natives.get(route);
    if (native !== undefined) this.dropEarlyBefore(native, Infinity);
    this.natives.delete(route);
    if (native !== undefined && this.routes.get(native) === route) this.routes.delete(native);
    this.offsets.delete(route);
    this.told.delete(route);
    this.routeConfirmedFollowed.delete(route);
    this.toldConfirmedFollowed.delete(route);
    this.routeConfirmedPanel.delete(route);
    this.toldConfirmedPanel.delete(route);
    this.routeWidgets.delete(route);
    this.failed.delete(route);
    this.facts.retireRoute(route);
  }

  private routeHasSubs(route: number) {
    for (const sub of this.subs.values()) if (sub.route === route) return true;
    return false;
  }

  bindRoute(sub: number, route: number, native: number, seek = 0, offset = 0) {
    const held = this.subs.get(sub);
    if (!held || held.route !== null) return;
    const prevRoute = this.routes.get(native);
    held.route = route;
    this.routes.set(native, route);
    this.natives.set(route, native);
    this.offsets.set(route, offset);
    if (prevRoute !== undefined && prevRoute !== route && !this.routeHasSubs(prevRoute)) this.retireRoute(prevRoute);
    this.facts.follow(sub, route, held.channel);
    this.facts.stamp(route, seek);
    const currentSeek = this.facts.generation(route).seek;
    const newestUnbound = this.newestUnboundSeek.get(native);
    this.newestUnboundSeek.delete(native);
    if (newestUnbound !== undefined && newestUnbound > currentSeek) {
      this.newestFutureSeek.set(native, Math.max(this.newestFutureSeek.get(native) ?? 0, newestUnbound));
    }
    const ov = this.overflowedSubs.get(native);
    if (ov) {
      if (ov.seek < currentSeek) this.overflowedSubs.delete(native);
      else if (ov.seek === currentSeek && ov.subs.has(sub)) {
        ov.subs.delete(sub);
        if (!ov.subs.size) this.overflowedSubs.delete(native);
        this.failSub(sub, EARLY_OVERFLOW);
      }
    }
    const set = this.subWidgets.get(sub);
    if (set) {
      let rset = this.routeWidgets.get(route);
      if (!rset) this.routeWidgets.set(route, rset = new Set());
      for (const wid of set) {
        rset.add(wid);
        this.dirtyWidgets.add(wid);
        const widget = this.widgets.get(wid);
        if (widget) this.syncOpenTracking(widget);
      }
    }
    this.settleResets();
    this.replay();
    this.replayEarly();
    this.replay();
    this.after();
  }

  unfollow(sub: number) {
    const route = this.subs.get(sub)?.route ?? null;
    this.subs.delete(sub);
    this.lost.delete(sub);
    this.limitedSubs.delete(sub);
    this.failedSubs.delete(sub);
    this.waitingSubs.delete(sub);
    this.prunableNow.delete(sub);
    this.prunableAt.delete(sub);
    for (const [native, ov] of [...this.overflowedSubs]) {
      if (ov.subs.delete(sub) && !ov.subs.size) this.overflowedSubs.delete(native);
    }
    this.forget(sub);
    for (const cell of this.facts.unfollow(sub)) {
      this.gone.add(cell);
      this.limitedSubs.delete(cell);
      this.forget(cell);
    }
    if (route !== null && !this.routeHasSubs(route)) this.retireRoute(route);
    this.after();
  }

  mount(widget: number, subs: readonly number[], view: string, model?: unknown) {
    const was = this.widgets.get(widget);
    const pendingWithdrawn = was ? this.pendingWithdrawn.get(widget) : undefined;
    if (was) this.unindexWidget(was);
    const switched = !!was && was.view !== view;
    if (switched) this.host.views(was.view).unmount?.(widget, was.state);
    const early = was ? undefined : this.earlySupply.get(widget);
    this.earlySupply.delete(widget);
    const supplied: Supplied = was?.supplied ?? { stamp: undefined, statics: null, identity: null };
    const own = this.store.resolve(model);
    const made: Widget = { id: widget, order: was?.order ?? this.nextOrder++, subs: [...subs], view, model: own, own, supplied, supply: was?.supply ?? 0,
      binding: was?.binding ?? { generation: 1, surfaces: [], offset: 0, pin: null },
      installed: was?.installed ?? null, shown: was?.shown ?? null, inflight: null, sent: {}, forceFull: false, motion: false, inspect: was?.inspect ?? null, validWindow: null, start: null, state: switched ? new Map() : was?.state ?? new Map(), cost: was?.cost ?? 0.1,
      served: was?.served ?? 0, deferrals: 0, halted: null, parked: null, withdrawn: was?.withdrawn ?? null };
    made.model = this.compose(made);
    this.widgets.set(widget, made);
    if (early) this.supplied(made, early);
    this.indexWidget(made);
    if (pendingWithdrawn !== undefined) this.pendingWithdrawn.set(widget, pendingWithdrawn);
    if (made.inspect !== null) this.inspectedWidgets.add(widget);
    this.dirtyWidgets.add(widget);
    this.syncWindowTracking(made);
    this.wake();
  }

  unmount(widget: number) {
    this.strips?.delete(widget);
    this.scrubScenes?.widgets.delete(widget);
    const held = this.widgets.get(widget);
    if (held) {
      this.host.views(held.view).unmount?.(widget, held.state);
      this.unindexWidget(held);
      this.widgets.delete(widget);
    }
    this.earlySupply.delete(widget);
    this.failedWidgets.delete(widget);
    this.workerCanvases.delete(widget);
    this.disabledWorkerCanvases.delete(widget);
    this.dirtyWorkerCanvases.delete(widget);
    const shared = this.sharedSurface;
    if (shared) {
      const prev = shared.slots.get(widget);
      if (prev && prev.visible && shared.width > 0 && shared.height > 0) {
        const dpr = shared.dpr > 0 ? shared.dpr : 1;
        if (shared.surfaceColor) {
          shared.ctx.save();
          shared.rawSetTransform(1, 0, 0, 1, 0, 0);
          shared.ctx.fillStyle = shared.surfaceColor;
          shared.ctx.fillRect(
            Math.floor(prev.x * dpr),
            Math.floor(prev.y * dpr),
            Math.ceil(prev.w * dpr),
            Math.ceil(prev.h * dpr),
          );
          shared.ctx.restore();
        }
        for (const [otherId, other] of shared.slots) {
          if (otherId === widget || !other.visible) continue;
          if (other.x < prev.x + prev.w && other.x + other.w > prev.x && other.y < prev.y + prev.h && other.y + other.h > prev.y) {
            this.markDirectDirty(otherId);
          }
        }
        this.windowSafeDirty = true;
        this.wake();
      }
      shared.slots.delete(widget);
      shared.dirtySlots.delete(widget);
    }
    for (const [id, owner] of this.presented) if (owner.widget === widget) this.presented.delete(id);
  }

  setCanvas(widget: number, canvas: OffscreenCanvas, opaque: boolean) {
    this.workerCanvases.set(widget, { canvas, opaque });
    this.disabledWorkerCanvases.delete(widget);
    this.dirtyWorkerCanvases.add(widget);
    this.sharedSurface?.slots.delete(widget);
    this.sharedSurface?.dirtySlots.delete(widget);
    this.windowSafeDirty = true;
    const held = this.widgets.get(widget);
    if (held) {
      held.forceFull = true;
      this.dirtyWidgets.add(widget);
      this.wake();
    }
  }

  clearCanvas(widget: number) {
    if (!this.workerCanvases.has(widget) || this.disabledWorkerCanvases.has(widget)) return;
    this.disabledWorkerCanvases.add(widget);
    this.dirtyWorkerCanvases.delete(widget);
    this.windowSafeDirty = true;
    const held = this.widgets.get(widget);
    if (held) {
      held.forceFull = true;
      this.dirtyWidgets.add(widget);
      this.wake();
    }
  }

  useCanvas(widget: number) {
    if (!this.workerCanvases.has(widget) || !this.disabledWorkerCanvases.delete(widget)) return;
    this.dirtyWorkerCanvases.add(widget);
    this.sharedSurface?.slots.delete(widget);
    this.sharedSurface?.dirtySlots.delete(widget);
    this.windowSafeDirty = true;
    const held = this.widgets.get(widget);
    if (held) {
      held.forceFull = true;
      this.dirtyWidgets.add(widget);
      this.wake();
    }
  }

  setLaneSurface(canvas: OffscreenCanvas, opaque: boolean) {
    const ctx = canvas.getContext('2d', opaque ? { alpha: false } : undefined);
    if (!ctx) {
      this.host.failLaneSurface?.();
      return;
    }
    const activeSlot = {
      ox: 0,
      oy: 0,
      w: 0,
      h: 0,
      dpr: 1,
      surfaceColor: '',
      widgetKey: {},
    };
    const rawSetTransform = ctx.setTransform.bind(ctx);
    const rawResetTransform = typeof ctx.resetTransform === 'function' ? ctx.resetTransform.bind(ctx) : null;
    ctx.setTransform = ((a?: number | DOMMatrix2DInit, b?: number, c?: number, d?: number, e?: number, f?: number) => {
      if (typeof a === 'number') {
        rawSetTransform(a, b ?? 0, c ?? 0, d ?? 1, (e ?? 0) + activeSlot.ox, (f ?? 0) + activeSlot.oy);
      } else if (a && typeof a === 'object') {
        rawSetTransform({
          ...a,
          e: (a.e ?? a.m41 ?? 0) + activeSlot.ox,
          f: (a.f ?? a.m42 ?? 0) + activeSlot.oy,
        });
      } else {
        rawSetTransform(1, 0, 0, 1, activeSlot.ox, activeSlot.oy);
      }
    }) as OffscreenCanvasRenderingContext2D['setTransform'];
    if (rawResetTransform) {
      ctx.resetTransform = () => {
        rawSetTransform(1, 0, 0, 1, activeSlot.ox, activeSlot.oy);
      };
    }
    snapCanvasText(ctx);
    if (opaque) {
      const rawClearRect = ctx.clearRect.bind(ctx);
      ctx.clearRect = (x: number, y: number, w: number, h: number) => {
        if (!activeSlot.surfaceColor) { rawClearRect(x, y, w, h); return; }
        const prev = ctx.fillStyle;
        ctx.fillStyle = activeSlot.surfaceColor;
        ctx.fillRect(x, y, w, h);
        ctx.fillStyle = prev;
      };
    }
    this.sharedSurface = {
      canvas,
      ctx,
      rawSetTransform,
      opaque,
      width: 0,
      height: 0,
      dpr: 1,
      scrollTop: 0,
      scrollLeft: 0,
      originX: 0,
      originY: 0,
      surfaceColor: '',
      activeSlot,
      slots: new Map(),
      dirtySlots: new Set(),
    };
  }

  disposeLaneSurface() {
    const shared = this.sharedSurface;
    if (!shared) return;
    shared.canvas.width = 1;
    shared.canvas.height = 1;
    for (const wid of shared.slots.keys()) {
      const held = this.widgets.get(wid);
      if (held) {
        held.forceFull = true;
        this.dirtyWidgets.add(wid);
      }
    }
    shared.slots.clear();
    shared.dirtySlots.clear();
    this.sharedSurface = null;
    this.windowSafeDirty = true;
    this.wake();
  }

  setLaneSlots(payload: LaneSlotsPayload) {
    const shared = this.sharedSurface;
    if (!shared) return;
    const dpr = payload.dpr > 0 ? payload.dpr : 1;
    const bw = Math.max(1, Math.round(payload.width * dpr));
    const bh = Math.max(1, Math.round(payload.height * dpr));
    const ox = payload.originX ?? payload.scrollLeft;
    const oy = payload.originY ?? payload.scrollTop;
    const resized = shared.canvas.width !== bw || shared.canvas.height !== bh || shared.surfaceColor !== payload.surfaceColor || shared.dpr !== dpr;
    const dx = Math.round((ox - shared.originX) * dpr);
    const dy = Math.round((oy - shared.originY) * dpr);
    const originMoved = !resized && (dx !== 0 || dy !== 0);
    const prevSlots = new Map(shared.slots);
    const prevW = shared.width;
    const prevH = shared.height;
    shared.width = payload.width;
    shared.height = payload.height;
    shared.dpr = dpr;
    shared.scrollTop = payload.scrollTop;
    shared.scrollLeft = payload.scrollLeft;
    shared.originX = ox;
    shared.originY = oy;
    shared.surfaceColor = payload.surfaceColor;

    if (resized) {
      shared.canvas.width = bw;
      shared.canvas.height = bh;
      if (payload.surfaceColor) {
        shared.ctx.save();
        shared.rawSetTransform(1, 0, 0, 1, 0, 0);
        shared.ctx.fillStyle = payload.surfaceColor;
        shared.ctx.fillRect(0, 0, bw, bh);
        shared.ctx.restore();
      }
    } else if (originMoved && payload.width > 0 && payload.height > 0) {
      if (Math.abs(dx) < bw && Math.abs(dy) < bh) {
        shared.ctx.save();
        shared.rawSetTransform(1, 0, 0, 1, 0, 0);
        shared.ctx.drawImage(shared.canvas, -dx, -dy);
        if (payload.surfaceColor) {
          shared.ctx.fillStyle = payload.surfaceColor;
          if (dy > 0) shared.ctx.fillRect(0, bh - dy, bw, dy);
          else if (dy < 0) shared.ctx.fillRect(0, 0, bw, -dy);
          if (dx > 0) shared.ctx.fillRect(bw - dx, 0, dx, bh);
          else if (dx < 0) shared.ctx.fillRect(0, 0, -dx, bh);
        }
        shared.ctx.restore();
      } else if (payload.surfaceColor) {
        shared.ctx.save();
        shared.rawSetTransform(1, 0, 0, 1, 0, 0);
        shared.ctx.fillStyle = payload.surfaceColor;
        shared.ctx.fillRect(0, 0, bw, bh);
        shared.ctx.restore();
      }
    }

    const nextSlots = new Map<number, SharedSurfaceSlot>();
    for (const slot of payload.slots) nextSlots.set(slot.widget, slot);

    const clearedRects: { x: number; y: number; w: number; h: number }[] = [];
    if (!resized && payload.width > 0 && payload.height > 0) {
      for (const [wid, prev] of prevSlots) {
        if (!prev.visible) continue;
        const rx = originMoved ? prev.x - dx / dpr : prev.x;
        const ry = originMoved ? prev.y - dy / dpr : prev.y;
        const next = nextSlots.get(wid);
        const stillSame = next && next.visible
          && Math.abs(next.x - rx) < 0.5 && Math.abs(next.y - ry) < 0.5 && next.w === prev.w && next.h === prev.h;
        if (stillSame) continue;
        clearedRects.push({ x: rx, y: ry, w: prev.w, h: prev.h });
      }
      if (clearedRects.length && payload.surfaceColor) {
        shared.ctx.save();
        shared.rawSetTransform(1, 0, 0, 1, 0, 0);
        shared.ctx.fillStyle = payload.surfaceColor;
        for (const r of clearedRects) {
          shared.ctx.fillRect(
            Math.floor(r.x * dpr),
            Math.floor(r.y * dpr),
            Math.ceil(r.w * dpr),
            Math.ceil(r.h * dpr),
          );
        }
        shared.ctx.restore();
      }
    }

    let directChanged = false;
    for (const [wid, prev] of prevSlots) {
      const next = nextSlots.get(wid);
      if (prev.visible && (!next || !next.visible)) {
        directChanged = true;
        shared.dirtySlots.delete(wid);
        const held = this.widgets.get(wid);
        if (held) {
          held.forceFull = true;
          this.dirtyWidgets.add(wid);
        }
      }
    }
    for (const slot of payload.slots) {
      const prev = prevSlots.get(slot.widget);
      if (slot.visible !== Boolean(prev?.visible)) directChanged = true;
      const moved = !prev || prev.x !== slot.x || prev.y !== slot.y || prev.w !== slot.w || prev.h !== slot.h || prev.visible !== slot.visible;
      const overlappedClear = clearedRects.some(r =>
        slot.x < r.x + r.w && slot.x + slot.w > r.x && slot.y < r.y + r.h && slot.y + slot.h > r.y
      );
      const preservedByBlit = !resized && originMoved && prev !== undefined && prev.visible
        && Math.abs(slot.x - (prev.x - dx / dpr)) < 0.5
        && Math.abs(slot.y - (prev.y - dy / dpr)) < 0.5
        && slot.w === prev.w && slot.h === prev.h
        && prev.x >= 0 && prev.y >= 0 && prev.x + prev.w <= prevW && prev.y + prev.h <= prevH
        && slot.x >= 0 && slot.y >= 0 && slot.x + slot.w <= payload.width && slot.y + slot.h <= payload.height
        && !overlappedClear;
      if (slot.visible && !preservedByBlit && (resized || originMoved || moved || overlappedClear)) {
        shared.dirtySlots.add(slot.widget);
        const held = this.widgets.get(slot.widget);
        if (held) held.forceFull = true;
        this.dirtyWidgets.add(slot.widget);
      }
    }
    if (directChanged) this.windowSafeDirty = true;
    shared.slots.clear();
    for (const [wid, slot] of nextSlots) shared.slots.set(wid, slot);
    this.wake();
  }

  bind(widget: number, binding: Binding, model?: unknown, retry = false, remounted = false) {
    const held = this.widgets.get(widget);
    if (!held) return;
    const ownerChanged = binding.owner !== undefined && held.binding.owner !== undefined && binding.owner !== held.binding.owner;
    const prev = held.binding;
    held.binding = binding;
    if (ownerChanged) held.supplied = { stamp: undefined, statics: null, identity: held.supplied.identity };
    if (model !== undefined) held.own = this.store.resolve(model);
    if (model !== undefined || ownerChanged) held.model = this.compose(held);
    this.bounded(held);
    const keepSurfaces = !ownerChanged && !retry && !remounted && held.installed !== null
      && prev.pin === binding.pin && prev.offset === binding.offset
      && prev.surfaces.length === binding.surfaces.length
      && prev.surfaces.every((surface, at) => {
        const next = binding.surfaces[at];
        return !!next && surface.width === next.width && surface.height === next.height && surface.dpr === next.dpr;
      });
    if (!keepSurfaces && held.installed) held.installed = { ...held.installed, contents: {} };
    held.sent = keepSurfaces ? { ...held.installed!.contents } : {};
    held.forceFull = false;
    held.inflight = null;
    if (ownerChanged) {
      held.installed = null;
      held.shown = null;
      held.parked = null;
      held.inspect = null;
      held.withdrawn = null;
      this.pendingWithdrawn.delete(widget);
      this.parkedWidgets.delete(widget);
      this.inspectedWidgets.delete(widget);
    }
    const zeroSized = binding.surfaces.some(surface => surface.width <= 0 || surface.height <= 0);
    if (!ownerChanged && zeroSized && (held.installed !== null || held.shown !== null || this.hasPresented(widget) || held.withdrawn !== null)) {
      held.installed = null;
      held.shown = null;
      held.inflight = null;
      held.parked = null;
      held.sent = {};
      held.forceFull = true;
      held.motion = false;
      this.motionWidgets.delete(widget);
      held.withdrawn = binding.generation;
      this.pendingWithdrawn.set(widget, binding.generation);
    }
    if (retry) {
      this.failedWidgets.delete(widget);
      held.halted = null;
    } else if (held.halted !== null && held.halted !== binding.generation) {
      held.halted = null;
    }
    this.dirtyWidgets.add(widget);
    this.syncWindowTracking(held);
    this.wake();
  }

  payloads(message: PayloadsMessage) {
    this.store.apply(message);
  }

  supply(widget: number, change: Supply) {
    const resolved: Resolved = {
      ...change.stamp !== undefined ? { stamp: change.stamp } : {},
      ...change.statics !== undefined ? { statics: change.statics === null ? null : this.store.resolve(change.statics) as Readonly<Record<string, unknown>> } : {},
      ...change.identity !== undefined ? { identity: change.identity === null ? null : { id: change.identity, snapshot: (this.store.resolve({ revisions: { identitySnapshot: change.identity } }) as { identitySnapshot: unknown }).identitySnapshot } } : {},
      ...change.dated !== undefined ? { dated: change.dated } : {},
    };
    const held = this.widgets.get(widget);
    if (held) { this.supplied(held, resolved); return; }
    const early = this.earlySupply.get(widget);
    const next: Resolved = { ...early };
    if (resolved.statics !== undefined) { next.statics = resolved.statics; next.stamp = change.stamp; }
    if (resolved.identity !== undefined) next.identity = resolved.identity;
    if (resolved.dated !== undefined && (resolved.statics !== undefined || change.stamp === early?.stamp)) next.dated = resolved.dated;
    this.earlySupply.set(widget, next);
  }

  private closeOpen(widget: Widget) {
    const route = this.routeOf(widget);
    if (widget.installed) {
      if (widget.installed.open === 'followed' && route !== null) {
        widget.installed.c = Math.max(widget.installed.c, this.routeConfirmedFollowed.get(route) ?? -Infinity);
      } else if (widget.installed.open === 'panel' && route !== null) {
        widget.installed.c = Math.max(widget.installed.c, this.routeConfirmedPanel.get(route) ?? -Infinity);
      }
      widget.installed.open = undefined;
    }
    if (widget.inflight) widget.inflight.open = undefined;
    this.syncOpenTracking(widget);
  }

  private noteClosed(widget: number, lo: number, hi: number) {
    const was = this.pendingClosed.get(widget);
    this.pendingClosed.set(widget, was ? [Math.max(was[0], lo), Math.min(was[1], hi)] : [lo, hi]);
  }

  private bounded(widget: Widget) {
    const dated = widget.binding.dated;
    if (!dated || !Number.isFinite(dated.c)) return;
    if (!widget.installed?.open && !widget.inflight?.open) return;
    this.noteClosed(widget.id, dated.h, dated.c);
    this.closeOpen(widget);
    const shown = widget.installed;
    if (!shown) return;
    shown.c = Math.min(shown.c, dated.c);
    if (Math.min(shown.c, shown.hi) < Math.max(shown.h, dated.h)) {
      widget.installed = null;
      widget.shown = null;
      widget.sent = {};
    }
  }

  private supplied(held: Widget, change: Resolved) {
    if (change.statics === undefined && change.dated !== undefined && change.stamp !== held.supplied.stamp) return;
    if (change.statics !== undefined) held.supplied = { ...held.supplied, stamp: change.stamp, statics: change.statics };
    if (change.identity !== undefined) held.supplied = { ...held.supplied, identity: change.identity };
    if (change.dated !== undefined) held.binding = change.dated ? { ...withoutDated(held.binding), dated: change.dated } : withoutDated(held.binding);
    this.bounded(held);
    if (change.statics !== undefined || change.identity !== undefined) {
      held.model = this.compose(held);
      held.supply++;
    }
    this.dirtyWidgets.add(held.id);
    this.syncWindowTracking(held);
    this.wake();
  }

  private compose(widget: Widget): unknown {
    const own = widget.own, { statics, identity } = widget.supplied;
    if (!own || typeof own !== 'object' || (statics === null && identity === null)) return own;
    const revisions = {
      ...(own as { revisions?: Revisions }).revisions,
      ...(statics as { revisions?: Revisions } | null)?.revisions,
      ...identity !== null ? { identitySnapshot: identity.id } : {},
    };
    return {
      ...own,
      ...statics,
      ...identity !== null ? { identitySnapshot: identity.snapshot } : {},
      revisions,
    };
  }

  touch(widget: number) {
    const held = this.widgets.get(widget);
    if (!held) return;
    held.motion = true;
    this.motionWidgets.add(widget);
    this.dirtyWidgets.add(widget);
    this.wake();
  }

  pointer(widget: number, _surface: number, point: [number, number] | null, installed?: number) {
    const held = this.widgets.get(widget);
    if (!held) return;
    const owner = held.binding.owner;
    const accepted = installed === undefined ? undefined : this.presented.get(installed);
    const named = installed === undefined ? held.installed : accepted?.widget === widget ? accepted.sent : held.shown?.id === installed ? held.shown : null;
    const inst = named && (named.owner === undefined || owner === undefined || named.owner === owner) ? named : null;
    this.host.hit?.(widget, point && inst?.hit ? inst.hit(point) : null, installed ?? inst?.id, inst?.owner);
  }

  exact(sub: number, hour: number, data: unknown, refused: string | null) {
    if (this.failedSubs.has(sub)) return;
    this.facts.exact(sub, hour, { kind: 'panel', bytes: null, data, refused }, refused);
    this.markSub(sub);
    this.wake();
  }

  stream(route: number, seek: number, target: number) {
    if (!this.natives.has(route)) return;
    if (this.lastExact && this.lastExact.seek < seek) this.lastExact = null;
    this.failed.delete(route);
    this.routeConfirmedFollowed.delete(route);
    this.toldConfirmedFollowed.delete(route);
    this.routeConfirmedPanel.delete(route);
    this.toldConfirmedPanel.delete(route);
    for (const [subId, sub] of this.subs) if (sub.route === route) {
      this.limitedSubs.delete(subId);
      for (const cell of this.facts.usesOf(subId) ?? []) this.limitedSubs.delete(cell);
      if (!this.failedSubs.has(subId)) this.schedulePrune(subId);
      this.noteWaiting(subId, false);
    }
    const native = this.natives.get(route);
    if (native !== undefined) for (const [id, from] of this.lost) if (from === native) this.lost.delete(id);
    this.facts.seek(route, seek, target + (this.offsets.get(route) ?? 0));
    for (const wid of this.routeWidgets.get(route) ?? []) {
      const widget = this.widgets.get(wid);
      if (!widget) continue;
      widget.inflight = null;
      if (widget.installed) widget.installed.open = undefined;
      this.dirtyWidgets.add(wid);
      this.syncWindowTracking(widget);
    }
    this.settleResets();
    this.replay();
    this.replayEarly();
    this.replay();
    this.after();
  }

  fail(route: number) {
    this.failed.add(route);
    for (const wid of this.routeWidgets.get(route) ?? []) {
      const widget = this.widgets.get(wid);
      if (!widget) continue;
      widget.inflight = null;
      this.dirtyWidgets.add(wid);
      this.syncWindowTracking(widget);
    }
  }

  receive(kind: 'follow' | 'panels', bytes: Uint8Array, resets?: number) {
    try {
      if (kind === 'follow') this.follows(bytes, resets);
      else this.panels(bytes);
      this.replay();
    } finally { this.after(); }
  }

  private after() {
    for (const id of this.facts.drain()) this.gone.add(id);
    const bytes = this.freed + this.releasedFacts();
    this.freed = 0;
    if (bytes) this.host.release(bytes);
    this.wake();
  }

  private releasedFacts() {
    if (!this.facts.hasRemoved()) return this.facts.release(() => false);
    const shown = new Set<unknown>();
    for (const widget of this.widgets.values()) {
      for (const sent of [widget.installed, widget.inflight]) for (const picture of sent?.pictures ?? []) for (const part of picture.parts) shown.add(part);
    }
    for (const owner of this.presented.values()) for (const picture of owner.sent.pictures) for (const part of picture.parts) shown.add(part);
    return this.facts.release(fact => shown.has(fact));
  }

  private routeOf(widget: Widget) {
    for (const sub of widget.subs) { const route = this.subs.get(sub)?.route; if (route !== null && route !== undefined) return route; }
    return null;
  }

  private known(id: number) {
    return id >= CELL ? this.facts.owns(id) : this.subs.get(id)?.route !== null && this.subs.has(id);
  }

  private hold(item: Held) {
    if (this.failedSubs.has(item.id) || this.lost.has(item.id)) { this.freed += item.size; return; }
    if (this.heldBytes + item.size + HELD_COST > BUFFER) {
      this.freed += item.size;
      this.forget(item.id);
      if (item.id < CELL) this.failSub(item.id, LOST);
      else this.lost.set(item.id, item.native);
      return;
    }
    this.heldBytes += item.size + HELD_COST;
    this.heldIds.set(item.id, item.native);
    this.held.push(item);
  }

  private release(items: readonly Held[]) {
    for (const item of items) { this.heldBytes -= item.size + HELD_COST; this.freed += item.size; }
  }

  private reindex() {
    this.heldIds.clear();
    for (const item of this.held) this.heldIds.set(item.id, item.native);
  }

  private forget(id: number) {
    if (!this.heldIds.has(id)) return;
    this.release(this.held.filter(item => item.id === id));
    this.held = this.held.filter(item => item.id !== id);
    this.reindex();
  }

  private unboundReset(native: number, seek: number) {
    let seeks = this.unbound.get(native);
    if (!seeks) this.unbound.set(native, seeks = new Map());
    seeks.set(seek, (seeks.get(seek) ?? 0) + 1);
    const dropped = this.held.filter(item => item.native === native && item.kind === 'follow' && item.seek <= seek);
    this.release(dropped);
    this.held = this.held.filter(item => !dropped.includes(item));
    this.reindex();
  }

  private settleResets() {
    for (const [native, stamp] of this.stamped) if (this.routes.has(native)) { this.resets.set(native, stamp); this.unbound.delete(native); }
    for (const [native, seeks] of this.unbound) {
      const route = this.routes.get(native);
      if (route === undefined) continue;
      const current = this.facts.generation(route).seek;
      for (const [seek, count] of seeks) {
        if (seek === current) this.resets.set(native, (this.resets.get(native) ?? 0) + count);
        if (seek <= current) seeks.delete(seek);
      }
      if (!seeks.size) this.unbound.delete(native);
    }
  }

  private drop(native: number) {
    for (const [id, from] of this.lost) if (from === native) this.lost.delete(id);
    const dropped = this.held.filter(item => item.native === native && item.kind === 'follow');
    this.release(dropped);
    this.held = this.held.filter(item => !dropped.includes(item));
    this.reindex();
  }

  private holdMessage(kind: Held['kind'], native: number, seek: number, hour: number, records: ReadonlyMap<number, { records: Uint8Array[]; size: number }>, completed?: number) {
    const ids = new Set(records.keys());
    for (const [id, from] of this.heldIds) if (from === native && (kind === 'follow') === this.heldFollows(id)) ids.add(id);
    for (const id of ids) {
      const own = records.get(id);
      if (!own && kind === 'panels' && completed === undefined) continue;
      this.hold({ kind, native, seek, hour, id, records: own?.records ?? [], size: own?.size ?? 0, ...completed !== undefined ? { completed } : {} });
    }
  }

  private heldFollows(id: number) { return id >= CELL || this.subs.get(id)?.channel === 'followed'; }

  private current(item: { native: number; seek: number }): -1 | 0 | 1 | null {
    const route = this.routes.get(item.native);
    if (route === undefined) return -1;
    const seek = this.facts.generation(route).seek;
    return item.seek < seek ? -1 : item.seek > seek ? 1 : 0;
  }

  private earlyLimit() { return this.host.earlyBuffer ?? BUFFER; }

  private dropHeldBefore(native: number, seek: number) {
    const dropped = this.held.filter(item => item.native === native && item.seek < seek);
    if (!dropped.length) return;
    this.release(dropped);
    this.held = this.held.filter(item => !dropped.includes(item));
    this.reindex();
  }

  private acceptUnboundSeek(native: number, seek: number, charge: number): boolean {
    const newest = this.newestUnboundSeek.get(native);
    if (newest !== undefined && seek < newest) {
      this.freed += charge;
      return false;
    }
    if (newest === undefined || seek > newest) {
      this.newestUnboundSeek.set(native, seek);
      this.dropHeldBefore(native, seek);
      const seeks = this.unbound.get(native);
      if (seeks) {
        for (const s of [...seeks.keys()]) if (s < seek) seeks.delete(s);
        if (!seeks.size) this.unbound.delete(native);
      }
    }
    return true;
  }

  private dropEarlyBefore(native: number, seek: number) {
    this.dropHeldBefore(native, seek);
    const keepFollow: typeof this.early = [];
    for (const item of this.early) {
      if (item.native === native && item.seek < seek) {
        this.earlyBytes -= item.bytes.byteLength + HELD_COST;
        this.freed += recordsCharge(item.bytes);
      } else keepFollow.push(item);
    }
    this.early = keepFollow;
    const keepPanels: typeof this.earlyPanels = [];
    for (const item of this.earlyPanels) {
      if (item.native === native && item.seek < seek) {
        this.earlyBytes -= item.bytes.byteLength + HELD_COST;
        this.freed += panelRecordsCharge(item.bytes);
      } else keepPanels.push(item);
    }
    this.earlyPanels = keepPanels;
  }

  private recordOverflowSubs(native: number, seek: number, ids: Iterable<number>) {
    let entry = this.overflowedSubs.get(native);
    if (!entry || entry.seek !== seek) this.overflowedSubs.set(native, entry = { seek, subs: new Set() });
    for (const id of ids) if (id < CELL) entry.subs.add(id);
  }

  private acceptFutureSeek(native: number, seek: number, charge: number, subs: readonly number[]): boolean {
    const newest = this.newestFutureSeek.get(native);
    if (newest !== undefined && seek < newest) {
      this.freed += charge;
      return false;
    }
    if (newest === undefined || seek > newest) {
      this.newestFutureSeek.set(native, seek);
      this.dropEarlyBefore(native, seek);
      const overflowed = this.earlyOverflow.get(native);
      if (overflowed !== undefined && overflowed < seek) this.earlyOverflow.delete(native);
      const ovSubs = this.overflowedSubs.get(native);
      if (ovSubs && ovSubs.seek < seek) this.overflowedSubs.delete(native);
    }
    if (this.earlyOverflow.get(native) === seek) {
      this.recordOverflowSubs(native, seek, subs);
      this.freed += charge;
      return false;
    }
    return true;
  }

  private overflowEarly(native: number, seek: number, extraSubs: readonly number[]) {
    this.earlyOverflow.set(native, seek);
    this.recordOverflowSubs(native, seek, extraSubs);
    const keepFollow: typeof this.early = [];
    for (const item of this.early) {
      if (item.native === native && item.seek === seek) {
        this.recordOverflowSubs(native, seek, followRecordSubs(item.bytes));
        this.earlyBytes -= item.bytes.byteLength + HELD_COST;
        this.freed += recordsCharge(item.bytes);
      } else keepFollow.push(item);
    }
    this.early = keepFollow;
    const keepPanels: typeof this.earlyPanels = [];
    for (const item of this.earlyPanels) {
      if (item.native === native && item.seek === seek) {
        this.recordOverflowSubs(native, seek, panelRecordSubs(item.bytes));
        this.earlyBytes -= item.bytes.byteLength + HELD_COST;
        this.freed += panelRecordsCharge(item.bytes);
      } else keepPanels.push(item);
    }
    this.earlyPanels = keepPanels;
    const droppedHeld = this.held.filter(item => item.native === native && item.seek === seek);
    if (droppedHeld.length) {
      this.recordOverflowSubs(native, seek, droppedHeld.map(item => item.id));
      this.release(droppedHeld);
      this.held = this.held.filter(item => !droppedHeld.includes(item));
      this.reindex();
    }
  }

  private queueEarlyFollow(native: number, seek: number, bytes: Uint8Array, stamp?: number) {
    const charge = recordsCharge(bytes);
    const subs = followRecordSubs(bytes);
    if (!this.acceptFutureSeek(native, seek, charge, subs)) return;
    const cost = bytes.byteLength + HELD_COST;
    if (this.earlyBytes + cost > this.earlyLimit()) {
      this.freed += charge;
      this.overflowEarly(native, seek, subs);
      return;
    }
    this.earlyBytes += cost;
    this.early.push({ native, seek, bytes, stamp });
  }

  private queueEarlyPanel(native: number, seek: number, bytes: Uint8Array) {
    const charge = panelRecordsCharge(bytes);
    const subs = panelRecordSubs(bytes);
    if (!this.acceptFutureSeek(native, seek, charge, subs)) return;
    const data = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const count = bytes.byteLength >= 24 ? data.getUint32(8, true) : 0;
    const isCompletedOnly = count === 1 && bytes.byteLength >= 40 && data.getUint8(24 + 8) === 4;
    if (isCompletedOnly) {
      const completed = data.getInt32(24 + 4, true);
      let prev: (typeof this.earlyPanels)[number] | undefined;
      for (let i = this.earlyPanels.length - 1; i >= 0; i--) {
        const item = this.earlyPanels[i]!;
        if (item.native === native && item.seek === seek) { prev = item; break; }
      }
      if (prev && prev.completedOnly !== undefined) {
        if (completed > prev.completedOnly) {
          this.earlyBytes += bytes.byteLength - prev.bytes.byteLength;
          prev.completedOnly = completed;
          prev.bytes = bytes;
        }
        return;
      }
      const cost = bytes.byteLength + HELD_COST;
      if (this.earlyBytes + cost > this.earlyLimit()) { this.overflowEarly(native, seek, subs); return; }
      this.earlyBytes += cost;
      this.earlyPanels.push({ native, seek, bytes, completedOnly: completed });
      return;
    }
    const cost = bytes.byteLength + HELD_COST;
    if (this.earlyBytes + cost > this.earlyLimit()) {
      this.freed += charge;
      this.overflowEarly(native, seek, subs);
      return;
    }
    this.earlyBytes += cost;
    this.earlyPanels.push({ native, seek, bytes });
  }

  private replayEarly() {
    for (const [native, newest] of [...this.newestFutureSeek]) {
      const route = this.routes.get(native);
      if (route !== undefined && this.facts.generation(route).seek >= newest) this.newestFutureSeek.delete(native);
    }
    for (const [native, seek] of [...this.earlyOverflow]) {
      const route = this.routes.get(native);
      if (route === undefined) continue;
      const current = this.facts.generation(route).seek;
      if (seek < current) {
        this.earlyOverflow.delete(native);
        if (this.overflowedSubs.get(native)?.seek === seek) this.overflowedSubs.delete(native);
      } else if (seek === current) {
        this.earlyOverflow.delete(native);
        const ov = this.overflowedSubs.get(native);
        for (const [subId, sub] of this.subs) if (sub.route === route) {
          ov?.subs.delete(subId);
          this.failSub(subId, EARLY_OVERFLOW);
        }
        if (ov && !ov.subs.size) this.overflowedSubs.delete(native);
      }
    }
    for (const [native, ov] of [...this.overflowedSubs]) {
      const route = this.routes.get(native);
      if (route !== undefined && ov.seek < this.facts.generation(route).seek) this.overflowedSubs.delete(native);
    }
    const duePanels = this.earlyPanels.filter(item => (this.current(item) ?? 1) <= 0);
    if (duePanels.length) {
      this.earlyPanels = this.earlyPanels.filter(item => !duePanels.includes(item));
      for (const item of duePanels) {
        this.earlyBytes -= item.bytes.byteLength + HELD_COST;
        this.panels(item.bytes);
      }
    }
    const due = this.early.filter(item => (this.current(item) ?? 1) <= 0);
    if (due.length) {
      this.early = this.early.filter(item => !due.includes(item));
      for (const item of due) {
        this.earlyBytes -= item.bytes.byteLength + HELD_COST;
        this.follows(item.bytes, item.stamp);
      }
    }
  }

  private replay() {
    const stale = this.held.filter(item => this.current(item) === -1);
    this.release(stale);
    this.held = this.held.filter(item => !stale.includes(item));
    const ready = this.held.filter(item => this.known(item.id) && this.routes.has(item.native) && this.current(item) === 0);
    if (!ready.length) { this.reindex(); return; }
    this.held = this.held.filter(item => !ready.includes(item));
    for (const item of ready) this.heldBytes -= item.size + HELD_COST;
    this.reindex();
    for (const item of ready) {
      if (this.failedSubs.has(item.id) || (item.id >= CELL && !this.facts.owns(item.id))) {
        this.freed += item.size;
        continue;
      }
      const route = this.routes.get(item.native)!, only = new Set([item.id]);
      if (item.kind === 'follow') {
        const limited = new Map<number, number>();
        let seen = false;
        for (const record of item.records) {
          const size = charged(record);
          if (this.failedSubs.has(item.id) || (item.id >= CELL && !this.facts.owns(item.id))) {
            this.freed += size;
            continue;
          }
          const view = new DataView(record.buffer, record.byteOffset, record.byteLength);
          if (!view.getUint32(8, true)) {
            limited.set(item.id, view.getInt32(4, true));
            this.freed += size;
            this.limitedSubs.add(item.id);
          } else if (this.commitFollowAnswer(route, item.seek, item.id, record, size, seen)) {
            seen = true;
          }
        }
        if (this.failedSubs.has(item.id) || (item.id >= CELL && !this.facts.owns(item.id))) continue;
        this.facts.confirmFollowed(route, item.seek, item.hour, limited, only);
        this.markSub(item.id, true);
      } else {
        for (const record of item.records) {
          if (this.failedSubs.has(item.id)) {
            this.freed += 2 * record.length;
            continue;
          }
          const made = this.panel(record);
          if (made) {
            this.facts.panels(route, item.seek, [made], undefined, only);
            this.markSub(item.id, made.kind !== 'stands');
            this.noteWaiting(made.id, made.kind === 'waiting');
          }
        }
        if (this.failedSubs.has(item.id)) continue;
        if (item.completed !== undefined) {
          this.facts.panels(route, item.seek, [], item.completed, only);
          this.markSub(item.id, false);
        }
      }
    }
    this.replay();
  }

  private commitFollowAnswer(route: number, seek: number, id: number, record: Uint8Array, bytes: number, extendPrevious: boolean): boolean {
    if (this.failedSubs.has(id) || (id >= CELL && !this.facts.owns(id))) {
      this.freed += bytes;
      return false;
    }
    const view = new DataView(record.buffer, record.byteOffset, record.byteLength), at = view.getInt32(4, true);
    let value: AnswerValue;
    try {
      const parsed = parseAnswer(record.subarray(16, 16 + view.getUint32(8, true)));
      if (parsed.kind === 'changed') {
        const base = this.facts.latest(id);
        if (!base || base.kind !== 'answer') throw new Error('Changed rows arrived without the table they change.');
        const patched = patchAnswer(base.answer, parsed);
        value = { kind: 'answer', answer: patched };
      } else {
        const body = record.slice(16);
        value = { kind: 'answer', body, answer: parseAnswer(body) };
      }
    } catch (error) {
      this.freed += bytes;
      this.failSub(id, error instanceof Error ? error.message : String(error));
      return false;
    }
    this.facts.addFollowedAnswer(route, seek, { id, hour: at, value, bytes }, extendPrevious);
    this.markSub(id, true);
    return true;
  }

  private follows(bytes: Uint8Array, stamped?: number) {
    let message: ReturnType<typeof parseFollowedFrame>;
    try { message = parseFollowedFrame(bytes); }
    catch (error) { this.freed += recordsCharge(bytes); throw error; }
    if (stamped !== undefined) this.stamped.set(message.route, Math.max(this.stamped.get(message.route) ?? 0, stamped));
    const order = this.current({ native: message.route, seek: message.seek });
    const data = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const count = message.count;
    if (order === -1) { this.freed += recordsCharge(bytes); return; }
    if (order === 1) { this.queueEarlyFollow(message.route, message.seek, bytes, stamped); return; }
    if (order === null && !this.acceptUnboundSeek(message.route, message.seek, recordsCharge(bytes))) return;
    if (message.reset && order === null) this.unboundReset(message.route, message.seek);
    else if (message.reset) {
      this.resets.set(message.route, (this.resets.get(message.route) ?? 0) + 1);
      this.drop(message.route);
    }
    const stamp = this.stamped.get(message.route);
    if (stamp !== undefined) this.resets.set(message.route, stamp);
    const route = this.routes.get(message.route);
    const limited = new Map<number, number>(), unbound = new Map<number, { records: Uint8Array[]; size: number }>();
    const seenInPacket = new Set<number>();
    if (route !== undefined) {
      const behind = stamped !== undefined && (this.applied.get(message.route) ?? stamped) < stamped, reset = message.reset || behind;
      if (stamped !== undefined) this.applied.set(message.route, stamped);
      if (reset) {
        this.routeConfirmedFollowed.delete(route);
        this.toldConfirmedFollowed.delete(route);
        for (const [subId, sub] of this.subs) if (sub.route === route && sub.channel === 'followed') {
          this.limitedSubs.delete(subId);
        }
        for (const [subId, sub] of this.subs) if (sub.route === route && sub.channel === 'panel') {
          for (const cell of this.facts.usesOf(subId) ?? []) {
            this.limitedSubs.delete(cell);
          }
        }
        this.facts.resetFollowed(route, message.seek);
        this.markRoute(route, 'followed');
      }
    }
    let at = 24;
    for (let i = 0; i < count; i++) {
      const id = data.getUint32(at, true), hour = data.getInt32(at + 4, true), length = data.getUint32(at + 8, true), record = bytes.subarray(at, at + 16 + length);
      const size = charged(record);
      at += 16 + length;
      if (this.failedSubs.has(id)) { this.freed += size; continue; }
      if (route === undefined || !this.known(id)) {
        if (this.subs.has(id)) {
          let own = unbound.get(id);
          if (!own) unbound.set(id, own = { records: [], size: 0 });
          own.records.push(record.slice()); own.size += size;
        } else if (id >= CELL) {
          if (this.gone.has(id)) {
            if (length > 0 && isWholeFollowRecord(record)) this.gone.delete(id);
            else { this.freed += size; continue; }
          }
          let own = unbound.get(id);
          if (!own) unbound.set(id, own = { records: [], size: 0 });
          own.records.push(record.slice()); own.size += size;
        } else this.freed += size;
        continue;
      }
      if (!length) {
        limited.set(id, hour);
        this.freed += size;
        this.limitedSubs.add(id);
        this.markSub(id, true);
        continue;
      }
      if (this.commitFollowAnswer(route, message.seek, id, record, size, seenInPacket.has(id))) {
        seenInPacket.add(id);
      }
    }
    this.holdMessage('follow', message.route, message.seek, message.hour, unbound);
    if (route === undefined) return;
    this.facts.confirmFollowed(route, message.seek, message.hour, limited);
    for (const id of [...this.limitedSubs]) {
      const sub = this.subs.get(id);
      if (sub && sub.route === route && !limited.has(id)) {
        this.limitedSubs.delete(id);
        this.markSub(id, true);
      }
    }
    const prevConfirmed = this.routeConfirmedFollowed.get(route) ?? -Infinity;
    if (message.hour > prevConfirmed) this.routeConfirmedFollowed.set(route, message.hour);
    for (const wid of this.nonOpenFollowed) {
      const widget = this.widgets.get(wid);
      if (widget && this.routeOf(widget) === route) this.dirtyWidgets.add(wid);
    }
  }

  private panel(record: Uint8Array): PanelRecord<Value> | null {
    const size = 2 * record.length;
    const rawId = record.byteLength >= 4 ? new DataView(record.buffer, record.byteOffset, 4).getUint32(0, true) : -1;
    if (rawId >= 0 && this.failedSubs.has(rawId)) {
      this.freed += size;
      return null;
    }
    let panel: ReturnType<typeof parseDatedPanelRecord>;
    let cells: ReturnType<typeof cellsOf> = [];
    try {
      const previous = this.facts.latest(rawId);
      panel = parseDatedPanelRecord(record, previous?.kind === 'panel' ? previous : undefined);
      if (panel.panel && panel.panel.refused === null) cells = cellsOf(panel.panel.data);
    } catch (error) {
      this.freed += size;
      const reason = error instanceof Error ? error.message : String(error);
      if (this.subs.has(rawId)) this.failSub(rawId, reason);
      else this.host.fail(reason);
      return null;
    }
    if (panel.waiting) { this.freed += size; return { kind: 'waiting', id: panel.id, hour: panel.hour }; }
    if (panel.again) { this.freed += size; return { kind: 'again', id: panel.id, hour: panel.hour, stepped: panel.stepped }; }
    if (!panel.panel) {
      this.freed += size;
      return { kind: 'stands', id: panel.id, hour: panel.hour, stepped: panel.stepped };
    }
    if (panel.panel.refused !== null) return { kind: 'refused', id: panel.id, hour: panel.hour, reason: panel.panel.refused, stepped: panel.stepped, bytes: size,
      value: { kind: 'panel', bytes: null, data: undefined, refused: panel.panel.refused } };
    if (panel.panel.stopped !== undefined) return { kind: 'data', id: panel.id, hour: panel.hour, stepped: panel.stepped, bytes: size, cells: [],
      value: { kind: 'panel', bytes: null, data: undefined, refused: null, stopped: panel.panel.stopped } };
    for (const cell of cells) {
      if (this.lost.has(cell.id)) {
        this.freed += size;
        this.failSub(panel.id, LOST);
        return null;
      }
      this.gone.delete(cell.id);
    }
    return { kind: 'data', id: panel.id, hour: panel.hour, stepped: panel.stepped, bytes: size, cells,
      value: { kind: 'panel', bytes: panel.panel.bytes ?? null, data: panel.panel.data, refused: null } };
  }

  private noteWaiting(id: number, waiting: boolean) {
    if (waiting === this.waitingSubs.has(id)) return;
    if (waiting) this.waitingSubs.add(id); else this.waitingSubs.delete(id);
    this.host.waitSub?.(id, waiting);
  }

  private panels(bytes: Uint8Array) {
    const data = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const native = data.getUint32(16, true), seek = data.getUint32(20, true), count = data.getUint32(8, true);
    const order = this.current({ native, seek });
    if (order === -1) { this.freed += panelRecordsCharge(bytes); return; }
    if (order === 1) { this.queueEarlyPanel(native, seek, bytes); return; }
    if (order === null && !this.acceptUnboundSeek(native, seek, panelRecordsCharge(bytes))) return;
    const route = this.routes.get(native);
    let at = 24, completed: number | undefined;
    const unbound = new Map<number, { records: Uint8Array[]; size: number }>();
    for (let i = 0; i < count; i++) {
      const id = data.getUint32(at, true), kind = data.getUint8(at + 8), length = data.getUint32(at + 12, true);
      const size = ((16 + length + 7) & ~7);
      const start = at;
      at += size;
      if (kind === 4) {
        completed = data.getInt32(start + 4, true);
        if (route !== undefined) {
          this.facts.panels(route, seek, [], completed);
          const prev = this.routeConfirmedPanel.get(route) ?? -Infinity;
          if (completed > prev) this.routeConfirmedPanel.set(route, completed);
          for (const wid of this.nonOpenPanel) {
            const widget = this.widgets.get(wid);
            if (widget && this.routeOf(widget) === route) this.dirtyWidgets.add(wid);
          }
        }
        continue;
      }
      if (this.failedSubs.has(id)) { this.freed += 2 * size; continue; }
      if (route === undefined || !this.known(id)) {
        if (this.subs.has(id)) {
          let own = unbound.get(id);
          if (!own) unbound.set(id, own = { records: [], size: 0 });
          own.records.push(bytes.slice(start, start + size)); own.size += 2 * size;
        } else this.freed += 2 * size;
        continue;
      }
      const made = this.panel(bytes.subarray(start, start + size));
      if (made) {
        this.facts.panels(route, seek, [made]);
        this.markSub(made.id, made.kind !== 'stands');
        this.noteWaiting(made.id, made.kind === 'waiting');
      }
    }
    this.holdMessage('panels', native, seek, completed ?? data.getInt32(12, true), unbound, completed);
  }

  private applyAcks(installed: readonly Ack[], refused: readonly Refusal[]) {
    const began = this.time();
    this.acknowledge(installed, refused);
    for (const id of this.facts.drain()) this.gone.add(id);
    const bytes = this.freed + this.releasedFacts();
    this.freed = 0;
    if (bytes) this.host.release(bytes);
    this.ackMs += Math.max(0, this.time() - began);
  }

  private flushAcks() {
    if (!this.acks.length) return;
    const began = this.time();
    const batches = this.acks.splice(0);
    for (const batch of batches) this.acknowledge(batch.installed, batch.refused);
    for (const id of this.facts.drain()) this.gone.add(id);
    const bytes = this.freed + this.releasedFacts();
    this.freed = 0;
    if (bytes) this.host.release(bytes);
    this.ackMs += Math.max(0, this.time() - began);
  }

  frame(request: FrameRequest) {
    this.pageRequests++;
    if (this.lastSpeed !== request.speed) {
      this.lastSpeed = request.speed;
      for (const widget of this.widgets.values()) {
        this.host.views(widget.view).onSpeed?.(request.speed, widget.state);
      }
    }
    if (request.installed.length || request.refused.length) {
      if (this.serving) this.acks.push({ installed: request.installed, refused: request.refused });
      else this.applyAcks(request.installed, request.refused);
    }
    const bare: FrameRequest = { ...request, installed: [], refused: [] };
    this.request = bare;
    this.lastExact = request.exact ? bare : null;
    if (request.exact) this.pendingExact = bare;
    else if (this.pendingExact?.seek === request.seek) {
      this.pendingExact = { ...bare, exact: true, target: request.now };
    }
    this.schedule();
  }

  private wake() {
    if (!this.request && this.lastExact) this.request = { ...this.lastExact, installed: [], refused: [] };
    this.schedule();
  }

  private schedule() {
    if (!this.request) return;
    if (this.request.exact && this.inHandover) {
      this.inHandover = false;
      this.handoverToken++;
      this.working = false;
    }
    if (this.working) return;
    this.working = true;
    queueMicrotask(() => { void this.run(); });
  }

  private armMotion() {
    if (!this.pacing && this.motionWidgets.size > 0) {
      this.pacing = true;
      this.host.later(() => { this.pacing = false; this.wake(); });
    }
  }

  private pendingRequest(): FrameRequest | null {
    return this.pendingExact ?? this.request;
  }

  private finishRun() {
    this.working = false;
    if (this.request) this.schedule();
    else this.armMotion();
  }

  private async run() {
    const request = this.pendingRequest();
    this.pendingExact = null;
    if (request === this.request) this.request = null;
    const ackMs = this.ackMs;
    this.ackMs = 0;
    const began = this.time() - ackMs;
    this.serving = true;
    try { if (request) await this.serve(request, began); }
    catch (error) { this.host.fail(error instanceof Error ? error.message : String(error)); }
    finally {
      this.serving = false;
      this.flushAcks();
      const deferredExact = this.deferredExact;
      this.deferredExact = false;
      const next = this.pendingRequest();
      const overBudget = this.time() - began >= (this.host.budget ?? 10);
      if (this.host.schedule && (!next?.exact || deferredExact) && (next || overBudget)) {
        if (!next) this.armMotion();
        const token = ++this.handoverToken;
        this.inHandover = true;
        this.host.schedule(() => {
          if (!this.inHandover || token !== this.handoverToken) return;
          this.inHandover = false;
          this.finishRun();
        });
      } else if (deferredExact && !this.host.schedule) {
        this.working = false;
        if (!this.pacing) {
          this.pacing = true;
          this.host.later(() => { this.pacing = false; this.wake(); });
        }
      } else {
        this.finishRun();
      }
    }
  }

  private acknowledge(installed: readonly Ack[], refused: readonly Refusal[]) {
    for (const ack of installed) {
      const owner = this.presented.get(ack.id);
      this.presented.delete(ack.id);
      const widget = owner === undefined ? undefined : this.widgets.get(owner.widget);
      if (!widget || !owner) continue;
      const sameOwner = owner.sent.owner === undefined || widget.binding.owner === undefined || owner.sent.owner === widget.binding.owner;
      if (sameOwner && widget.withdrawn === null && (!widget.shown || widget.shown.id < owner.sent.id)) widget.shown = owner.sent;
      if (owner.sent.binding !== widget.binding.generation) continue;
      if (!widget.installed || widget.installed.id < owner.sent.id) widget.installed = owner.sent;
      if (widget.inflight?.id === ack.id) widget.inflight = null;
      for (const sub of widget.subs) this.schedulePrune(sub);
      this.syncWindowTracking(widget);
    }
    for (const refusal of refused) {
      const owner = this.presented.get(refusal.id);
      this.presented.delete(refusal.id);
      this.stats.refused[refusal.reason] = (this.stats.refused[refusal.reason] ?? 0) + 1;
      const widget = this.widgets.get(refusal.widget);
      if (!widget) continue;
      if (widget.inflight?.id === refusal.id) widget.inflight = null;
      if (this.isDirectTarget(widget.id)) this.markDirectDirty(widget.id);
      if (!owner || owner.sent.binding !== widget.binding.generation || (widget.installed && refusal.id < widget.installed.id)) {
        for (const sub of widget.subs) this.schedulePrune(sub);
        this.syncWindowTracking(widget);
        continue;
      }
      if (refusal.reason === 'failed' || refusal.reason === 'refused') widget.halted = widget.binding.generation;
      if (refusal.reason === 'kept content') { widget.installed = null; widget.shown = null; }
      widget.parked = refusal.reason === 'uncovered' ? owner.sent : null;
      widget.sent = { ...widget.installed?.contents ?? {} };
      this.dirtyWidgets.add(widget.id);
      for (const sub of widget.subs) this.schedulePrune(sub);
      this.syncWindowTracking(widget);
    }
  }

  private targetOf(widget: Widget, hour: number) { return widget.binding.pin ?? hour + widget.binding.offset; }

  private activeWorkerCanvas(widgetId: number): WorkerCanvasInfo | undefined {
    return this.disabledWorkerCanvases.has(widgetId) ? undefined : this.workerCanvases.get(widgetId);
  }

  private markDirectDirty(widgetId: number) {
    if (this.sharedSurface?.slots.get(widgetId)?.visible) this.sharedSurface.dirtySlots.add(widgetId);
    if (this.activeWorkerCanvas(widgetId)) this.dirtyWorkerCanvases.add(widgetId);
    const held = this.widgets.get(widgetId);
    if (held) held.forceFull = true;
    this.dirtyWidgets.add(widgetId);
  }

  private keptPicture(widget: Widget): Sent | null {
    const kept = widget.installed && widget.shown
      ? (widget.shown.id > widget.installed.id ? widget.shown : widget.installed)
      : (widget.installed ?? (widget.withdrawn === null ? widget.shown : null));
    if (!kept || kept.mode !== 'picture') return null;
    return kept.owner !== undefined && widget.binding.owner !== undefined && kept.owner !== widget.binding.owner ? null : kept;
  }

  private clearDirectSlot(shared: SharedLaneCanvas | null, slot: SharedSurfaceSlot | undefined, wc: WorkerCanvasInfo | undefined) {
    if (shared && slot && slot.visible && shared.width > 0 && shared.height > 0) {
      const dpr = shared.dpr > 0 ? shared.dpr : 1;
      shared.ctx.save();
      shared.rawSetTransform(1, 0, 0, 1, 0, 0);
      const x = Math.floor(slot.x * dpr), y = Math.floor(slot.y * dpr), w = Math.ceil(slot.w * dpr), h = Math.ceil(slot.h * dpr);
      if (shared.surfaceColor) {
        shared.ctx.fillStyle = shared.surfaceColor;
        shared.ctx.fillRect(x, y, w, h);
      } else {
        shared.ctx.clearRect(x, y, w, h);
      }
      shared.ctx.restore();
    } else if (wc) {
      const ctx = wc.canvas.getContext('2d', wc.opaque ? { alpha: false } : undefined);
      if (ctx) {
        ctx.save();
        ctx.setTransform(1, 0, 0, 1, 0, 0);
        ctx.clearRect(0, 0, wc.canvas.width, wc.canvas.height);
        ctx.restore();
      }
    }
  }

  private restoreDirtyDirectSlots(request: FrameRequest, now: number) {
    const shared = this.sharedSurface;
    const sharedReady = shared !== null && shared.width > 0 && shared.height > 0;
    const dirty = new Set<number>();
    if (sharedReady) for (const wid of shared.dirtySlots) dirty.add(wid);
    for (const wid of this.dirtyWorkerCanvases) if (!sharedReady || !shared.slots.get(wid)?.visible) dirty.add(wid);
    for (const wid of this.dirtyWidgets) if (this.widgets.get(wid)?.forceFull && this.isDirectTarget(wid)) dirty.add(wid);
    for (const wid of dirty) {
      const widget = this.widgets.get(wid);
      if (!widget) continue;
      const wc = this.activeWorkerCanvas(wid);
      const slot = sharedReady ? shared.slots.get(wid) : undefined;
      const sharedArg = sharedReady && slot?.visible ? { lane: shared, slot, dirty: true } : undefined;
      if (!sharedArg && !wc) continue;
      const halted = widget.halted === widget.binding.generation || this.failedWidgets.has(wid);
      const kept = halted ? null : this.keptPicture(widget);
      if (!kept) {
        this.clearDirectSlot(sharedArg ? shared : null, sharedArg?.slot, sharedArg ? undefined : wc);
        continue;
      }
      try {
        this.host.views(widget.view).paint({
          widget: wid, subs: kept.subsSnapshot, pictures: kept.pictures, model: kept.model,
          ...(kept.owner !== undefined ? { owner: kept.owner } : {}),
          hour: kept.lo, seek: request.seek, speed: request.speed, binding: kept.bindingSnapshot, inspect: kept.inspect, now,
          ...(wc && !sharedArg ? { workerCanvas: { ...wc, dirty: true } } : {}),
          ...(sharedArg ? { sharedSurface: sharedArg } : {}),
        }, widget.state, this.content);
        if (sharedArg && shared) shared.dirtySlots.delete(wid);
        if (wc && !sharedArg) this.dirtyWorkerCanvases.delete(wid);
        widget.forceFull = false;
      } catch {
        this.stats.restoreFailures++;
      }
    }
  }

  private willPageRefuseDirect(widget: Widget, selected: Selected, target: number, request: FrameRequest): boolean {
    if (this.seekSuperseded(request.seek) || widget.withdrawn === widget.binding.generation || target < selected.window[0] || target > selected.window[1]) return true;
    if (!request.exact || !this.keptPicture(widget)) return false;
    const route = this.routeOf(widget);
    const open = this.isOpenFollowed(widget, selected) ? 'followed' : this.isOpenPanel(widget, selected) ? 'panel' : undefined;
    const confirmed = open && route !== null
      ? Math.max(selected.c, (open === 'followed' ? this.routeConfirmedFollowed : this.routeConfirmedPanel).get(route) ?? -Infinity)
      : selected.c;
    return target > confirmed;
  }

  private isDirectTarget(widgetId: number): boolean {
    if (this.activeWorkerCanvas(widgetId)) return true;
    const shared = this.sharedSurface;
    return Boolean(shared && shared.width > 0 && shared.height > 0 && shared.slots.get(widgetId)?.visible);
  }

  private isDirectSlotDirty(widgetId: number): boolean {
    const shared = this.sharedSurface;
    if (shared && shared.width > 0 && shared.height > 0 && shared.slots.get(widgetId)?.visible) {
      return shared.dirtySlots.has(widgetId);
    }
    return this.activeWorkerCanvas(widgetId) !== undefined && this.dirtyWorkerCanvases.has(widgetId);
  }

  private requestHourFor(widget: Widget, request: FrameRequest): number {
    return this.targetOf(widget, this.isDirectTarget(widget.id) ? request.now : request.target);
  }

  private select(widget: Widget, target: number): Selected | null {
    const pictures: Picture<Value>[] = [];
    let joined: { h: number; c: number; window: Window } | null;
    if (!widget.subs.length) {
      const dated = widget.binding.dated;
      if (!dated || target < dated.h) return null;
      joined = { h: dated.h, c: dated.c, window: [dated.h, Infinity] };
    } else {
      const components: { picture: Picture<Value>; window: Window }[] = [];
      for (const id of widget.subs) {
        if (this.failedSubs.has(id)) return null;
        const sub = this.subs.get(id);
        if (!sub || sub.route !== null && this.failed.has(sub.route)) return null;
        const picture = this.facts.select(id, target);
        if (!picture) return null;
        const window = this.facts.window(id, target, picture);
        if (!window) return null;
        pictures.push(picture);
        components.push({ picture, window });
      }
      joined = joinComponents(components);
      const dated = widget.binding.dated;
      if (joined && dated) {
        const lo = Math.max(joined.window[0], dated.h);
        const c = Math.min(joined.c, dated.c);
        if (target < dated.h || lo > c) return null;
        joined = { h: Math.max(joined.h, dated.h), c, window: [lo, joined.window[1]] };
      }
    }
    if (!joined) return null;
    const view = this.host.views(widget.view);
    if (!view.next) return { pictures, ...joined };
    const scene: Scene = { widget: widget.id, subs: widget.subs.map(id => this.subs.get(id)!), pictures, model: widget.model, ...widget.binding.owner !== undefined ? { owner: widget.binding.owner } : {} };
    const version = `${widget.subs.length ? pictures.map(picture => picture.parts.map(part => part.version).join('.')).join('|') : `bound:${widget.binding.generation}`}:${widget.supply}`;
    const at = Math.max(joined.window[0], Math.min(target, joined.c));
    let window = changeWindow(joined.window, at, hour => view.next!(scene, hour, widget.state), widget.start?.version === version ? widget.start.hour : undefined);
    if (target > window[1]) window = [window[0], joined.window[1]];
    widget.start = { version, hour: window[0] };
    return { pictures, window, h: joined.h, c: joined.c };
  }

  private isOpenFollowed(widget: Widget, selected: Selected): boolean {
    if (!widget.subs.length || selected.window[1] !== Infinity || this.host.views(widget.view).next) return false;
    if (widget.binding.dated && Number.isFinite(widget.binding.dated.c)) return false;
    const route = this.routeOf(widget);
    if (route === null) return false;
    const confirmed = this.routeConfirmedFollowed.get(route);
    if (confirmed === undefined || selected.c < confirmed) return false;
    for (let i = 0; i < widget.subs.length; i++) {
      const id = widget.subs[i]!, sub = this.subs.get(id), picture = selected.pictures[i]!;
      if (!sub || sub.channel !== 'followed' || sub.route !== route || this.limitedSubs.has(id)) return false;
      if (picture.carried || picture.parts.length !== 1) return false;
    }
    return true;
  }

  private isOpenPanel(widget: Widget, selected: Selected): boolean {
    if (!widget.subs.length || selected.window[1] !== Infinity || this.host.views(widget.view).next) return false;
    if (widget.binding.dated && Number.isFinite(widget.binding.dated.c)) return false;
    const route = this.routeOf(widget);
    if (route === null) return false;
    const confirmed = this.routeConfirmedPanel.get(route);
    if (confirmed === undefined || selected.c < confirmed) return false;
    for (let i = 0; i < widget.subs.length; i++) {
      const id = widget.subs[i]!, sub = this.subs.get(id), picture = selected.pictures[i]!;
      if (!sub || sub.channel !== 'panel' || sub.route !== route || !this.facts.isOpenSteppedPanel(id)) return false;
      if (picture.carried || picture.parts.length !== 1) return false;
    }
    return true;
  }

  private matches(sent: Sent | null, selected: Selected, target: number, binding: number, supply: number, inspect: string | null, route: number | null, seek: number) {
    if (!sent || (sent.ephemeral && sent.seek !== seek) || sent.binding !== binding || sent.supply !== supply || sent.inspect !== inspect || sent.pictures.length !== selected.pictures.length) return false;
    const confirmed = sent.open === 'followed' && route !== null
      ? Math.max(sent.c, this.routeConfirmedFollowed.get(route) ?? -Infinity)
      : sent.open === 'panel' && route !== null
      ? Math.max(sent.c, this.routeConfirmedPanel.get(route) ?? -Infinity)
      : sent.c;
    return confirmed === selected.c && sent.pictures.every((picture, at) => samePicture(picture, selected.pictures[at]))
      && sent.lo === selected.window[0] && sent.lo <= target && target <= sent.hi;
  }

  private canRedate(sent: Sent | null, selected: Selected, binding: number, supply: number, inspect: string | null, seek: number) {
    return !!sent && (!sent.ephemeral || sent.seek === seek) && sent.binding === binding && sent.supply === supply && sent.inspect === inspect
      && sent.pictures.length === selected.pictures.length
      && sent.pictures.every((picture, at) => samePicture(picture, selected.pictures[at]))
      && sent.lo === selected.window[0]
      && selected.window[1] <= sent.hi;
  }

  private hasPresented(widget: number): boolean {
    for (const owner of this.presented.values()) if (owner.widget === widget) return true;
    return false;
  }

  private seekSuperseded(seek: number): boolean {
    for (const route of this.natives.keys()) {
      if (this.facts.generation(route).seek > seek) return true;
    }
    return false;
  }

  private async serve(request: FrameRequest, began = this.time()) {
    this.requests++;
    this.stats.frames++;
    if (this.seekSuperseded(request.seek)) {
      if (request.installed.length || request.refused.length) this.acknowledge(request.installed, request.refused);
      for (const id of this.facts.drain()) this.gone.add(id);
      const bytes = this.freed + this.releasedFacts();
      this.freed = 0;
      if (bytes) this.host.release(bytes);
      this.host.stretch?.(this.time() - began);
      this.host.served?.({ seek: request.seek, target: request.target, exact: request.exact, changed: 0, ms: this.time() - began, cancelled: true });
      return;
    }
    this.lastTarget = request.target;
    this.lastNow = request.now;
    this.acknowledge(request.installed, request.refused);
    this.restoreDirtyDirectSlots(request, began);
    const inspect = new Map(request.inspect);
    const candidates = new Set<number>();
    for (const wid of this.dirtyWidgets) candidates.add(wid);
    this.dirtyWidgets.clear();
    for (const wid of this.motionWidgets) candidates.add(wid);
    for (const wid of this.ephemeralWidgets) {
      const widget = this.widgets.get(wid);
      const active = widget?.inflight ?? widget?.installed;
      if (active?.ephemeral && active.seek !== request.seek) candidates.add(wid);
    }
    for (const wid of this.inflightWidgets) {
      this.stats.visited++;
      const widget = this.widgets.get(wid);
      if (widget?.inflight && this.pageRequests - widget.inflight.request > STALE) candidates.add(wid);
    }
    if (!request.exact && this.parkedWidgets.size) {
      for (const wid of this.parkedWidgets) {
        this.stats.visited++;
        const widget = this.widgets.get(wid);
        if (widget) { widget.parked = null; candidates.add(wid); }
      }
      this.parkedWidgets.clear();
    }
    for (const wid of this.inspectedWidgets) {
      this.stats.visited++;
      const widget = this.widgets.get(wid);
      if (widget && (inspect.get(wid) ?? null) !== widget.inspect) candidates.add(wid);
    }
    for (const [wid, key] of inspect) {
      this.stats.visited++;
      const widget = this.widgets.get(wid);
      if (widget && widget.inspect !== key) candidates.add(wid);
    }
    if (this.windowedWidgets.size && (this.windowSafeDirty || request.target < this.windowSafeLo || request.target > this.windowSafeHi || request.now < this.windowSafeLo || request.now > this.windowSafeHi)) {
      let lo = -Infinity, hi = Infinity;
      for (const wid of this.windowedWidgets) {
        this.stats.visited++;
        const widget = this.widgets.get(wid);
        if (!widget) continue;
        const base = this.isDirectTarget(wid) ? request.now : request.target;
        const target = this.targetOf(widget, base);
        const w = widget.validWindow;
        const active = widget.inflight ?? widget.installed;
        if ((!active && !w) || (w && (target < w[0] || target > w[1])) || (active && (target < active.lo || target > active.hi))) {
          candidates.add(wid);
        }
        if (w) [lo, hi] = this.clampSafeRange(lo, hi, w[0], w[1], widget.binding.offset, base);
        if (active) [lo, hi] = this.clampSafeRange(lo, hi, active.lo, active.hi, widget.binding.offset, base);
      }
      this.windowSafeLo = lo;
      this.windowSafeHi = hi;
      this.windowSafeDirty = false;
    }
    const due: { widget: Widget; selected: Selected; motion: boolean; target: number }[] = [];
    for (const wid of candidates) {
      const widget = this.widgets.get(wid);
      if (!widget || widget.halted === widget.binding.generation || this.failedWidgets.has(wid)) continue;
      this.stats.visited++;
      if (!request.exact && widget.parked) { widget.parked = null; this.parkedWidgets.delete(wid); }
      const key = inspect.get(widget.id) ?? null, target = this.requestHourFor(widget, request);
      const route = this.routeOf(widget);
      const selected = this.select(widget, target);
      if (!selected) {
        let next = Infinity;
        if (!widget.subs.length) {
          if (widget.binding.dated && widget.binding.dated.h > target) next = widget.binding.dated.h;
        } else if (!widget.subs.some(id => this.failedSubs.has(id))) {
          for (const id of widget.subs) {
            const h = this.facts.nextStart(id, target);
            if (h < next) next = h;
          }
          if (widget.binding.dated && widget.binding.dated.h > target && widget.binding.dated.h < next) next = widget.binding.dated.h;
        }
        widget.validWindow = Number.isFinite(next) ? [-Infinity, next - 1] : null;
        this.syncWindowTracking(widget);
        widget.inspect = key;
        if (key === null) this.inspectedWidgets.delete(wid); else this.inspectedWidgets.add(wid);
        continue;
      }
      widget.validWindow = selected.window;
      this.syncWindowTracking(widget);
      const shown = this.matches(widget.installed, selected, target, widget.binding.generation, widget.supply, key, route, request.seek);
      const slotDirty = this.isDirectSlotDirty(wid);
      if (this.matches(widget.parked, selected, target, widget.binding.generation, widget.supply, key, route, request.seek)) continue;
      const pending = widget.inflight && this.pageRequests - widget.inflight.request <= STALE && this.matches(widget.inflight, selected, target, widget.binding.generation, widget.supply, key, route, request.seek);
      if ((pending || (shown && !widget.motion)) && !widget.forceFull && !slotDirty) {
        widget.inspect = key;
        if (key === null) this.inspectedWidgets.delete(wid); else this.inspectedWidgets.add(wid);
        continue;
      }
      due.push({ widget, selected, motion: shown && !widget.forceFull && !slotDirty, target });
      widget.inspect = key;
      if (key === null) this.inspectedWidgets.delete(wid); else this.inspectedWidgets.add(wid);
    }
    const starved = (w: Widget) => w.deferrals >= 3 ? 0 : 1;
    due.sort((a, b) => {
      const sa = starved(a.widget), sb = starved(b.widget);
      if (sa !== sb) return sa - sb;
      if (sa === 0) return b.widget.deferrals - a.widget.deferrals || a.widget.served - b.widget.served || Number(a.motion) - Number(b.motion) || a.widget.order - b.widget.order;
      return Number(a.motion) - Number(b.motion) || a.widget.served - b.widget.served || a.widget.order - b.widget.order;
    });
    const budget = this.host.budget ?? 10;
    const entries: Entry<B>[] = [], surfaces: (B | typeof SAME | null)[][] = [], sources: Bitmapped[] = [], slots: [number, number, number][] = [];
    const made: { widget: Widget; selected: Selected }[] = [];
    const shared = new Map<string, number>();
    const unpresentedDirect = new Set<number>();
    let tried = 0;
    let deferred = false;
    try {
      for (let i = 0; i < due.length; i++) {
        const { widget, selected, motion, target } = due[i]!;
        const elapsed = this.time() - began;
        if (tried > 0 && elapsed + widget.cost > budget) {
          deferred = true;
          widget.deferrals++;
          this.dirtyWidgets.add(widget.id);
          if (elapsed >= budget) {
            for (let j = i + 1; j < due.length; j++) {
              due[j]!.widget.deferrals++;
              this.dirtyWidgets.add(due[j]!.widget.id);
            }
            break;
          }
          continue;
        }
        tried++;
        widget.deferrals = 0;
        const start = this.time();
        widget.served = this.requests;
        const initialOpen = this.isOpenFollowed(widget, selected) ? 'followed' : this.isOpenPanel(widget, selected) ? 'panel' : undefined;
        const slotDirty = this.isDirectSlotDirty(widget.id);
        if (!motion && !widget.motion && !widget.forceFull && !slotDirty && widget.installed && this.canRedate(widget.installed, selected, widget.binding.generation, widget.supply, widget.inspect, request.seek)) {
          const redated = this.redateEntry(widget, widget.installed, selected, request.seek, initialOpen);
          entries.push(redated);
          made.push({ widget, selected });
          surfaces.push([...redated.surfaces]);
          continue;
        }
        const view = this.host.views(widget.view);
        if (this.isDirectTarget(widget.id) && this.willPageRefuseDirect(widget, selected, target, request)) continue;
        let activeSelected = selected;
        let painted: Painted = { surfaces: [], fields: {}, motion: false };
        let mode: Entry['mode'] = 'picture', reason: string | undefined, failureOrigin: FailureOrigin | undefined;
        let failedLocally = false;
        for (let pass = 0; pass < 4; pass++) {
          try {
            const stops = activeSelected.pictures.map(stopOf).filter(stop => stop !== null);
            if (stops.length) {
              painted = { surfaces: [], fields: {}, motion: false };
              mode = 'failed'; reason = [...new Set(stops)].join('; '); failureOrigin = 'stream';
              break;
            }
            const wc = this.activeWorkerCanvas(widget.id);
            const shared = this.sharedSurface;
            const sharedSlot = shared && shared.width > 0 && shared.height > 0 ? shared.slots.get(widget.id) : undefined;
            const sharedArg = shared && sharedSlot && sharedSlot.visible
              ? { lane: shared, slot: sharedSlot, dirty: shared.dirtySlots.has(widget.id) }
              : undefined;
            if (sharedArg || wc) unpresentedDirect.add(widget.id);
            painted = view.paint({ widget: widget.id, subs: widget.subs.map(id => this.subs.get(id)!), pictures: activeSelected.pictures, model: widget.model, ...widget.binding.owner !== undefined ? { owner: widget.binding.owner } : {},
              hour: activeSelected.window[0], seek: request.seek, speed: request.speed, binding: widget.binding, inspect: widget.inspect, now: start,
              ...(wc && !sharedArg ? { workerCanvas: this.dirtyWorkerCanvases.has(widget.id) ? { ...wc, dirty: true } : wc } : {}),
              ...(sharedArg ? { sharedSurface: sharedArg } : {}) }, widget.state, this.content);
            if (sharedArg && shared) shared.dirtySlots.delete(widget.id);
            if (wc && !sharedArg) this.dirtyWorkerCanvases.delete(widget.id);
            mode = painted.mode ?? 'picture'; reason = painted.reason; failureOrigin = undefined;
            const refusals = activeSelected.pictures.map(refusalOf);
            if (painted.mode === undefined && refusals.length && refusals.every(refusal => refusal !== null)) { mode = 'refused'; reason = [...new Set(refusals)].join('; '); }
            else if (mode === 'picture' && refusals.some(refusal => refusal !== null))
              painted = { ...painted, fields: { ...painted.fields, ...Object.fromEntries(refusals.flatMap((refusal, at) => refusal === null ? [] : [[`refused:${widget.subs[at]}`, { content: refusal, value: refusal }]])) } };
          } catch (error) {
            const message = 'drawing failed: ' + (error instanceof Error ? error.message : String(error));
            if (this.host.failWidget) {
              this.failWidget(widget, message, widget.supplied.stamp);
              failedLocally = true;
            } else {
              painted = { surfaces: [], fields: {}, motion: false };
              mode = 'failed'; reason = message; failureOrigin = 'local';
            }
            break;
          }
          if (painted.nextChangeAt !== undefined && painted.nextChangeAt > activeSelected.window[0] && (painted.nextChangeAt <= activeSelected.c || request.speed === 0)) {
            const clampedHi = Math.min(activeSelected.window[1], painted.nextChangeAt - 1);
            if (clampedHi < activeSelected.window[1]) {
              activeSelected = { ...activeSelected, window: [activeSelected.window[0], clampedHi] };
            }
            if (target > activeSelected.window[1] && pass < 3) {
              const nextSelected = this.select(widget, target);
              if (nextSelected && nextSelected.window[0] > activeSelected.window[0]) {
                activeSelected = nextSelected;
                continue;
              }
            }
          }
          break;
        }
        if (failedLocally) continue;
        if (painted.waiting) {
          widget.validWindow = null;
          widget.motion = false;
          this.motionWidgets.delete(widget.id);
          this.syncWindowTracking(widget);
          continue;
        }
        widget.validWindow = activeSelected.window;
        const paintMs = this.time() - start;
        widget.motion = painted.motion;
        if (widget.motion) this.motionWidgets.add(widget.id); else this.motionWidgets.delete(widget.id);
        widget.cost = widget.cost * 0.8 + paintMs * 0.2;
        widget.served = this.requests + Math.min(4, Math.floor(paintMs / budget));
        if (motion) this.stats.motion++;
        if (this.targetOf(widget, request.now) > activeSelected.window[1]) {
          this.syncWindowTracking(widget);
          continue;
        }
        const open = mode === 'failed' ? undefined : this.isOpenFollowed(widget, activeSelected) ? 'followed' : this.isOpenPanel(widget, activeSelected) ? 'panel' : undefined;
        const parts: (B | typeof SAME | null)[] = [];
        const emitted = mode === 'picture' ? painted : { ...painted, surfaces: [] };
        const entry = this.entry(widget, activeSelected, emitted, mode, reason, parts, request.seek, paintMs, open, failureOrigin);
        if (mode === 'picture') {
          painted.surfaces.forEach((surface, k) => {
            if (!surface || parts[k] === SAME) return;
            let source = shared.get(surface.content);
            if (source === undefined) { source = sources.push(surface.source) - 1; shared.set(surface.content, source); }
            slots.push([entries.length, k, source]);
          });
        }
        entries.push(entry);
        made.push({ widget, selected: activeSelected });
        surfaces.push(parts);
      }
      const resets: [number, number][] = [];
      const toldMoved: [number, number | undefined][] = [];
      let moved = false;
      for (const [route, native] of this.natives) {
        const count = this.resets.get(native) ?? 0;
        const prev = this.told.get(route);
        if (prev !== count) { moved = true; toldMoved.push([route, prev]); this.told.set(route, count); }
        resets.push([route, count]);
      }
      const confirmed: [number, 'followed' | 'panel', number, number, number][] = [];
      for (const [route, hour] of this.routeConfirmedFollowed) {
        if (this.toldConfirmedFollowed.get(route) !== hour) {
          this.toldConfirmedFollowed.set(route, hour);
          const native = this.natives.get(route);
          const seek = this.facts.generation(route).seek;
          const reset = native === undefined ? 0 : this.applied.get(native) ?? this.resets.get(native) ?? 0;
          confirmed.push([route, 'followed', hour, seek, reset]);
          moved = true;
        }
      }
      for (const [route, hour] of this.routeConfirmedPanel) {
        if (this.toldConfirmedPanel.get(route) !== hour) {
          this.toldConfirmedPanel.set(route, hour);
          const native = this.natives.get(route);
          const seek = this.facts.generation(route).seek;
          const reset = native === undefined ? 0 : this.applied.get(native) ?? this.resets.get(native) ?? 0;
          confirmed.push([route, 'panel', hour, seek, reset]);
          moved = true;
        }
      }
      const closed = this.pendingClosed.size ? [...this.pendingClosed].map(([widget, [lo, hi]]): [number, number, number] => [widget, lo, hi]) : undefined;
      if (closed) { this.pendingClosed.clear(); moved = true; }
      const withdrawn = this.pendingWithdrawn.size ? [...this.pendingWithdrawn].map(([widget, binding]): [number, number] => [widget, binding]) : undefined;
      if (withdrawn) { this.pendingWithdrawn.clear(); moved = true; }
      let crops: B[] = [];
      let failedEntries: Set<number> | null = null;
      try { crops = slots.length ? await this.host.crop(slots.map(([, , source]) => sources[source]!)) : []; }
      catch (error) {
        const reason = 'drawing failed: ' + (error instanceof Error ? error.message : String(error));
        if (this.host.failWidget) {
          failedEntries = new Set(slots.map(([e]) => e));
          for (const e of failedEntries) {
            const { widget } = made[e]!;
            const orig = entries[e]!;
            this.presented.delete(orig.id);
            const route = this.routeOf(widget);
            const stillCurrent = this.widgets.get(widget.id) === widget && widget.binding.generation === orig.binding && (route === null || this.facts.generation(route).seek === orig.gen.seek);
            if (stillCurrent) this.failWidget(widget, reason, orig.stamp);
            else if (widget.inflight?.id === orig.id) { widget.inflight = null; this.syncWindowTracking(widget); }
          }
        } else {
          for (const e of new Set(slots.map(([e]) => e))) {
            const { widget, selected } = made[e]!;
            this.presented.delete(entries[e]!.id);
            widget.sent = { ...widget.installed?.contents ?? {} };
            entries[e] = this.entry(widget, selected, { surfaces: [], fields: {}, motion: false }, 'failed', reason, [], request.seek, 0, undefined, 'local');
            surfaces[e] = [];
          }
        }
        slots.length = 0;
      }
      if (this.seekSuperseded(request.seek)) {
        for (const crop of crops) crop.close();
        for (let e = 0; e < entries.length; e++) {
          const orig = entries[e]!;
          const { widget } = made[e]!;
          this.presented.delete(orig.id);
          if (this.widgets.get(widget.id) === widget) {
            if (widget.inflight?.id === orig.id) widget.inflight = null;
            if (widget.binding.generation === orig.binding) { widget.forceFull = true; widget.sent = {}; }
            this.dirtyWidgets.add(widget.id);
            this.syncWindowTracking(widget);
          }
        }
        if (closed) for (const [wid, lo, hi] of closed) if (this.widgets.has(wid)) this.noteClosed(wid, lo, hi);
        if (withdrawn) for (const [wid, binding] of withdrawn) {
          if (this.widgets.has(wid) && (this.pendingWithdrawn.get(wid) ?? -1) < binding) this.pendingWithdrawn.set(wid, binding);
        }
        for (const [route, prev] of toldMoved) {
          if (!this.natives.has(route)) continue;
          if (prev === undefined) this.told.delete(route);
          else this.told.set(route, prev);
        }
        for (const id of this.facts.drain()) this.gone.add(id);
        const bytes = this.freed + this.releasedFacts();
        this.freed = 0;
        if (bytes) this.host.release(bytes);
        this.host.stretch?.(this.time() - began);
        this.host.served?.({ seek: request.seek, target: request.target, exact: request.exact, changed: 0, ms: this.time() - began, cancelled: true });
        return;
      }
      this.stats.crops += crops.length;
      slots.forEach(([e, k], at) => { surfaces[e]![k] = crops[at]!; });
      const presentEntries = failedEntries ? entries.filter((_, e) => !failedEntries.has(e)) : entries;
      if (presentEntries.length || moved) {
        this.host.present({ lane: this.host.lane, serial: request.serial, resets, ...closed ? { closed } : {}, ...withdrawn ? { withdrawn } : {}, ...confirmed.length ? { confirmed } : {}, entries: presentEntries }, crops);
      }
      for (const entry of presentEntries) unpresentedDirect.delete(entry.widget);
      this.stats.entries += presentEntries.length;
    } finally {
      for (const wid of unpresentedDirect) {
        if (this.isDirectTarget(wid)) this.markDirectDirty(wid);
      }
    }
    this.prune(request.now);
    for (const id of this.facts.drain()) this.gone.add(id);
    const bytes = this.freed + this.releasedFacts();
    this.freed = 0;
    if (bytes) this.host.release(bytes);
    this.host.stretch?.(this.time() - began);
    const costs: [number, number][] | undefined = made.length ? made.map(({ widget }) => [widget.id, widget.cost]) : undefined;
    this.host.served?.({ seek: request.seek, target: request.target, exact: request.exact, changed: entries.length, ms: this.time() - began, ...costs ? { costs } : {} });
    if (deferred && request.exact && !this.request && !this.pendingExact && this.lastExact?.seek === request.seek) {
      if (this.host.schedule) this.request = { ...this.lastExact, installed: [], refused: [] };
      this.deferredExact = true;
    }
  }

  private redateEntry(widget: Widget, base: Sent, selected: Selected, seek: number, open?: 'followed' | 'panel'): Entry<B> {
    const id = this.host.lane + 1 + this.host.lanes * this.nextEntry++;
    const contents: Record<string, string> = { ...base.contents, hour: String(selected.window[0]) };
    const parts: (B | typeof SAME | null)[] = Array.from({ length: base.surfaceCount }, (_, k) => base.contents[k] !== undefined ? SAME : null);
    const fields: Record<string, unknown> = {};
    for (const key of base.fieldKeys) fields[key] = SAME;
    const route = this.routeOf(widget);
    const gen = route === null ? { seek, reset: 0 } : { seek: this.facts.generation(route).seek, reset: this.applied.get(this.natives.get(route)!) ?? this.resets.get(this.natives.get(route)!) ?? 0 };
    const sent: Sent = { id, ...(widget.binding.owner !== undefined ? { owner: widget.binding.owner } : {}), request: this.pageRequests, seek, ephemeral: base.ephemeral, pictures: selected.pictures, h: selected.h, c: selected.c, lo: selected.window[0], hi: selected.window[1],
      binding: widget.binding.generation, contents, fieldKeys: base.fieldKeys, surfaceCount: base.surfaceCount, mode: base.mode, reason: base.reason, open, ...(base.viewport ? { viewport: base.viewport } : {}), inspect: widget.inspect, hit: base.hit, supply: base.supply,
      model: base.model, bindingSnapshot: base.bindingSnapshot, subsSnapshot: base.subsSnapshot };
    widget.withdrawn = null;
    widget.sent = { ...contents };
    widget.inflight = sent;
    this.presented.set(id, { widget: widget.id, sent });
    this.syncWindowTracking(widget);
    return Object.freeze({ widget: widget.id, binding: widget.binding.generation, ...(widget.binding.owner !== undefined ? { owner: widget.binding.owner } : {}), id, h: selected.h, c: selected.c, lo: selected.window[0], hi: selected.window[1],
      gen: Object.freeze(gen), follows: selected.pictures.some(picture => followsChannel(picture)), mode: base.mode, ...base.reason !== undefined ? { reason: base.reason } : {},
      ...open ? { open } : {}, ...(base.viewport ? { viewport: base.viewport } : {}), surfaces: parts, content: Object.freeze(contents), fields: Object.freeze(fields), ms: 0, ...widget.supplied.stamp !== undefined ? { stamp: widget.supplied.stamp } : {} });
  }

  private entry(widget: Widget, selected: Selected, painted: Painted, mode: Entry['mode'], reason: string | undefined, parts: (B | typeof SAME | null)[], seek: number, ms = 0, open?: 'followed' | 'panel', failureOrigin?: FailureOrigin): Entry<B> {
    const id = this.host.lane + 1 + this.host.lanes * this.nextEntry++, contents: Record<string, string> = { hour: String(selected.window[0]) };
    const full = widget.forceFull;
    if (mode === 'picture') widget.forceFull = false;
    if (mode === 'failed' && reason !== undefined) contents.failed = reason;
    const fields: Record<string, unknown> = {};
    if (painted.surfaces.length) {
      painted.surfaces.forEach((surface, k) => {
        if (!surface) { parts[k] = null; return; }
        contents[k] = surface.content;
        parts[k] = !full && widget.sent[k] === surface.content ? SAME : null;
      });
    } else if (painted.surfaceContent !== undefined) {
      contents[0] = painted.surfaceContent;
    }
    const fieldKeys = Object.keys(painted.fields);
    for (const [key, field] of Object.entries(painted.fields)) {
      contents[key] = field.content;
      fields[key] = !full && widget.sent[key] === field.content ? SAME : field.value;
    }
    const route = this.routeOf(widget);
    const gen = route === null ? { seek, reset: 0 } : { seek: this.facts.generation(route).seek, reset: this.applied.get(this.natives.get(route)!) ?? this.resets.get(this.natives.get(route)!) ?? 0 };
    const primarySurface = widget.binding.surfaces[0];
    const viewport = painted.viewport ?? (primarySurface ? { width: primarySurface.width, height: primarySurface.height, dpr: primarySurface.dpr, scrollLeft: 0, scrollTop: 0 } : undefined);
    const sent: Sent = { id, ...(widget.binding.owner !== undefined ? { owner: widget.binding.owner } : {}), request: this.pageRequests, seek, ephemeral: painted.ephemeral === true, pictures: selected.pictures, h: selected.h, c: selected.c, lo: selected.window[0], hi: selected.window[1],
      binding: widget.binding.generation, contents, fieldKeys, surfaceCount: painted.surfaces.length, mode, reason, open, ...(viewport ? { viewport } : {}), inspect: widget.inspect, hit: painted.hit, supply: widget.supply,
      model: widget.model, bindingSnapshot: widget.binding, subsSnapshot: widget.subs.map(id => this.subs.get(id)!) };
    widget.withdrawn = null;
    widget.sent = { ...contents };
    widget.inflight = sent;
    this.presented.set(id, { widget: widget.id, sent });
    this.syncWindowTracking(widget);
    return Object.freeze({ widget: widget.id, binding: widget.binding.generation, ...(widget.binding.owner !== undefined ? { owner: widget.binding.owner } : {}), id, h: selected.h, c: selected.c, lo: selected.window[0], hi: selected.window[1],
      gen: Object.freeze(gen), follows: selected.pictures.some(picture => followsChannel(picture)), mode, ...reason !== undefined ? { reason } : {},
      ...open ? { open } : {}, ...failureOrigin !== undefined ? { failureOrigin } : {}, ...(viewport ? { viewport } : {}),
      ...(mode === 'picture' && painted.repainted === true ? { repainted: true } : {}),
      surfaces: parts, content: Object.freeze(contents), fields: Object.freeze(fields), ms, ...widget.supplied.stamp !== undefined ? { stamp: widget.supplied.stamp } : {} });
  }

  private prune(hour: number) {
    if (hour >= this.minPruneHour) {
      let nextMin = Infinity;
      for (const [subId, due] of this.prunableAt) {
        if (due <= hour) {
          this.prunableAt.delete(subId);
          this.prunableNow.add(subId);
        } else if (due < nextMin) {
          nextMin = due;
        }
      }
      this.minPruneHour = nextMin;
    }
    if (!this.prunableNow.size) return;
    for (const subId of [...this.prunableNow]) {
      this.prunableNow.delete(subId);
      const sub = this.subs.get(subId);
      if (!sub) { this.prunableAt.delete(subId); continue; }
      let actual = Infinity;
      const keep: Picture<Value>[] = [];
      const widgets = this.subWidgets.get(subId);
      if (widgets) {
        for (const wid of widgets) {
          const widget = this.widgets.get(wid);
          if (!widget) continue;
          const target = this.targetOf(widget, hour);
          if (target < actual) actual = target;
          const at = widget.subs.indexOf(subId);
          if (at < 0) continue;
          if (widget.installed?.pictures[at]) keep.push(widget.installed.pictures[at]!);
          if (widget.inflight?.pictures[at]) keep.push(widget.inflight.pictures[at]!);
        }
      }
      if (!Number.isFinite(actual)) actual = hour + (sub.route === null ? 0 : this.offsets.get(sub.route) ?? 0);
      const nextAt = this.facts.prune(subId, actual, keep);
      this.stats.pruned++;
      if (Number.isFinite(nextAt)) {
        const due = nextAt - (actual - hour);
        this.prunableAt.set(subId, due);
        if (due < this.minPruneHour) this.minPruneHour = due;
      } else {
        this.prunableAt.delete(subId);
      }
    }
  }
}
