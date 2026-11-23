import { Facts, changeWindow, followsChannel, joinComponents, samePicture, type Channel, type FollowAnswer, type PanelRecord, type Picture, type Window } from '../dashboard/follow/facts.ts';
import { SAME, type Ack, type Bitmap, type Entry, type FailureOrigin, type FrameRequest, type Refusal } from '../dashboard/follow/page.ts';
import { parseAnswer, parseDatedPanelRecord, parseFollowed, type FollowedAnswer } from '../dashboard/follow/message.ts';
import { mergeChanged } from '../dashboard/follow/tables.ts';
import { sectionOf } from '../dashboard/follow/liveCells.ts';

export interface PanelValue { kind: 'panel'; bytes: Uint8Array | null; data: unknown; refused: string | null }
export interface AnswerValue { kind: 'answer'; body: Uint8Array; answer: FollowedAnswer }
export interface SeedValue { kind: 'seed'; values: readonly (number | string)[] }
export type Value = PanelValue | AnswerValue | SeedValue;

export interface Sub { id: number; channel: Channel; product?: string; route: number | null }

export interface Surface { width: number; height: number; dpr: number }
export interface Binding {
  generation: number; surfaces: readonly Surface[]; offset: number; pin: number | null; settings?: unknown; dated?: { h: number; c: number };
}
export interface Bitmapped { readonly width: number; readonly height: number }

export interface Scene { widget: number; subs: readonly Sub[]; pictures: readonly Picture<Value>[]; model: unknown }

export interface PaintInput extends Scene {
  hour: number;
  speed: number;
  binding: Binding;
  inspect: string | null;
  now: number;
}

export interface Painted {
  surfaces: readonly ({ source: Bitmapped; content: string } | null)[];
  fields: Readonly<Record<string, { value: unknown; content: string }>>;
  motion: boolean;
  mode?: 'picture' | 'refused';
  reason?: string;
  nextChangeAt?: number;
  hit?: (point: readonly [number, number]) => string | null;
}

export interface LaneView {
  next?(scene: Scene, hour: number, state?: Map<string, unknown>): number;
  paint(input: PaintInput, state: Map<string, unknown>, content: () => string): Painted;
  unmount?(widget: number, state: Map<string, unknown>): void;
}

export interface Presented<B extends Bitmap> { lane: number; serial: number; resets: [number, number][]; entries: Entry<B>[] }

export interface LaneHost<B extends Bitmap> {
  lane: number;
  lanes: number;
  views(kind: string): LaneView;
  crop(sources: readonly Bitmapped[]): Promise<B[]>;
  present(presentation: Presented<B>, transfer: B[]): void;
  release(bytes: number): void;
  fail(reason: string): void;
  later(run: () => void): void;
  hit?(widget: number, key: string | null): void;
  served?(frame: { seek: number; target: number; exact: boolean; changed: number; ms: number }): void;
  budget?: number;
  clock?: () => number;
  stretch?: (ms: number) => void;
}

interface Sent {
  id: number; request: number; pictures: Picture<Value>[]; h: number; c: number; lo: number; hi: number; binding: number;
  contents: Record<string, string>; inspect: string | null; hit?: Painted['hit'];
}

interface Widget {
  id: number; order: number; subs: number[]; view: string; model: unknown; binding: Binding;
  installed: Sent | null; inflight: Sent | null; sent: Record<string, string>; motion: boolean; inspect: string | null;
  start: { version: string; hour: number } | null; state: Map<string, unknown>; cost: number; served: number; halted: number | null; parked: Sent | null;
}

interface Held { kind: 'follow' | 'panels'; native: number; seek: number; hour: number; id: number; records: Uint8Array[]; size: number; completed?: number }

const charged = (record: Uint8Array) => {
  const view = new DataView(record.buffer, record.byteOffset, record.byteLength), length = view.getUint32(8, true);
  return 16 + 2 * Math.max(length, view.getUint32(12, true));
};

const recordsCharge = (bytes: Uint8Array) => {
  const data = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let total = 0;
  for (let i = 0, at = 24, count = data.getUint32(12, true); i < count; i++) { const length = data.getUint32(at + 8, true); total += charged(bytes.subarray(at, at + 16 + length)); at += 16 + length; }
  return total;
};

const refusalOf = (picture: Picture<Value>): string | null => {
  const value = picture.parts[0]!.value;
  if (value.kind === 'answer') return value.answer.kind === 'notRecorded' ? value.answer.reason : null;
  return value.kind === 'panel' ? value.refused : null;
};

const BUFFER = 8 << 20;
const HELD_COST = 32;
const STALE = 120;
const CELL = 0x8000_0000;
const LOST = 'Answers that arrived before this reading was registered overflowed the early buffer and were lost';

type Selected = { pictures: Picture<Value>[]; window: Window; h: number; c: number };

const now = () => performance.now();


function cellsOf(data: unknown): { id: number; seedHour?: number; values?: Value }[] {
  const section = sectionOf(data);
  if (!section) return [];
  return section.groups.flatMap(group => group.follow === undefined ? []
    : [{ id: group.follow, ...group.seedHour !== undefined && group.values ? { seedHour: group.seedHour, values: { kind: 'seed' as const, values: group.values } } : {} }]);
}

export class Lane<B extends { readonly width: number; close(): void }> {
  readonly facts = new Facts<Value>();
  readonly stats = { frames: 0, entries: 0, motion: 0, crops: 0, refused: {} as Record<string, number> };
  private readonly subs = new Map<number, Sub>();
  private readonly widgets = new Map<number, Widget>();
  private readonly routes = new Map<number, number>();
  private readonly natives = new Map<number, number>();
  private readonly offsets = new Map<number, number>();
  private readonly resets = new Map<number, number>();
  private readonly unbound = new Map<number, Map<number, number>>();
  private readonly stamped = new Map<number, number>();
  private readonly applied = new Map<number, number>();
  private readonly told = new Map<number, number>();
  private held: Held[] = [];
  private early: { native: number; seek: number; bytes: Uint8Array; stamp?: number }[] = [];
  private heldBytes = 0;
  private readonly heldIds = new Map<number, number>();
  private readonly lost = new Map<number, number>();
  private readonly gone = new Set<number>();
  private readonly failed = new Set<number>();
  private readonly presented = new Map<number, { widget: number; sent: Sent }>();
  private request: FrameRequest | null = null;
  private lastExact: FrameRequest | null = null;
  private working = false;
  private pacing = false;
  private requests = 0;
  private nextEntry = 1;
  private nextContent = 1;
  private nextOrder = 1;
  private freed = 0;
  private readonly host: LaneHost<B>;

  constructor(host: LaneHost<B>) { this.host = host; }

  private time() { return this.host.clock?.() ?? now(); }
  private content = () => `${this.host.lane}:${this.nextContent++}`;

  follow(sub: number, channel: Channel, product?: string) {
    this.subs.set(sub, { id: sub, channel, product, route: null });
    if (channel === 'pinned') this.facts.follow(sub, -1, 'pinned');
  }

  bindRoute(sub: number, route: number, native: number, seek = 0, offset = 0) {
    const held = this.subs.get(sub);
    if (!held || held.route !== null) return;
    held.route = route;
    this.routes.set(native, route);
    this.natives.set(route, native);
    this.offsets.set(route, offset);
    this.facts.follow(sub, route, held.channel);
    this.facts.stamp(route, seek);
    this.settleResets();
    this.replay();
    this.replayEarly();
    this.after();
  }

  unfollow(sub: number) {
    this.subs.delete(sub);
    this.lost.delete(sub);
    this.forget(sub);
    for (const cell of this.facts.unfollow(sub)) { this.gone.add(cell); this.forget(cell); }
    this.after();
  }

  mount(widget: number, subs: readonly number[], view: string, model?: unknown) {
    const was = this.widgets.get(widget);
    this.widgets.set(widget, { id: widget, order: was?.order ?? this.nextOrder++, subs: [...subs], view, model,
      binding: was?.binding ?? { generation: 1, surfaces: [], offset: 0, pin: null },
      installed: was?.installed ?? null, inflight: null, sent: {}, motion: false, inspect: was?.inspect ?? null, start: null, state: was?.state ?? new Map(), cost: was?.cost ?? 0.1,
      served: was?.served ?? 0, halted: null, parked: null });
    this.wake();
  }

  unmount(widget: number) {
    const held = this.widgets.get(widget);
    if (held) {
      this.host.views(held.view).unmount?.(widget, held.state);
      this.widgets.delete(widget);
    }
    for (const [id, owner] of this.presented) if (owner.widget === widget) this.presented.delete(id);
  }

  bind(widget: number, binding: Binding, model?: unknown) {
    const held = this.widgets.get(widget);
    if (!held) return;
    held.binding = binding;
    if (model !== undefined) held.model = model;
    held.sent = {};
    held.inflight = null;
    if (held.halted !== null && held.halted !== binding.generation) held.halted = null;
    this.wake();
  }

  touch(widget: number) {
    const held = this.widgets.get(widget);
    if (!held) return;
    held.motion = true;
    this.wake();
  }

  pointer(widget: number, _surface: number, point: [number, number] | null) {
    const held = this.widgets.get(widget);
    if (!held) return;
    const hit = held.installed?.hit;
    this.host.hit?.(widget, point && hit ? hit(point) : null);
  }

  exact(sub: number, hour: number, data: unknown, refused: string | null) {
    this.facts.exact(sub, hour, { kind: 'panel', bytes: null, data, refused }, refused);
    this.wake();
  }

  stream(route: number, seek: number, target: number) {
    this.failed.delete(route);
    const native = this.natives.get(route);
    if (native !== undefined) for (const [id, from] of this.lost) if (from === native) this.lost.delete(id);
    this.facts.seek(route, seek, target + (this.offsets.get(route) ?? 0));
    for (const widget of this.widgets.values()) if (this.routeOf(widget) === route) widget.inflight = null;
    this.settleResets();
    this.replay();
    this.replayEarly();
    this.after();
  }

  fail(route: number) {
    this.failed.add(route);
    for (const widget of this.widgets.values()) if (this.routeOf(widget) === route) widget.inflight = null;
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
    const shown = new Set<unknown>();
    const sents = [...[...this.widgets.values()].flatMap(widget => [widget.installed, widget.inflight]), ...[...this.presented.values()].map(owner => owner.sent)];
    for (const sent of sents) for (const picture of sent?.pictures ?? []) for (const part of picture.parts) shown.add(part);
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
    if (item.id >= CELL && this.gone.has(item.id) || this.lost.has(item.id)) { this.freed += item.size; return; }
    if (this.heldBytes + item.size + HELD_COST > BUFFER) {
      this.freed += item.size;
      this.forget(item.id);
      this.lost.set(item.id, item.native);
      if (item.id >= CELL) this.host.fail(LOST);
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
    if (route === undefined) return null;
    const seek = this.facts.generation(route).seek;
    return item.seek < seek ? -1 : item.seek > seek ? 1 : 0;
  }

  private replayEarly() {
    const due = this.early.filter(item => (this.current(item) ?? 1) <= 0);
    if (!due.length) return;
    this.early = this.early.filter(item => !due.includes(item));
    for (const item of due) this.follows(item.bytes, item.stamp);
  }

  private replay() {
    const stale = this.held.filter(item => item.kind === 'follow' && this.current(item) === -1);
    this.release(stale);
    this.held = this.held.filter(item => !stale.includes(item));
    const ready = this.held.filter(item => this.known(item.id) && this.routes.has(item.native) && (item.kind === 'panels' || this.current(item) === 0));
    if (!ready.length) { this.reindex(); return; }
    this.held = this.held.filter(item => !ready.includes(item));
    for (const item of ready) this.heldBytes -= item.size + HELD_COST;
    this.reindex();
    for (const item of ready) {
      const route = this.routes.get(item.native)!, only = new Set([item.id]);
      if (item.kind === 'follow') {
        const limited = new Map<number, number>(), answers: FollowAnswer<Value>[] = [], latest = new Map<number, AnswerValue>();
        for (const record of item.records) {
          const view = new DataView(record.buffer, record.byteOffset, record.byteLength);
          if (!view.getUint32(8, true)) { limited.set(item.id, view.getInt32(4, true)); this.freed += charged(record); }
          else { const answer = this.answer(item.id, record, charged(record), false, latest); if (answer) answers.push(answer); }
        }
        this.facts.followed(route, item.seek, { hour: item.hour, answers, limited }, only);
      } else this.facts.panels(route, item.seek, item.records.flatMap(record => this.panel(record) ?? []), item.completed, only);
    }
    this.replay();
  }

  private answer(id: number, record: Uint8Array, bytes: number, reset: boolean, latest: Map<number, AnswerValue>): FollowAnswer<Value> | null {
    const view = new DataView(record.buffer, record.byteOffset, record.byteLength), at = view.getInt32(4, true);
    let value: AnswerValue;
    try {
      const parsed = parseAnswer(record.subarray(16, 16 + view.getUint32(8, true)));
      if (parsed.kind === 'changed') {
        const base = latest.get(id) ?? (reset ? undefined : this.facts.latest(id));
        if (!base || base.kind !== 'answer') throw new Error('Changed rows arrived without the table they change.');
        const merged = mergeChanged(base.answer, parsed, view.getUint32(12, true));
        value = { kind: 'answer', body: merged, answer: parseAnswer(merged) };
      } else {
        const body = record.slice(16);
        value = { kind: 'answer', body, answer: parseAnswer(body) };
      }
    } catch (error) {
      this.freed += bytes;
      this.host.fail(error instanceof Error ? error.message : String(error));
      return null;
    }
    latest.set(id, value);
    return { id, hour: at, value, bytes };
  }

  private follows(bytes: Uint8Array, stamped?: number) {
    let message: ReturnType<typeof parseFollowed>;
    try { message = parseFollowed(bytes); }
    catch (error) { this.freed += recordsCharge(bytes); throw error; }
    if (stamped !== undefined) this.stamped.set(message.route, Math.max(this.stamped.get(message.route) ?? 0, stamped));
    const order = this.current({ native: message.route, seek: message.seek });
    const data = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const count = data.getUint32(12, true);
    if (order === -1) { this.freed += recordsCharge(bytes); return; }
    if (order === 1) { this.early.push({ native: message.route, seek: message.seek, bytes, stamp: stamped }); return; }
    if (message.reset && order === null) this.unboundReset(message.route, message.seek);
    else if (message.reset) {
      this.resets.set(message.route, (this.resets.get(message.route) ?? 0) + 1);
      this.drop(message.route);
    }
    const stamp = this.stamped.get(message.route);
    if (stamp !== undefined) this.resets.set(message.route, stamp);
    const route = this.routes.get(message.route);
    const limited = new Map<number, number>(), unbound = new Map<number, { records: Uint8Array[]; size: number }>();
    const mine: [number, number, number][] = [];
    let at = 24;
    for (let i = 0; i < count; i++) {
      const id = data.getUint32(at, true), hour = data.getInt32(at + 4, true), length = data.getUint32(at + 8, true), record = bytes.subarray(at, at + 16 + length);
      const size = charged(record);
      if (route === undefined || !this.known(id)) {
        if (this.subs.has(id) || id >= CELL) {
          let own = unbound.get(id);
          if (!own) unbound.set(id, own = { records: [], size: 0 });
          own.records.push(record.slice()); own.size += size;
        } else this.freed += size;
      } else if (!length) { limited.set(id, hour); this.freed += size; }
      else mine.push([id, at, length]);
      at += 16 + length;
    }
    this.holdMessage('follow', message.route, message.seek, message.hour, unbound);
    if (route === undefined) return;
    const behind = stamped !== undefined && (this.applied.get(message.route) ?? stamped) < stamped, reset = message.reset || behind;
    if (stamped !== undefined) this.applied.set(message.route, stamped);
    const latest = new Map<number, AnswerValue>();
    const answers = mine.flatMap(([id, start, length]) => { const record = bytes.subarray(start, start + 16 + length); return this.answer(id, record, charged(record), reset, latest) ?? []; });
    this.facts.followed(route, message.seek, { hour: message.hour, reset, answers, limited });
  }

  private panel(record: Uint8Array): PanelRecord<Value> | null {
    const size = 2 * record.length;
    let panel: ReturnType<typeof parseDatedPanelRecord>;
    try { panel = parseDatedPanelRecord(record); }
    catch (error) { this.freed += size; this.host.fail(error instanceof Error ? error.message : String(error)); return null; }
    if (panel.waiting) { this.freed += size; return { kind: 'waiting', id: panel.id, hour: panel.hour }; }
    if (!panel.panel) { this.freed += size; return { kind: 'stands', id: panel.id, hour: panel.hour, stepped: panel.stepped }; }
    if (panel.panel.refused !== null) return { kind: 'refused', id: panel.id, hour: panel.hour, reason: panel.panel.refused, stepped: panel.stepped, bytes: size,
      value: { kind: 'panel', bytes: null, data: undefined, refused: panel.panel.refused } };
    let cells: ReturnType<typeof cellsOf> = [];
    try { cells = cellsOf(panel.panel.data); } catch { cells = []; }
    return { kind: 'data', id: panel.id, hour: panel.hour, stepped: panel.stepped, bytes: size, cells,
      value: { kind: 'panel', bytes: panel.panel.bytes ? panel.panel.bytes.slice() : null, data: panel.panel.data, refused: null } };
  }

  private panels(bytes: Uint8Array) {
    const data = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const native = data.getUint32(16, true), seek = data.getUint32(20, true), count = data.getUint32(8, true);
    const route = this.routes.get(native);
    let at = 24, completed: number | undefined;
    const mine: PanelRecord<Value>[] = [], unbound = new Map<number, { records: Uint8Array[]; size: number }>();
    for (let i = 0; i < count; i++) {
      const id = data.getUint32(at, true), kind = data.getUint8(at + 8), length = data.getUint32(at + 12, true);
      const size = ((16 + length + 7) & ~7);
      if (kind === 4) completed = data.getInt32(at + 4, true);
      else if (route === undefined || !this.known(id)) {
        if (this.subs.has(id)) {
          let own = unbound.get(id);
          if (!own) unbound.set(id, own = { records: [], size: 0 });
          own.records.push(bytes.slice(at, at + size)); own.size += 2 * size;
        } else this.freed += 2 * size;
      } else { const made = this.panel(bytes.subarray(at, at + size)); if (made) mine.push(made); }
      at += size;
    }
    this.holdMessage('panels', native, seek, completed ?? data.getInt32(12, true), unbound, completed);
    if (route === undefined) return;
    this.facts.panels(route, seek, mine, completed);
  }

  frame(request: FrameRequest) {
    this.request = this.request
      ? { ...request, installed: [...this.request.installed, ...request.installed], refused: [...this.request.refused, ...request.refused] }
      : request;
    this.lastExact = request.exact ? { ...request, installed: [], refused: [] } : null;
    this.schedule();
  }

  private wake() {
    if (!this.request && this.lastExact) this.request = { ...this.lastExact, installed: [], refused: [] };
    this.schedule();
  }

  private schedule() {
    if (this.working || !this.request) return;
    this.working = true;
    queueMicrotask(() => { void this.run(); });
  }

  private async run() {
    const request = this.request;
    this.request = null;
    try { if (request) await this.serve(request); }
    catch (error) { this.host.fail(error instanceof Error ? error.message : String(error)); }
    finally {
      this.working = false;
      if (this.request) this.schedule();
      else if (!this.pacing && [...this.widgets.values()].some(widget => widget.motion)) {
        this.pacing = true;
        this.host.later(() => { this.pacing = false; this.wake(); });
      }
    }
  }

  private acknowledge(installed: readonly Ack[], refused: readonly Refusal[]) {
    for (const ack of installed) {
      const owner = this.presented.get(ack.id);
      this.presented.delete(ack.id);
      const widget = owner === undefined ? undefined : this.widgets.get(owner.widget);
      if (!widget || !owner || owner.sent.binding !== widget.binding.generation) continue;
      if (!widget.installed || widget.installed.id < owner.sent.id) widget.installed = owner.sent;
      if (widget.inflight?.id === ack.id) widget.inflight = null;
    }
    for (const refusal of refused) {
      const owner = this.presented.get(refusal.id);
      this.presented.delete(refusal.id);
      this.stats.refused[refusal.reason] = (this.stats.refused[refusal.reason] ?? 0) + 1;
      const widget = this.widgets.get(refusal.widget);
      if (!widget) continue;
      if (widget.inflight?.id === refusal.id) widget.inflight = null;
      if (refusal.reason === 'failed') widget.halted = widget.binding.generation;
      if (refusal.reason === 'kept content') widget.installed = null;
      widget.parked = refusal.reason === 'uncovered' && owner ? owner.sent : null;
      widget.sent = { ...widget.installed?.contents ?? {} };
    }
  }

  private targetOf(widget: Widget, hour: number) { return widget.binding.pin ?? hour + widget.binding.offset; }

  private select(widget: Widget, target: number): Selected | 'lost' | null {
    const pictures: Picture<Value>[] = [];
    let joined: { h: number; c: number; window: Window } | null;
    if (!widget.subs.length) {
      const dated = widget.binding.dated;
      if (!dated || target < dated.h) return null;
      joined = { h: dated.h, c: dated.c, window: [dated.h, Infinity] };
    } else {
      const components: { picture: Picture<Value>; window: Window }[] = [];
      for (const id of widget.subs) {
        if (this.lost.has(id)) return 'lost';
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
    }
    if (!joined) return null;
    const view = this.host.views(widget.view);
    if (!view.next) return { pictures, ...joined };
    const scene: Scene = { widget: widget.id, subs: widget.subs.map(id => this.subs.get(id)!), pictures, model: widget.model };
    const version = widget.subs.length ? pictures.map(picture => picture.parts.map(part => part.version).join('.')).join('|') : `bound:${widget.binding.generation}`;
    const at = Math.max(joined.window[0], Math.min(target, joined.c));
    let window = changeWindow(joined.window, at, hour => view.next!(scene, hour, widget.state), widget.start?.version === version ? widget.start.hour : undefined);
    if (target > window[1]) window = [window[0], joined.window[1]];
    widget.start = { version, hour: window[0] };
    return { pictures, window, h: joined.h, c: joined.c };
  }

  private matches(sent: Sent | null, selected: Selected, target: number, binding: number, inspect: string | null) {
    return !!sent && sent.binding === binding && sent.c === selected.c && sent.inspect === inspect && sent.pictures.length === selected.pictures.length
      && sent.pictures.every((picture, at) => samePicture(picture, selected.pictures[at])) && sent.lo === selected.window[0] && sent.lo <= target && target <= sent.hi;
  }

  private async serve(request: FrameRequest) {
    const began = this.time();
    this.requests++;
    this.stats.frames++;
    this.acknowledge(request.installed, request.refused);
    const inspect = new Map(request.inspect);
    const due: { widget: Widget; selected: Selected | 'lost'; motion: boolean; target: number }[] = [];
    for (const widget of this.widgets.values()) {
      if (widget.halted === widget.binding.generation) continue;
      if (!request.exact) widget.parked = null;
      const key = inspect.get(widget.id) ?? null, target = this.targetOf(widget, request.target);
      const selected = this.select(widget, target);
      if (!selected) continue;
      if (selected === 'lost') {
        if (widget.inflight || widget.installed?.contents.failed === LOST) continue;
        due.push({ widget, selected, motion: false, target });
        continue;
      }
      const shown = this.matches(widget.installed, selected, target, widget.binding.generation, key);
      if (this.matches(widget.parked, selected, target, widget.binding.generation, key)) continue;
      const pending = widget.inflight && this.requests - widget.inflight.request <= STALE && this.matches(widget.inflight, selected, target, widget.binding.generation, key);
      if (pending || shown && !widget.motion) continue;
      due.push({ widget, selected, motion: shown, target });
      widget.inspect = key;
    }
    due.sort((a, b) => Number(a.motion) - Number(b.motion) || a.widget.served - b.widget.served || a.widget.order - b.widget.order);
    const budget = this.host.budget ?? 10;
    const entries: Entry<B>[] = [], surfaces: (B | typeof SAME | null)[][] = [], sources: Bitmapped[] = [], slots: [number, number, number][] = [];
    const made: { widget: Widget; selected: Selected }[] = [];
    const shared = new Map<string, number>();
    let tried = 0;
    for (const { widget, selected, motion, target } of due) {
      if (tried++ && this.time() - began + widget.cost > budget) break;
      const start = this.time();
      widget.served = this.requests;
      if (selected === 'lost') {
        const lost = { pictures: [], window: [-Infinity, Infinity] as Window, h: target, c: target };
        entries.push(this.entry(widget, lost, { surfaces: [], fields: {}, motion: false }, 'failed', LOST, [], request.seek, 0, 'stream'));
        made.push({ widget, selected: lost });
        surfaces.push([]);
        continue;
      }
      const view = this.host.views(widget.view);
      let activeSelected = selected;
      let painted: Painted = { surfaces: [], fields: {}, motion: false };
      let mode: Entry['mode'] = 'picture', reason: string | undefined, failureOrigin: FailureOrigin | undefined;
      for (let pass = 0; pass < 4; pass++) {
        try {
          painted = view.paint({ widget: widget.id, subs: widget.subs.map(id => this.subs.get(id)!), pictures: activeSelected.pictures, model: widget.model,
            hour: activeSelected.window[0], speed: request.speed, binding: widget.binding, inspect: widget.inspect, now: start }, widget.state, this.content);
          mode = painted.mode ?? 'picture'; reason = painted.reason; failureOrigin = undefined;
          const refusals = activeSelected.pictures.map(refusalOf);
          if (painted.mode === undefined && refusals.length && refusals.every(refusal => refusal !== null)) { mode = 'refused'; reason = [...new Set(refusals)].join('; '); }
          else if (mode === 'picture' && refusals.some(refusal => refusal !== null))
            painted = { ...painted, fields: { ...painted.fields, ...Object.fromEntries(refusals.flatMap((refusal, at) => refusal === null ? [] : [[`refused:${widget.subs[at]}`, { content: refusal, value: refusal }]])) } };
        } catch (error) {
          painted = { surfaces: [], fields: {}, motion: false };
          mode = 'failed'; reason = 'drawing failed: ' + (error instanceof Error ? error.message : String(error));
          failureOrigin = 'local';
          break;
        }
        if (painted.nextChangeAt !== undefined && painted.nextChangeAt > activeSelected.window[0]) {
          const clampedHi = Math.min(activeSelected.window[1], painted.nextChangeAt - 1);
          if (clampedHi < activeSelected.window[1]) {
            activeSelected = { ...activeSelected, window: [activeSelected.window[0], clampedHi] };
          }
          if (target > activeSelected.window[1] && pass < 3) {
            const nextSelected = this.select(widget, target);
            if (nextSelected && nextSelected !== 'lost' && nextSelected.window[0] > activeSelected.window[0]) {
              activeSelected = nextSelected;
              continue;
            }
          }
        }
        break;
      }
      widget.motion = painted.motion;
      widget.cost = widget.cost * 0.8 + (this.time() - start) * 0.2;
      if (motion) this.stats.motion++;
      const parts: (B | typeof SAME | null)[] = [];
      const entry = this.entry(widget, activeSelected, painted, mode, reason, parts, request.seek, this.time() - start, failureOrigin);
      painted.surfaces.forEach((surface, k) => {
        if (!surface || parts[k] === SAME) return;
        let source = shared.get(surface.content);
        if (source === undefined) { source = sources.push(surface.source) - 1; shared.set(surface.content, source); }
        slots.push([entries.length, k, source]);
      });
      entries.push(entry);
      made.push({ widget, selected: activeSelected });
      surfaces.push(parts);
    }
    let crops: B[] = [];
    try { crops = slots.length ? await this.host.crop(slots.map(([, , source]) => sources[source]!)) : []; }
    catch (error) {
      const reason = 'drawing failed: ' + (error instanceof Error ? error.message : String(error));
      for (const e of new Set(slots.map(([e]) => e))) {
        const { widget, selected } = made[e]!;
        this.presented.delete(entries[e]!.id);
        widget.sent = { ...widget.installed?.contents ?? {} };
        entries[e] = this.entry(widget, selected, { surfaces: [], fields: {}, motion: false }, 'failed', reason, [], request.seek, 0, 'local');
        surfaces[e] = [];
      }
      slots.length = 0;
    }
    this.stats.crops += crops.length;
    slots.forEach(([e, k], at) => { surfaces[e]![k] = crops[at]!; });
    const resets: [number, number][] = [];
    let moved = false;
    for (const route of new Set([...this.subs.values()].flatMap(sub => sub.route === null ? [] : [sub.route]))) {
      const count = this.resets.get(this.natives.get(route)!) ?? 0;
      if (this.told.get(route) !== count) { moved = true; this.told.set(route, count); }
      resets.push([route, count]);
    }
    if (entries.length || moved) this.host.present({ lane: this.host.lane, serial: request.serial, resets, entries }, crops);
    this.stats.entries += entries.length;
    this.prune(request.now);
    const bytes = this.freed + this.releasedFacts();
    this.freed = 0;
    if (bytes) this.host.release(bytes);
    this.host.stretch?.(this.time() - began);
    this.host.served?.({ seek: request.seek, target: request.target, exact: request.exact, changed: entries.length, ms: this.time() - began });
  }

  private entry(widget: Widget, selected: Selected, painted: Painted, mode: Entry['mode'], reason: string | undefined, parts: (B | typeof SAME | null)[], seek: number, ms = 0, failureOrigin?: FailureOrigin): Entry<B> {
    const id = this.host.lane + 1 + this.host.lanes * this.nextEntry++, contents: Record<string, string> = { hour: String(selected.window[0]) };
    if (mode === 'failed' && reason !== undefined) contents.failed = reason;
    const fields: Record<string, unknown> = {};
    painted.surfaces.forEach((surface, k) => {
      if (!surface) { parts[k] = null; return; }
      contents[k] = surface.content;
      parts[k] = widget.sent[k] === surface.content ? SAME : null;
    });
    for (const [key, field] of Object.entries(painted.fields)) {
      contents[key] = field.content;
      fields[key] = widget.sent[key] === field.content ? SAME : field.value;
    }
    const route = this.routeOf(widget);
    const gen = route === null ? { seek, reset: 0 } : { seek: this.facts.generation(route).seek, reset: this.applied.get(this.natives.get(route)!) ?? this.resets.get(this.natives.get(route)!) ?? 0 };
    const sent: Sent = { id, request: this.requests, pictures: selected.pictures, h: selected.h, c: selected.c, lo: selected.window[0], hi: selected.window[1],
      binding: widget.binding.generation, contents, inspect: widget.inspect, hit: painted.hit };
    widget.sent = { ...contents };
    widget.inflight = sent;
    this.presented.set(id, { widget: widget.id, sent });
    return Object.freeze({ widget: widget.id, binding: widget.binding.generation, id, h: selected.h, c: selected.c, lo: selected.window[0], hi: selected.window[1],
      gen: Object.freeze(gen), follows: selected.pictures.some(picture => followsChannel(picture)), mode, ...reason !== undefined ? { reason } : {},
      ...failureOrigin !== undefined ? { failureOrigin } : {},
      surfaces: parts, content: Object.freeze(contents), fields: Object.freeze(fields), ms });
  }

  private prune(hour: number) {
    const keep = new Map<number, Picture<Value>[]>(), actual = new Map<number, number>();
    for (const widget of this.widgets.values()) {
      const target = this.targetOf(widget, hour);
      widget.subs.forEach((sub, at) => {
        actual.set(sub, Math.min(actual.get(sub) ?? Infinity, target));
        let list = keep.get(sub);
        if (!list) keep.set(sub, list = []);
        for (const sent of [widget.installed, widget.inflight]) if (sent?.pictures[at]) list.push(sent.pictures[at]!);
      });
    }
    for (const sub of this.subs.values())
      this.facts.prune(sub.id, actual.get(sub.id) ?? hour + (sub.route === null ? 0 : this.offsets.get(sub.route) ?? 0), keep.get(sub.id) ?? []);
  }
}
