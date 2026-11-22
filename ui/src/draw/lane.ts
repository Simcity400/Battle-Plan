import { Facts, changeWindow, followsChannel, joinComponents, samePicture, type Channel, type FollowAnswer, type PanelRecord, type Picture, type Window } from '../dashboard/follow/facts.ts';
import { SAME, type Ack, type Bitmap, type Entry, type FrameRequest, type Refusal } from '../dashboard/follow/page.ts';
import { parseAnswer, parseDatedPanels, parseFollowed, type FollowedAnswer } from '../dashboard/follow/message.ts';
import { mergeChanged } from '../dashboard/follow/tables.ts';
import { sectionOf } from '../dashboard/follow/liveCells.ts';

export interface PanelValue { kind: 'panel'; bytes: Uint8Array | null; data: unknown; refused: string | null }
export interface AnswerValue { kind: 'answer'; body: Uint8Array; answer: FollowedAnswer }
export interface SeedValue { kind: 'seed'; values: readonly (number | string)[] }
export type Value = PanelValue | AnswerValue | SeedValue;

export interface Sub { id: number; channel: Channel; product?: string; route: number | null; rank: number }

export interface Surface { width: number; height: number; dpr: number }
export interface Binding { generation: number; surfaces: readonly Surface[]; settings?: unknown; dated?: { h: number; c: number } }
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
}

export interface LaneView {
  next?(scene: Scene, hour: number): number;
  paint(input: PaintInput, state: Map<string, unknown>, content: () => string): Painted;
}

export interface Painted {
  hit?: string | null;
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
  budget?: number;
  clock?: () => number;
  stretch?: (ms: number) => void;
}

interface Sent {
  id: number; request: number; pictures: Picture<Value>[]; h: number; c: number; binding: number;
  contents: Record<string, string>; inspect: string | null;
}

interface Widget {
  id: number; order: number; subs: number[]; view: string; model: unknown; binding: Binding;
  installed: Sent | null; inflight: Sent | null; sent: Record<string, string>; motion: boolean; inspect: string | null;
  start: { version: string; hour: number } | null; state: Map<string, unknown>; cost: number;
}

interface Held { kind: 'follow' | 'panels'; native: number; seek: number; hour: number; id: number; record: Uint8Array; size: number; completed?: number }

const BUFFER = 1 << 20;
const STALE = 120;
const HELD_PER_ID = 8;
const CELL = 0x8000_0000;

type Selected = { pictures: Picture<Value>[]; window: Window; h: number; c: number };

const now = () => performance.now();

const answerWindow = (answer: FollowedAnswer) => answer.kind === 'notRecorded' ? answer.window ?? 0 : answer.window;

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
  private held: Held[] = [];
  private heldBytes = 0;
  private readonly gone = new Set<number>();
  private readonly failed = new Set<number>();
  private readonly presented = new Map<number, number>();
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
    this.subs.set(sub, { id: sub, channel, product, route: null, rank: 0 });
    if (channel === 'pinned') this.facts.follow(sub, -1, 'pinned');
  }

  bindRoute(sub: number, route: number, native: number) {
    const held = this.subs.get(sub);
    if (!held || held.route !== null) return;
    held.route = route;
    this.routes.set(native, route);
    this.natives.set(route, native);
    this.facts.follow(sub, route, held.channel);
    this.replay();
    this.after();
  }

  unfollow(sub: number) {
    this.subs.delete(sub);
    this.facts.unfollow(sub);
    this.after();
  }

  rank(sub: number, version: number) {
    const held = this.subs.get(sub);
    if (!held || held.rank === version) return;
    held.rank = version;
    this.wake();
  }

  mount(widget: number, subs: readonly number[], view: string, model?: unknown) {
    const was = this.widgets.get(widget);
    this.widgets.set(widget, { id: widget, order: was?.order ?? this.nextOrder++, subs: [...subs], view, model, binding: was?.binding ?? { generation: 1, surfaces: [] },
      installed: null, inflight: null, sent: {}, motion: false, inspect: was?.inspect ?? null, start: null, state: was?.state ?? new Map(), cost: was?.cost ?? 0.1 });
    this.wake();
  }

  unmount(widget: number) { this.widgets.delete(widget); }

  bind(widget: number, binding: Binding, model?: unknown) {
    const held = this.widgets.get(widget);
    if (!held) return;
    held.binding = binding;
    if (model !== undefined) held.model = model;
    held.sent = {};
    held.inflight = null;
    this.wake();
  }

  touch(widget: number) {
    const held = this.widgets.get(widget);
    if (!held) return;
    held.motion = true;
    this.wake();
  }

  pointer(widget: number, surface: number, point: [number, number] | null) {
    const held = this.widgets.get(widget);
    if (!held) return;
    held.state.set('pointer', point && { surface, point });
    held.motion = true;
    this.wake();
  }

  exact(sub: number, hour: number, data: unknown, refused: string | null) {
    this.facts.exact(sub, hour, { kind: 'panel', bytes: null, data, refused }, refused);
    this.wake();
  }

  stream(route: number, seek: number, target: number) {
    this.failed.delete(route);
    this.facts.seek(route, seek, target);
    for (const widget of this.widgets.values()) if (this.routeOf(widget) === route) widget.inflight = null;
    this.after();
  }

  fail(route: number) {
    this.failed.add(route);
    for (const widget of this.widgets.values()) if (this.routeOf(widget) === route) widget.inflight = null;
  }

  receive(kind: 'follow' | 'panels', bytes: Uint8Array) {
    if (kind === 'follow') this.follows(bytes);
    else this.panels(bytes);
    this.replay();
    this.after();
  }

  private after() {
    for (const id of this.facts.drain()) this.gone.add(id);
    const bytes = this.freed + this.facts.releasedBytes();
    this.freed = 0;
    if (bytes) this.host.release(bytes);
    this.wake();
  }

  private routeOf(widget: Widget) {
    for (const sub of widget.subs) { const route = this.subs.get(sub)?.route; if (route !== null && route !== undefined) return route; }
    return null;
  }

  private known(id: number) {
    return id >= CELL ? this.facts.owns(id) : this.subs.get(id)?.route !== null && this.subs.has(id);
  }

  private hold(item: Held) {
    if (item.id >= CELL && this.gone.has(item.id)) { this.freed += item.size; return; }
    const same = this.held.filter(held => held.id === item.id);
    if (same.length >= HELD_PER_ID) {
      const oldest = same[0]!;
      this.held = this.held.filter(held => held !== oldest);
      this.heldBytes -= oldest.size; this.freed += oldest.size;
    }
    if (this.heldBytes + item.size > BUFFER) { this.freed += item.size; this.host.fail('registration lost its seed'); return; }
    this.heldBytes += item.size;
    this.held.push(item);
  }

  private drop(native: number) {
    this.held = this.held.filter(item => {
      if (item.native !== native || item.kind !== 'follow') return true;
      this.heldBytes -= item.size; this.freed += item.size;
      return false;
    });
  }

  private replay() {
    let changed = true;
    while (changed) {
      changed = false;
      const ready = this.held.filter(item => this.known(item.id) && this.routes.has(item.native));
      if (!ready.length) return;
      this.held = this.held.filter(item => !ready.includes(item));
      for (const item of ready) {
        this.heldBytes -= item.size;
        const route = this.routes.get(item.native)!;
        if (item.kind === 'follow') this.replayAnswer(route, item);
        else this.panel(route, item.seek, item.record, item.size, item.completed);
        changed = true;
      }
    }
  }

  private answer(id: number, record: Uint8Array, size: number, reset: boolean, latest: Map<number, AnswerValue>): FollowAnswer<Value> | null {
    const at = new DataView(record.buffer, record.byteOffset, record.byteLength).getInt32(4, true);
    const parsed = parseAnswer(record.subarray(16));
    const sub = this.subs.get(id);
    if (sub && sub.rank !== answerWindow(parsed)) { this.freed += size; return null; }
    let value: AnswerValue;
    if (parsed.kind === 'changed') {
      const base = latest.get(id) ?? (reset ? undefined : this.facts.latest(id));
      if (!base || base.kind !== 'answer') { this.freed += size; this.host.fail('Changed rows arrived without the table they change.'); return null; }
      const merged = mergeChanged(base.answer, parsed);
      value = { kind: 'answer', body: merged, answer: parseAnswer(merged) };
    } else {
      const body = record.slice(16);
      value = { kind: 'answer', body, answer: parseAnswer(body) };
    }
    latest.set(id, value);
    return { id, hour: at, value, bytes: size };
  }

  private replayAnswer(route: number, item: Held) {
    const view = new DataView(item.record.buffer, item.record.byteOffset, item.record.byteLength);
    if (!view.getUint32(8, true)) {
      this.freed += item.size;
      this.facts.followed(route, item.seek, { hour: item.hour, answers: [], limited: new Map([[item.id, view.getInt32(4, true)]]) });
      return;
    }
    const answer = this.answer(item.id, item.record, item.size, false, new Map());
    if (answer) this.facts.followed(route, item.seek, { hour: item.hour, answers: [answer] });
  }

  private follows(bytes: Uint8Array) {
    const message = parseFollowed(bytes);
    const route = this.routes.get(message.route);
    if (message.reset) this.drop(message.route);
    const data = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const count = data.getUint32(12, true), limited = new Map<number, number>();
    const mine: [number, number, number][] = [];
    let at = 24;
    for (let i = 0; i < count; i++) {
      const id = data.getUint32(at, true), hour = data.getInt32(at + 4, true), length = data.getUint32(at + 8, true), size = 16 + length;
      if (route === undefined || !this.known(id)) {
        if (this.subs.has(id) || id >= CELL) this.hold({ kind: 'follow', native: message.route, seek: message.seek, hour: message.hour, id, record: bytes.slice(at, at + size), size });
        else this.freed += size;
      } else if (!length) { limited.set(id, hour); this.freed += size; }
      else mine.push([id, at, size]);
      at += size;
    }
    if (route === undefined) return;
    const latest = new Map<number, AnswerValue>();
    const answers = mine.flatMap(([id, start, size]) => this.answer(id, bytes.subarray(start, start + size), size, message.reset, latest) ?? []);
    this.facts.followed(route, message.seek, { hour: message.hour, reset: message.reset, answers, limited });
  }

  private panel(route: number, seek: number, record: Uint8Array, size: number, completed?: number) {
    const wrapped = new Uint8Array(24 + record.length);
    const view = new DataView(wrapped.buffer);
    view.setUint32(0, 0x324c4e50, true); view.setUint16(4, 6, true); view.setUint32(8, record.length ? 1 : 0, true);
    view.setUint32(16, this.natives.get(route) ?? 0, true); view.setUint32(20, seek, true);
    wrapped.set(record, 24);
    const [panel] = parseDatedPanels(wrapped).panels;
    if (!panel) { this.facts.panels(route, seek, [], completed); return; }
    let item: PanelRecord<Value>;
    if (panel.waiting) { this.freed += size; item = { kind: 'waiting', id: panel.id, hour: panel.hour }; }
    else if (!panel.panel) { this.freed += size; item = { kind: 'stands', id: panel.id, hour: panel.hour, stepped: panel.stepped }; }
    else if (panel.panel.refused !== null) item = { kind: 'refused', id: panel.id, hour: panel.hour, reason: panel.panel.refused, stepped: panel.stepped, bytes: size,
      value: { kind: 'panel', bytes: null, data: undefined, refused: panel.panel.refused } };
    else {
      let cells: ReturnType<typeof cellsOf> = [];
      try { cells = cellsOf(panel.panel.data); } catch { cells = []; }
      item = { kind: 'data', id: panel.id, hour: panel.hour, stepped: panel.stepped, bytes: size, cells,
        value: { kind: 'panel', bytes: panel.panel.bytes ? panel.panel.bytes.slice() : null, data: panel.panel.data, refused: null } };
    }
    this.facts.panels(route, seek, [item], completed);
  }

  private panels(bytes: Uint8Array) {
    const data = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    const native = data.getUint32(16, true), seek = data.getUint32(20, true), count = data.getUint32(8, true);
    const route = this.routes.get(native);
    let at = 24, completed: number | undefined;
    const mine: [number, number][] = [];
    for (let i = 0; i < count; i++) {
      const id = data.getUint32(at, true), kind = data.getUint8(at + 8), length = data.getUint32(at + 12, true);
      const size = ((16 + length + 7) & ~7);
      if (kind === 4) completed = data.getInt32(at + 4, true);
      else if (route === undefined || !this.known(id)) {
        if (this.subs.has(id)) this.hold({ kind: 'panels', native, seek, hour: data.getInt32(at + 4, true), id, record: bytes.slice(at, at + size), size });
        else this.freed += size;
      } else mine.push([at, size]);
      at += size;
    }
    if (route === undefined) return;
    if (!mine.length) { this.facts.panels(route, seek, [], completed); return; }
    mine.forEach(([start, size], index) => this.panel(route, seek, bytes.subarray(start, start + size), size, index === mine.length - 1 ? completed : undefined));
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
      const widget = owner === undefined ? undefined : this.widgets.get(owner);
      const sent = widget?.inflight?.id === ack.id ? widget.inflight : null;
      if (!widget || !sent) continue;
      if (!widget.installed || widget.installed.id < sent.id) widget.installed = sent;
      widget.inflight = null;
    }
    for (const refusal of refused) {
      this.presented.delete(refusal.id);
      this.stats.refused[refusal.reason] = (this.stats.refused[refusal.reason] ?? 0) + 1;
      const widget = this.widgets.get(refusal.widget);
      if (!widget) continue;
      if (widget.inflight?.id === refusal.id) widget.inflight = null;
      widget.sent = { ...widget.installed?.contents ?? {} };
    }
  }

  private select(widget: Widget, target: number): Selected | null {
    const pictures: Picture<Value>[] = [], components: { picture: Picture<Value>; window: Window }[] = [];
    if (!widget.subs.length) {
      const dated = widget.binding.dated;
      return dated ? { pictures, window: [-Infinity, Infinity], h: dated.h, c: dated.c } : null;
    }
    for (const id of widget.subs) {
      const sub = this.subs.get(id);
      if (!sub || sub.route !== null && this.failed.has(sub.route)) return null;
      const picture = this.facts.select(id, target);
      if (!picture) return null;
      const value = picture.parts[0]!.value;
      if (value.kind === 'answer' && answerWindow(value.answer) !== sub.rank) return null;
      const window = this.facts.window(id, target, picture);
      if (!window) return null;
      pictures.push(picture);
      components.push({ picture, window });
    }
    const joined = joinComponents(components);
    if (!joined) return null;
    const view = this.host.views(widget.view);
    if (!view.next) return { pictures, ...joined };
    const scene: Scene = { widget: widget.id, subs: widget.subs.map(id => this.subs.get(id)!), pictures, model: widget.model };
    const version = pictures.map(picture => picture.parts.map(part => part.version).join('.')).join('|');
    const window = changeWindow(joined.window, target, hour => view.next!(scene, hour), widget.start?.version === version ? widget.start.hour : undefined);
    widget.start = { version, hour: window[0] };
    return { pictures, window, h: joined.h, c: joined.c };
  }

  private matches(sent: Sent | null, selected: Selected, binding: number, inspect: string | null) {
    return !!sent && sent.binding === binding && sent.c === selected.c && sent.inspect === inspect && sent.pictures.length === selected.pictures.length
      && sent.pictures.every((picture, at) => samePicture(picture, selected.pictures[at])) && sent.contents.hour === String(selected.window[0]);
  }

  private async serve(request: FrameRequest) {
    const began = this.time();
    this.requests++;
    this.stats.frames++;
    this.acknowledge(request.installed, request.refused);
    const inspect = new Map(request.inspect);
    const due: { widget: Widget; selected: Selected; motion: boolean }[] = [];
    for (const widget of this.widgets.values()) {
      const key = inspect.get(widget.id) ?? null;
      const selected = this.select(widget, request.target);
      if (!selected) continue;
      const shown = this.matches(widget.installed, selected, widget.binding.generation, key);
      const pending = widget.inflight && this.requests - widget.inflight.request <= STALE && this.matches(widget.inflight, selected, widget.binding.generation, key);
      if (pending || shown && !widget.motion) continue;
      due.push({ widget, selected, motion: shown });
      widget.inspect = key;
    }
    due.sort((a, b) => Number(a.motion) - Number(b.motion) || (a.widget.installed?.h ?? -Infinity) - (b.widget.installed?.h ?? -Infinity) || a.widget.order - b.widget.order);
    const budget = this.host.budget ?? 10;
    const entries: Entry<B>[] = [], surfaces: (B | typeof SAME | null)[][] = [], sources: Bitmapped[] = [], slots: [number, number, number][] = [];
    const shared = new Map<string, number>();
    for (const { widget, selected, motion } of due) {
      if (entries.length && this.time() - began + widget.cost > budget) break;
      const start = this.time();
      const view = this.host.views(widget.view);
      let painted: Painted, mode: Entry['mode'] = 'picture', reason: string | undefined;
      try {
        painted = view.paint({ widget: widget.id, subs: widget.subs.map(id => this.subs.get(id)!), pictures: selected.pictures, model: widget.model,
          hour: selected.window[0], speed: request.speed, binding: widget.binding, inspect: widget.inspect, now: start }, widget.state, this.content);
        mode = painted.mode ?? 'picture'; reason = painted.reason;
      } catch (error) {
        painted = { surfaces: [], fields: {}, motion: false };
        mode = 'failed'; reason = 'drawing failed: ' + (error instanceof Error ? error.message : String(error));
      }
      widget.motion = painted.motion;
      if (painted.hit !== undefined) this.host.hit?.(widget.id, painted.hit);
      widget.cost = widget.cost * 0.8 + (this.time() - start) * 0.2;
      if (motion) this.stats.motion++;
      const parts: (B | typeof SAME | null)[] = [];
      const entry = this.entry(widget, selected, painted, mode, reason, parts);
      painted.surfaces.forEach((surface, k) => {
        if (!surface || parts[k] === SAME) return;
        let source = shared.get(surface.content);
        if (source === undefined) { source = sources.push(surface.source) - 1; shared.set(surface.content, source); }
        slots.push([entries.length, k, source]);
      });
      entries.push(entry);
      surfaces.push(parts);
    }
    const crops = slots.length ? await this.host.crop(slots.map(([, , source]) => sources[source]!)) : [];
    this.stats.crops += crops.length;
    slots.forEach(([e, k], at) => { surfaces[e]![k] = crops[at]!; });
    const resets: [number, number][] = [];
    for (const route of new Set([...this.subs.values()].flatMap(sub => sub.route === null ? [] : [sub.route]))) resets.push([route, this.facts.generation(route).reset]);
    if (entries.length) this.host.present({ lane: this.host.lane, serial: request.serial, resets, entries }, crops);
    this.stats.entries += entries.length;
    this.prune(request.now);
    const bytes = this.freed + this.facts.releasedBytes();
    this.freed = 0;
    if (bytes) this.host.release(bytes);
    this.host.stretch?.(this.time() - began);
  }

  private entry(widget: Widget, selected: Selected, painted: Painted, mode: Entry['mode'], reason: string | undefined, parts: (B | typeof SAME | null)[]): Entry<B> {
    const id = this.host.lane + 1 + this.host.lanes * this.nextEntry++, contents: Record<string, string> = { hour: String(selected.window[0]) };
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
    const gen = route === null ? { seek: 0, reset: 0 } : this.facts.generation(route);
    const rank = widget.subs.reduce((at, sub) => Math.max(at, this.subs.get(sub)?.rank ?? 0), 0);
    const sent: Sent = { id, request: this.requests, pictures: selected.pictures, h: selected.h, c: selected.c, binding: widget.binding.generation, contents, inspect: widget.inspect };
    widget.sent = { ...contents };
    widget.inflight = sent;
    this.presented.set(id, widget.id);
    return Object.freeze({ widget: widget.id, binding: widget.binding.generation, id, h: selected.h, c: selected.c, lo: selected.window[0], hi: selected.window[1],
      gen: Object.freeze({ ...gen }), follows: selected.pictures.some(picture => followsChannel(picture)), ...rank ? { rank } : {}, mode, ...reason !== undefined ? { reason } : {},
      surfaces: parts, content: Object.freeze(contents), fields: Object.freeze(fields) });
  }

  private prune(actual: number) {
    const keep = new Map<number, Picture<Value>[]>();
    for (const widget of this.widgets.values()) widget.subs.forEach((sub, at) => {
      let list = keep.get(sub);
      if (!list) keep.set(sub, list = []);
      for (const sent of [widget.installed, widget.inflight]) if (sent?.pictures[at]) list.push(sent.pictures[at]!);
    });
    for (const sub of this.subs.keys()) this.facts.prune(sub, actual, keep.get(sub) ?? []);
  }
}
