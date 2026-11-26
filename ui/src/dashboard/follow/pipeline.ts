import { flushSync } from 'react-dom';
import { engine } from '../../engine/frame.ts';
import type { DashboardClient, DashboardSource, FollowOpen } from '../client';
import type { PlaybackController, PlaybackSnapshot } from '../playback';
import { canonical } from './canonical.ts';
import { Page, type PageMessage, type PageSurface, type RebindChange, type WidgetView } from './page.ts';
import type { Channel } from './facts';
import type { Binding, Dated, Supply } from '../../draw/lane';
import { PayloadSender } from './payloads.ts';
import type { FromLane, ToLane } from '../../draw/laneMessages';
import { auditing, noteFrame, recordPicture } from '../pulled.ts';
import { perfLog } from '../../shell/perf.ts';
import type { AuditRow } from '../../draw/notifications';
import { productRequest } from '../request.ts';
import type { GatedParams } from '../regionTime';
import { asFailure, RefusedError, type Failure } from '../queryError.ts';

export type FollowQuery = Readonly<Record<string, unknown>>;

export interface LaneWorker { postMessage(message: ToLane, transfer?: Transferable[]): void; onmessage: ((event: MessageEvent<FromLane>) => void) | null }

export interface PipelineHost {
  lanes: readonly LaneWorker[];
  open(ports: readonly MessagePort[]): FollowOpen;
  channel(): { port1: MessagePort; port2: MessagePort };
  frameRate(): number;
  rate(): number;
  close?(): void;
}

export type SubStatus = 'pending' | 'followed' | 'refused' | 'failed';

interface SubState {
  id: number; key: string; lane: number; channel: Channel; route: RouteState; holders: number; status: SubStatus; reason?: string; error?: Failure;
  widgets: Map<number, number>; released: boolean;
}

interface RouteState {
  page: number; native: number | null; key: string; source: DashboardSource; branch: string; offset: number;
  serial: Promise<unknown>; told: string | null; error?: { seek: number; failure: Failure };
}

interface WidgetState {
  id: number; lane: number; kind?: string; cost: number; subs: Map<object, number>; view: string; model?: unknown; mounted: boolean; route: RouteState | null;
  installListeners: Set<() => void>; dateListeners: Set<() => void>; binding: Binding; surfaces: readonly PageSurface<ImageBitmap>[];
  held: Map<string, unknown>; error?: Failure; wire?: unknown; stamp: number | undefined; payloads: { own: ReadonlySet<number>; statics: ReadonlySet<number>; identity: ReadonlySet<number> };
}

const NO_PAYLOADS: ReadonlySet<number> = new Set();

export interface SupplyChange {
  readonly stamp?: number;
  readonly statics?: { readonly staticPanels?: unknown; readonly staticAnswers?: unknown; readonly staticShifts?: unknown } | null;
  readonly identity?: object | null;
  readonly dated?: Dated | null;
}

export interface MountOptions { offset: number; pin: number | null; owner?: string; bounds?: readonly [number, number] | null; view?: string; model?: unknown; key?: string; widget?: string; kind?: string }

export interface PointerHitMeta {
  readonly installedId?: number;
  readonly owner?: string;
}

const NO_CHANNEL = 'This connection has no channel for pushed answers';
const DEFAULT_COST = 0.5;

export class Pipeline {
  readonly page: Page<ImageBitmap>;
  readonly stats = { dateVisits: 0 };
  readonly generation: number;
  private readonly host: PipelineHost;
  private readonly clock: PlaybackController;
  private readonly client: DashboardClient;
  private readonly abort = new AbortController();
  private readonly subs = new Map<string, SubState>();
  private readonly byId = new Map<number, SubState>();
  private readonly routes = new Map<string, RouteState>();
  private readonly widgets = new Map<number, WidgetState>();
  private readonly dateWatchers = new Set<WidgetState>();
  private readonly loads: number[];
  private readonly costLoads: number[];
  private readonly kindCost = new Map<string, number>();
  private readonly placed = new Map<string, number>();
  private readonly payloads: PayloadSender[];
  private readonly nextSub: number[];
  private nextWidget = 1;
  private nextRoute = 1;
  private opened: Promise<void> | null = null;
  private readonly ports: readonly MessagePort[];
  private changed = new Set<WidgetState>();
  private redated = new Set<WidgetState>();
  private dated = false;
  private framing = false;
  private closed = false;
  private closeReader: (() => void) | null = null;
  private stopPublish: (() => void) | null = null;
  private stopClock: (() => void) | null = null;
  private rate: number;

  constructor(clock: PlaybackController, client: DashboardClient, host: PipelineHost, generation = 0) {
    this.clock = clock; this.client = client; this.host = host; this.rate = host.rate(); this.generation = generation;
    const count = host.lanes.length;
    perfLog?.lanes(count);
    this.loads = Array.from({ length: count }, () => 0);
    this.costLoads = Array.from({ length: count }, () => 0);
    this.nextSub = Array.from({ length: count }, () => 1);
    this.payloads = host.lanes.map(lane => new PayloadSender(message => lane.postMessage(message)));
    this.page = new Page<ImageBitmap>({ lanes: count, seek: clock.getSnapshot().seekSequence, post: (lane, message) => this.post(lane, message) });
    host.lanes.forEach((lane, index) => { lane.onmessage = ({ data }) => this.receive(index, data); });
    const channels = host.lanes.map(() => host.channel());
    channels.forEach((channel, lane) => host.lanes[lane]!.postMessage({ type: 'init', lane, lanes: count, port: channel.port1, rate: this.rate }, [channel.port1]));
    this.ports = channels.map(channel => channel.port2);
    this.stopPublish = clock.onPublish((previous, next) => this.published(previous, next), true);
    this.stopClock = clock.subscribe(() => this.anchor());
    const s = clock.getSnapshot();
    this.page.publish(s.hour, { playing: s.playing, speed: s.hoursPerSecond, step: 0 });
  }

  get lanes() { return this.host.lanes.length; }

  close() {
    if (this.closed) return;
    this.closed = true;
    this.abort.abort();
    this.stopPublish?.(); this.stopPublish = null;
    this.stopClock?.(); this.stopClock = null;
    for (const lane of this.host.lanes) lane.onmessage = null;
    this.closeReader?.(); this.closeReader = null;
    this.host.close?.();
    for (const port of this.ports) port.close();
    this.page.dispose();
    perfLog?.dropGeneration(this.generation);
    this.widgets.clear(); this.shared.clear(); this.subs.clear(); this.byId.clear(); this.routes.clear();
    this.dateWatchers.clear(); this.placed.clear(); this.kindCost.clear();
    this.audits.clear(); this.answered.clear(); this.hits.clear(); this.changed.clear(); this.redated.clear();
  }

  private post(lane: number | 'all', message: PageMessage) {
    if (this.closed) return;
    if (lane === 'all') for (const worker of this.host.lanes) worker.postMessage(message);
    else this.host.lanes[lane]!.postMessage(message);
  }

  private open(): Promise<void> {
    if (!this.opened) {
      const started = this.host.open(this.ports);
      const ready = 'ready' in started ? started.ready : started;
      if ('close' in started) {
        if (this.closed) started.close();
        else this.closeReader = started.close;
      }
      this.opened = ready.then(opened => {
        const session = typeof opened === 'string' ? opened : opened?.session;
        const close = typeof opened === 'object' && opened ? opened.close : undefined;
        if (this.closed) { close?.(); return; }
        if (close && !this.closeReader) this.closeReader = close;
        if (typeof session === 'string') this.session = session;
      });
    }
    return this.opened;
  }

  private session: string | undefined;

  private holds(route: RouteState) {
    for (const sub of this.subs.values()) if (sub.route === route && sub.status === 'followed' && sub.channel !== 'pinned') return true;
    return false;
  }

  private published(previous: PlaybackSnapshot, next: PlaybackSnapshot) {
    if (this.closed) return;
    const rate = this.host.rate();
    if (rate !== this.rate) { this.rate = rate; for (const lane of this.host.lanes) lane.postMessage({ type: 'options', rate }); }
    const fps = Math.max(1, this.host.frameRate());
    const nominal = Math.max(1, Math.ceil(next.hoursPerSecond / fps * 1.5));
    const rawStep = next.playing && previous.playing && next.seekSequence === previous.seekSequence ? Math.max(0, next.hour - previous.hour) : this.page.step;
    const step = next.playing ? Math.min(rawStep, nominal) : rawStep;
    this.page.publish(next.hour, { playing: next.playing, seek: next.seekSequence, step, speed: next.hoursPerSecond });
    const pictures = this.page.takePictures();
    if (perfLog && next.playing) perfLog.pictures(pictures, this.expiredNames());
    this.dated = true;
    this.flush();
    if (next.playing) noteFrame(performance.now());
    if (auditing()) this.sample(next.seekSequence);
  }

  private readonly audits = new Map<number, Map<number, (fields: Readonly<Record<string, unknown>>) => readonly AuditRow[] | undefined>>();

  audit(id: number, owner: number, rows: ((fields: Readonly<Record<string, unknown>>) => readonly AuditRow[] | undefined) | null) {
    if (this.closed) return;
    let owners = this.audits.get(id);
    if (rows) { if (!owners) this.audits.set(id, owners = new Map()); owners.set(owner, rows); }
    else { owners?.delete(owner); if (owners && !owners.size) this.audits.delete(id); }
  }

  private readonly answered = new Map<number, Map<number, { hour: number; entry: number; rows: () => readonly AuditRow[] | undefined }>>();
  private answers = 0;

  answer(id: number, owner: number, shown: { hour: number; rows: () => readonly AuditRow[] | undefined } | null) {
    if (this.closed) return;
    let owners = this.answered.get(id);
    if (shown) { if (!owners) this.answered.set(id, owners = new Map()); owners.set(owner, { ...shown, entry: -++this.answers }); }
    else { owners?.delete(owner); if (owners && !owners.size) this.answered.delete(id); }
    if (auditing()) this.sample(this.clock.getSnapshot().seekSequence);
  }

  private sample(seek: number) {
    const at = performance.timeOrigin + performance.now();
    for (const held of this.shared.values()) {
      const view = this.page.view(held.id), widget = this.widgets.get(held.id), shown = this.page.widgets.get(held.id);
      if (!widget || !shown) continue;
      const offset = shown.offset, clock = (hour: number) => hour - offset;
      const name = held.widget, surface = widget.view === 'table' || widget.view === 'race' ? widget.view : 'page';
      const answers = [...this.answered.get(held.id) ?? []];
      for (const [owner, answer] of answers) {
        const hour = clock(answer.hour), rows = answer.rows();
        recordPicture({ t: 'picture', at, shownAt: at, seek, widget: name, surface, placement: held.id, owner, generation: this.generation, renderer: 'page', head: clock(view.target), hour, offset, fixedHour: hour,
          since: hour, through: hour, eligibleHour: hour, entry: answer.entry, parts: [], drawMs: 0, ...rows ? { rows } : {},
          mode: 'picture', dim: false, reset: 0, binding: shown.binding, inspect: shown.inspect, tooltip: false });
      }
      const hour = view.mode === 'picture' && view.through !== undefined ? clock(view.through) : null;
      const owners = [...this.audits.get(held.id) ?? []];
      if (!owners.length && answers.length) continue;
      for (const [owner, audit] of owners.length ? owners : [[0, undefined] as const]) {
      const rows = view.mode === 'picture' && view.fields ? audit?.(view.fields) : undefined;
      const auditMissing = audit !== undefined && view.mode === 'picture' && rows === undefined;
      const rawAudit = view.mode === 'picture' && view.fields ? view.fields.audit : undefined;
      const auditId = rawAudit && typeof rawAudit === 'object' && typeof (rawAudit as { id?: unknown }).id === 'number' && (rawAudit as { id: number }).id > 0
        ? (rawAudit as { id: number }).id
        : undefined;
      if (auditId !== undefined) perfLog?.pinPicture(this.generation, held.id, owner, auditId);
      recordPicture({ t: 'picture', at, shownAt: at, seek, widget: name,
        surface, placement: held.id, owner, generation: this.generation, renderer: 'page',
        head: clock(view.target), hour, offset, ...shown.pin !== null ? { fixedHour: clock(shown.pin) } : {},
        ...view.h !== undefined ? { since: clock(view.h) } : {}, ...shown.installed ? { through: clock(shown.installed.c) } : {},
        eligibleHour: shown.installed && view.mode === 'picture' ? clock(Math.min(view.target, shown.installed.c)) : null,
        ...shown.installed ? { entry: shown.installed.id, parts: Object.values(shown.content),
          face: Object.entries(shown.content).filter(([part]) => part !== 'hour').map(([part, id]) => `${part}=${id}`).join('|') } : {},
        drawMs: shown.installed?.ms ?? 0, ...rows ? { rows } : {}, ...auditId !== undefined && view.fields ? { auditId, fields: view.fields } : {}, ...auditMissing ? { auditMissing: true } : {},
        mode: view.mode, ...view.reason !== undefined ? { reason: view.reason } : {}, dim: view.dim, reset: shown.installed?.gen.reset ?? 0, binding: shown.installed?.binding ?? shown.binding,
        inspect: shown.inspect, tooltip: view.tooltip !== null });
      }
    }
  }

  private flush() {
    if (this.closed) return;
    const changed = this.changed, dated = this.dated, redated = this.redated;
    this.changed = new Set(); this.redated = new Set();
    this.dated = false;
    if (changed.size) flushSync(() => { for (const widget of changed) for (const listener of [...widget.installListeners]) listener(); });
    if (dated) {
      for (const widget of this.dateWatchers) {
        this.stats.dateVisits++;
        for (const listener of [...widget.dateListeners]) listener();
      }
    } else if (redated.size) {
      for (const widget of redated) if (widget.dateListeners.size) {
        this.stats.dateVisits++;
        for (const listener of [...widget.dateListeners]) listener();
      }
    }
  }

  private receive(lane: number, message: FromLane) {
    if (this.closed) return;
    if (message.type === 'failure') {
      const routes = new Set<number>();
      for (const widget of this.widgets.values()) if (message.lane === -1 || widget.lane === lane) routes.add(this.page.widgets.get(widget.id)!.route);
      const failure = asFailure(new Error(message.reason));
      for (const route of this.routes.values()) {
        if (routes.has(route.page)) route.error = { seek: this.page.seek, failure };
      }
      for (const route of routes) this.page.fail(route, this.page.command(route), message.reason);
      this.flush();
      return;
    }
    if (message.type === 'sub-failed') {
      const held = this.byId.get(message.sub);
      if (held && held.status !== 'failed' && held.status !== 'refused') {
        if (held.channel !== 'pinned' && held.status === 'followed') {
          void this.ask('unfollow', { id: held.id }, held.route, new AbortController().signal).catch(() => {});
        }
        this.fail(held, asFailure(new Error(message.reason)));
        if (!this.holds(held.route)) {
          held.route.told = null;
          const page = this.nextRoute++;
          this.page.carry(held.route.page, page);
          held.route.page = page;
        }
      }
      return;
    }
    if (message.type === 'widget-failed') {
      if (this.widgets.has(message.widget)) {
        const failure = asFailure(new Error(message.reason));
        this.page.name(message.widget, 'failed', failure.message, 'local');
        this.failedLocally(message.widget, message.stamp, failure);
        this.flush();
      }
      return;
    }
    if (message.type === 'hit') {
      const told = this.hits.get(message.widget)?.shift();
      const pageWidget = this.page.widgets.get(message.widget);
      const staleOwner = message.owner !== undefined && pageWidget?.owner !== undefined && message.owner !== pageWidget.owner;
      const staleInstall = told !== undefined && message.installedId !== told.installed;
      const acceptedKey = staleOwner || staleInstall ? null : message.key;
      if (!staleOwner && !staleInstall && pageWidget?.installed?.id === message.installedId) {
        this.page.inspect(message.widget, acceptedKey);
        this.flush();
      }
      told?.hit?.(acceptedKey, {
        ...(message.installedId !== undefined ? { installedId: message.installedId } : {}),
        ...(message.owner !== undefined ? { owner: message.owner } : {}),
      });
      return;
    }
    if (message.type === 'served') {
      if (message.costs) {
        for (const [id, cost] of message.costs) {
          const widget = this.widgets.get(id);
          if (!widget || !Number.isFinite(cost)) continue;
          const clamped = Math.max(0.05, cost);
          this.costLoads[widget.lane] = Math.max(0, this.costLoads[widget.lane]! + clamped - widget.cost);
          widget.cost = clamped;
          if (widget.kind !== undefined) {
            const prev = this.kindCost.get(widget.kind);
            this.kindCost.set(widget.kind, prev === undefined ? clamped : prev * 0.7 + clamped * 0.3);
          }
        }
      }
      if (auditing()) recordPicture({ t: 'workerFrame', at: performance.timeOrigin + performance.now(), lane, lanes: this.lanes, seek: message.seek, head: message.target,
        playing: !message.exact && this.page.playing, changed: message.changed, workMs: message.ms, ...message.cancelled ? { cancelled: true } : {} });
      return;
    }
    this.page.offer(message);
    if (!this.page.playing) this.commitSoon();
  }

  private commitSoon() {
    if (this.closed || this.framing) return;
    this.framing = true;
    engine.next('commit', () => { this.framing = false; if (this.closed) return; this.page.frame(); const pictures = this.page.takePictures(); if (perfLog && this.page.playing) perfLog.pictures(pictures, this.expiredNames()); for (const [id, stamp] of this.page.takeFailures()) this.failedLocally(id, stamp, asFailure(new Error('drawing failed'))); this.flush(); if (auditing()) this.sample(this.page.seek); });
  }

  private place(key: string | undefined, kind: string | undefined): { lane: number; cost: number } {
    const cost = (kind !== undefined ? this.kindCost.get(kind) : undefined) ?? DEFAULT_COST;
    const average = this.costLoads.reduce((sum, load) => sum + load, 0) / this.costLoads.length;
    const held = key === undefined ? undefined : this.placed.get(key);
    if (held !== undefined && this.costLoads[held]! + cost <= average + Math.max(cost, 1)) return { lane: held, cost };
    let lane = 0;
    for (let at = 1; at < this.costLoads.length; at++) {
      if (this.costLoads[at]! < this.costLoads[lane]! - 1e-6
        || (Math.abs(this.costLoads[at]! - this.costLoads[lane]!) <= 1e-6 && this.loads[at]! < this.loads[lane]!)) lane = at;
    }
    if (key !== undefined) this.placed.set(key, lane);
    return { lane, cost };
  }

  mount(options: MountOptions) {
    const kind = options.kind;
    const { lane, cost } = this.place(options.key, kind);
    const id = this.nextWidget++;
    this.loads[lane]!++;
    this.costLoads[lane]! += cost;
    const widget: WidgetState = { id, lane, kind, cost, subs: new Map(), view: options.view ?? 'fields', model: options.model, mounted: false, route: null,
      installListeners: new Set(), dateListeners: new Set(), binding: { generation: 1, ...(options.owner !== undefined ? { owner: options.owner } : {}), surfaces: [], offset: options.offset, pin: options.pin }, surfaces: [], held: new Map(), stamp: undefined,
      payloads: { own: NO_PAYLOADS, statics: NO_PAYLOADS, identity: NO_PAYLOADS } };
    if (options.model !== undefined) this.own(widget, options.model);
    this.widgets.set(id, widget);
    this.page.mount(id, { route: 0, lane, ...(options.owner !== undefined ? { owner: options.owner } : {}), offset: options.offset, pin: options.pin, bounds: options.bounds, surfaces: [], changed: () => this.changed.add(widget),
      dated: () => this.redated.add(widget) });
    return id;
  }

  private readonly shared = new Map<string, { id: number; holders: number; widget: string }>();

  private expiredNames(): string[] {
    const expired = this.page.expired();
    if (!expired.length) return [];
    const ids = new Set(expired), names = new Set<string>();
    for (const held of this.shared.values()) if (ids.has(held.id)) names.add(held.widget);
    return [...names];
  }

  acquire(key: string, options: MountOptions): number {
    let held = this.shared.get(key);
    if (!held) this.shared.set(key, held = { id: this.mount({ ...options, key: options.key ?? key }), holders: 0,
      widget: options.widget ?? (key.includes('|') ? key.slice(0, key.lastIndexOf('|')) : key) });
    held.holders++;
    return held.id;
  }

  drop(key: string) {
    const held = this.shared.get(key);
    if (!held || --held.holders > 0) return;
    this.shared.delete(key);
    this.unmount(held.id);
  }

  unmount(id: number) {
    if (this.closed) return;
    const widget = this.widgets.get(id);
    if (!widget) return;
    this.audits.delete(id);
    this.hits.delete(id);
    this.dateWatchers.delete(widget);
    for (const sub of widget.subs.values()) this.release(sub, id);
    const sender = this.payloads[widget.lane]!;
    for (const ids of Object.values(widget.payloads)) sender.release(ids);
    this.widgets.delete(id);
    this.loads[widget.lane]!--;
    this.costLoads[widget.lane] = Math.max(0, this.costLoads[widget.lane]! - widget.cost);
    this.page.unmount(id);
    this.host.lanes[widget.lane]!.postMessage({ type: 'unmount', widget: id });
  }

  lane(id: number) { return this.widgets.get(id)?.lane ?? 0; }

  status(sub: number): { status: SubStatus; reason?: string; error?: Failure } {
    const held = this.byId.get(sub);
    if (!held) return { status: 'pending' };
    const routeError = held.route.error?.seek === this.page.seek ? held.route.error.failure : undefined;
    const error = held.error ?? routeError;
    return { status: held.status, ...held.reason !== undefined ? { reason: held.reason } : {}, ...error !== undefined ? { error } : {} };
  }

  widgetError(id: number): Failure | undefined {
    const widget = this.widgets.get(id);
    if (!widget) return undefined;
    if (widget.error) return widget.error;
    for (const sub of widget.subs.values()) {
      const error = this.status(sub).error;
      if (error) return error;
    }
    return undefined;
  }

  binding(id: number): number { return this.page.widgets.get(id)?.binding ?? 0; }

  private failedLocally(id: number, stamp: number | undefined, failure: Failure) {
    const widget = this.widgets.get(id);
    if (!widget) return;
    widget.error = failure;
    if (stamp !== widget.stamp) this.retryWidget(id);
  }

  retryWidget(id: number) {
    if (this.closed) return;
    const widget = this.widgets.get(id);
    if (!widget) return;
    widget.error = undefined;
    this.page.retryLocal(id);
    widget.binding = this.timed(widget, widget.binding);
    this.host.lanes[widget.lane]!.postMessage({ type: 'bind', widget: id, binding: widget.binding, retry: true, ...widget.wire !== undefined ? { model: widget.wire } : {} });
    this.flush();
  }

  view(id: number): WidgetView | null { return this.widgets.has(id) ? this.page.view(id) : null; }

  onInstall(id: number, listener: () => void) {
    const widget = this.widgets.get(id);
    widget?.installListeners.add(listener);
    return () => { widget?.installListeners.delete(listener); };
  }

  onDate(id: number, listener: () => void) {
    const widget = this.widgets.get(id);
    if (widget) {
      widget.dateListeners.add(listener);
      this.dateWatchers.add(widget);
    }
    return () => {
      if (!widget) return;
      widget.dateListeners.delete(listener);
      if (!widget.dateListeners.size) this.dateWatchers.delete(widget);
    };
  }

  private routeOf(source: DashboardSource, branch: string, offset: number): RouteState {
    const key = JSON.stringify([source.id, source.revision, branch, offset]);
    let route = this.routes.get(key);
    if (!route) this.routes.set(key, route = { page: this.nextRoute++, native: null, key, source, branch, offset, serial: Promise.resolve(), told: null });
    return route;
  }

  subscribe(widgetId: number, token: object, source: DashboardSource, branch: string, offset: number, query: FollowQuery, channel: Channel, product?: string) {
    if (this.closed) return;
    const widget = this.widgets.get(widgetId);
    if (!widget) return;
    const route = this.routeOf(source, branch, offset);
    const key = JSON.stringify([widget.lane, route.key, channel, canonical(query)]);
    let sub = this.subs.get(key);
    if (!sub) {
      const id = this.nextSub[widget.lane]!++ * this.lanes + widget.lane;
      sub = { id, key, lane: widget.lane, channel, route, holders: 0, status: 'pending', widgets: new Map(), released: false };
      this.subs.set(key, sub); this.byId.set(id, sub);
      this.register(sub, query, product);
    }
    sub.holders++;
    sub.widgets.set(widgetId, (sub.widgets.get(widgetId) ?? 0) + 1);
    const was = widget.subs.get(token);
    widget.subs.set(token, sub.id);
    if (was !== undefined) this.release(was, widgetId);
    widget.route = route;
    this.remount(widget);
    return sub.id;
  }

  unsubscribe(widgetId: number, token: object) {
    if (this.closed) return;
    const widget = this.widgets.get(widgetId);
    const id = widget?.subs.get(token);
    if (!widget || id === undefined) return;
    widget.subs.delete(token);
    this.release(id, widgetId);
    this.remount(widget);
  }

  private release(id: number, widget: number) {
    const sub = this.byId.get(id);
    if (!sub) return;
    const count = sub.widgets.get(widget);
    if (count !== undefined) {
      if (count <= 1) sub.widgets.delete(widget);
      else sub.widgets.set(widget, count - 1);
    }
    if (--sub.holders > 0) return;
    this.subs.delete(sub.key); this.byId.delete(id);
    sub.released = true;
    this.host.lanes[sub.lane]!.postMessage({ type: 'unfollow', sub: id });
    if (sub.channel !== 'pinned' && sub.status === 'followed') void this.ask('unfollow', { id }, sub.route, new AbortController().signal).catch(() => {});
    if (!this.holds(sub.route)) { sub.route.told = null; const page = this.nextRoute++; this.page.carry(sub.route.page, page); sub.route.page = page; }
  }

  private remount(widget: WidgetState) {
    const subs = [...new Set(widget.subs.values())];
    this.host.lanes[widget.lane]!.postMessage({ type: 'mount', widget: widget.id, subs, view: widget.view, model: widget.wire });
    this.page.rebind(widget.id, { subscription: true });
    const followed = subs.map(id => this.byId.get(id)).find(sub => sub?.status === 'followed');
    if (followed) this.page.mountRoute(widget.id, followed.route.page);
    widget.binding = this.timed(widget, widget.binding);
    this.host.lanes[widget.lane]!.postMessage({ type: 'bind', widget: widget.id, binding: widget.binding });
    widget.mounted = true;
    const subStates = subs.map(id => this.byId.get(id));
    const refused = subStates.find(sub => sub?.status === 'refused');
    const failed = subStates.find(sub => sub?.status === 'failed');
    if (refused) this.page.name(widget.id, 'refused', refused.reason);
    else if (failed) this.page.name(widget.id, 'failed', failed.reason, 'stream');
    else if (widget.error) this.page.name(widget.id, 'failed', widget.error.message, 'local');
    this.flush();
  }

  private timed(widget: WidgetState, binding: Omit<Binding, 'generation' | 'offset' | 'pin'>): Binding {
    const shown = this.page.widgets.get(widget.id);
    const { dated: _, ...own } = binding;
    const was = widget.binding;
    const owner = shown?.owner;
    const kept = was.owner === undefined || owner === undefined || was.owner === owner;
    if (!kept) widget.stamp = undefined;
    return { ...own, ...kept && was.dated ? { dated: was.dated } : {}, generation: shown?.binding ?? 1, ...(shown?.owner !== undefined ? { owner: shown.owner } : {}), offset: shown?.offset ?? 0, pin: shown?.pin ?? null };
  }

  bind(id: number, surfaces: readonly PageSurface<ImageBitmap>[], binding: Omit<Binding, 'generation' | 'offset' | 'pin'>, change: RebindChange = {}, model?: unknown) {
    if (this.closed) return;
    const widget = this.widgets.get(id);
    if (!widget) return;
    const same = surfaces.length === widget.surfaces.length && surfaces.every((surface, at) => surface === widget.surfaces[at]);
    if (!same) { widget.surfaces = surfaces; this.page.remount(id, surfaces); }
    else this.page.rebind(id, change);
    if (!same || change.subscription) {
      const subStates = [...widget.subs.values()].map(sub => this.byId.get(sub));
      const refused = subStates.find(sub => sub?.status === 'refused');
      const failed = subStates.find(sub => sub?.status === 'failed');
      if (refused) this.page.name(id, 'refused', refused.reason);
      else if (failed) this.page.name(id, 'failed', failed.reason, 'stream');
      else if (widget.error) this.page.name(id, 'failed', widget.error.message, 'local');
    }
    const release = model !== undefined ? this.own(widget, model) : NO_PAYLOADS;
    widget.binding = this.timed(widget, binding);
    this.host.lanes[widget.lane]!.postMessage({
      type: 'bind',
      widget: id,
      binding: widget.binding,
      ...model !== undefined ? { model: widget.wire } : {},
      ...!same ? { remounted: true } : {},
    });
    this.payloads[widget.lane]!.release(release);
    this.flush();
  }

  present(id: number, view: string, model?: unknown) {
    if (this.closed) return;
    const widget = this.widgets.get(id);
    if (!widget || widget.view === view && model === undefined) return;
    widget.view = view;
    const release = model !== undefined ? this.own(widget, model) : NO_PAYLOADS;
    this.host.lanes[widget.lane]!.postMessage({ type: 'mount', widget: id, subs: [...new Set(widget.subs.values())], view, model: widget.wire });
    this.payloads[widget.lane]!.release(release);
    this.page.rebind(id);
    widget.binding = this.timed(widget, widget.binding);
    this.host.lanes[widget.lane]!.postMessage({ type: 'bind', widget: id, binding: widget.binding });
    this.flush();
  }

  private own(widget: WidgetState, model: unknown): ReadonlySet<number> {
    const was = widget.payloads.own;
    const encoded = this.payloads[widget.lane]!.acquire(model);
    widget.model = model;
    widget.wire = encoded.wire;
    widget.payloads = { ...widget.payloads, own: encoded.ids };
    return was;
  }

  supply(id: number, change: SupplyChange) {
    if (this.closed) return;
    const widget = this.widgets.get(id);
    if (!widget) return;
    const sender = this.payloads[widget.lane]!;
    const released: ReadonlySet<number>[] = [];
    let statics: unknown;
    if (change.statics !== undefined) {
      const encoded = sender.acquire(change.statics);
      released.push(widget.payloads.statics);
      widget.payloads = { ...widget.payloads, statics: encoded.ids };
      statics = encoded.wire;
    }
    let identity: number | null | undefined;
    if (change.identity !== undefined) {
      const encoded = sender.acquire(change.identity === null ? {} : { identitySnapshot: change.identity });
      const [next] = encoded.ids;
      const [held] = widget.payloads.identity;
      if (next === held) sender.release(encoded.ids);
      else {
        released.push(widget.payloads.identity);
        widget.payloads = { ...widget.payloads, identity: encoded.ids };
        identity = next ?? null;
      }
    }
    if (statics !== undefined) widget.stamp = change.stamp;
    const dated = change.dated !== undefined && (statics !== undefined || change.stamp === widget.stamp) ? change.dated : undefined;
    if (dated !== undefined) {
      const { dated: _, ...rest } = widget.binding;
      widget.binding = dated ? { ...rest, dated } : rest;
    }
    if (statics !== undefined || identity !== undefined || dated !== undefined) {
      this.host.lanes[widget.lane]!.postMessage({
        type: 'supply',
        widget: widget.id,
        ...widget.stamp !== undefined && (statics !== undefined || dated !== undefined) ? { stamp: widget.stamp } : {},
        ...statics !== undefined ? { statics: statics as Supply['statics'] } : {},
        ...identity !== undefined ? { identity } : {},
        ...dated !== undefined ? { dated } : {},
      });
    }
    for (const ids of released) sender.release(ids);
    if (statics !== undefined && widget.error) this.retryWidget(id);
  }

  held<T>(id: number, kind: string, make: () => T): T | null {
    const widget = this.widgets.get(id);
    if (!widget) return null;
    if (!widget.held.has(kind)) widget.held.set(kind, make());
    return widget.held.get(kind) as T;
  }

  retime(id: number, change: RebindChange) {
    if (this.closed) return;
    const widget = this.widgets.get(id);
    if (!widget) return;
    this.page.rebind(id, change);
    widget.binding = this.timed(widget, widget.binding);
    this.host.lanes[widget.lane]!.postMessage({ type: 'bind', widget: id, binding: widget.binding });
    this.flush();
  }

  inspect(id: number, key: string | null) { if (this.closed) return; this.page.inspect(id, key); this.flush(); }

  inspecting(id: number) { return this.page.widgets.get(id)?.inspect ?? null; }

  private readonly hits = new Map<number, { readonly hit?: (key: string | null, meta?: PointerHitMeta) => void; readonly installed?: number }[]>();

  pointer(id: number, surface: number, point: [number, number] | null, hit?: (key: string | null, meta?: PointerHitMeta) => void) {
    if (this.closed) return;
    const widget = this.widgets.get(id);
    if (!widget) return;
    let waiting = this.hits.get(id);
    if (!waiting) this.hits.set(id, waiting = []);
    const installed = this.page.widgets.get(id)?.installed?.id;
    waiting.push({ ...(hit ? { hit } : {}), ...(installed !== undefined ? { installed } : {}) });
    this.host.lanes[widget.lane]!.postMessage({ type: 'pointer', widget: id, surface, point, ...(installed !== undefined ? { installed } : {}) });
  }

  exact(sub: number, hour: number, data: unknown, refused: string | null) {
    if (this.closed) return;
    const held = this.byId.get(sub);
    if (held) this.host.lanes[held.lane]!.postMessage({ type: 'exact', sub, hour, data, refused });
  }

  private ask(product: 'follow' | 'unfollow' | 'follow-clock', params: Record<string, unknown>, route: RouteState, signal = this.abort.signal) {
    const sessioned = this.session ? { ...params, session: this.session } : params;
    return this.client.product(productRequest({ sourceId: route.source.id, revision: route.source.revision, product, params: sessioned as GatedParams }),
      signal).then(reply => reply.data);
  }

  private register(sub: SubState, query: FollowQuery, product?: string) {
    if (this.closed) return;
    this.host.lanes[sub.lane]!.postMessage({ type: 'follow', sub: sub.id, channel: sub.channel, ...product !== undefined ? { product } : {} });
    if (sub.channel === 'pinned') { sub.status = 'followed'; return; }
    if (!this.client.openFollow) { this.refuse(sub, NO_CHANNEL); return; }
    void this.open().then(() => this.closed || sub.released ? null
      : this.ask('follow', { id: sub.id, query, branchId: sub.route.branch, clock: sub.route.offset, lane: sub.lane, lanes: this.lanes }, sub.route))
      .then(reply => {
        const data = reply as { available?: boolean; reason?: string; route?: number } | null;
        if (this.closed || sub.released || sub.status === 'failed' || sub.status === 'refused') {
          if (data?.available === true) void this.ask('unfollow', { id: sub.id }, sub.route, new AbortController().signal).catch(() => {});
          return;
        }
        if (data?.available !== true || !Number.isInteger(data.route)) { this.refuse(sub, data?.reason ?? 'The service did not say whether it follows this'); return; }
        const first = !this.holds(sub.route), moved = sub.route.native !== null && sub.route.native !== data.route;
        if (moved) sub.route.page = this.nextRoute++;
        sub.status = 'followed';
        sub.route.native = data.route!;
        this.host.lanes[sub.lane]!.postMessage({ type: 'route', sub: sub.id, route: sub.route.page, native: data.route!, seek: this.page.seek, offset: sub.route.offset });
        for (const widget of sub.widgets.keys()) this.page.mountRoute(widget, sub.route.page);
        if (first || moved) sub.route.told = null;
        this.anchor();
      }, error => {
        if (this.closed || sub.released || sub.status === 'failed' || sub.status === 'refused') return;
        const failure = asFailure(error);
        if (error instanceof RefusedError) this.refuse(sub, failure.message);
        else this.fail(sub, failure);
      });
  }

  private refuse(sub: SubState, reason: string) {
    if (this.closed) return;
    sub.status = 'refused'; sub.reason = reason; sub.error = undefined;
    for (const widget of sub.widgets.keys()) this.page.name(widget, 'refused', reason);
    this.flush();
  }

  private fail(sub: SubState, failure: Failure) {
    if (this.closed) return;
    sub.status = 'failed'; sub.reason = failure.message; sub.error = failure;
    for (const widget of sub.widgets.keys()) this.page.name(widget, 'failed', failure.message, 'stream');
    this.flush();
  }

  private anchor() {
    if (this.closed) return;
    const s = this.clock.getSnapshot();
    const held = new Set([...this.subs.values()].filter(sub => sub.status === 'followed' && sub.channel !== 'pinned').map(sub => sub.route));
    for (const route of this.routes.values()) {
      if (route.native === null || !held.has(route)) { route.told = null; continue; }
      const told = JSON.stringify([s.playing, s.hoursPerSecond, s.seekSequence, s.playing ? null : s.hour, this.host.frameRate()]);
      if (route.told === told) continue;
      const sought = route.told === null || JSON.parse(route.told)[2] !== s.seekSequence || !s.playing && JSON.parse(route.told)[3] !== s.hour;
      route.told = told;
      const command = this.page.command(route.page);
      const composedAt = performance.timeOrigin + (s.composedAt ?? performance.now());
      const page = route.page;
      route.serial = route.serial.then(async () => {
        if (this.closed || !this.holds(route) || route.page !== page) return;
        let reason: string | null = null;
        let failure: Failure | undefined;
        try {
          const data = await this.ask('follow-clock', { branchId: route.branch, clock: route.offset, hour: s.position + route.offset, playing: s.playing, speed: s.hoursPerSecond,
            frameRate: this.host.frameRate(), seek: s.seekSequence, sought, composedAt, sentAt: performance.timeOrigin + performance.now() }, route) as { available?: boolean; reason?: string } | null;
          if (data?.available !== true) {
            reason = data?.reason ?? 'The followed stream is no longer available';
            failure = asFailure(new Error(reason));
          }
        } catch (error) {
          if (route.page === page) route.told = null;
          failure = asFailure(error);
          reason = failure.message;
        }
        if (!this.closed && reason !== null && this.holds(route) && route.page === page) {
          if (this.page.fail(page, command, reason)) {
            route.error = { seek: this.page.seek, failure: failure! };
            this.flush();
          }
        }
      });
    }
  }
}

interface PipelineSlot {
  pipeline: Pipeline;
  client: DashboardClient;
  host: () => PipelineHost;
  listeners: Set<() => void>;
}

const pipelines = new WeakMap<PlaybackController, WeakMap<DashboardClient, PipelineSlot>>();
const byClock = new WeakMap<PlaybackController, PipelineSlot>();
const clockGeneration = new WeakMap<PlaybackController, number>();
const clockListeners = new WeakMap<PlaybackController, Set<() => void>>();
const ownerOf = new WeakMap<Pipeline, { clock: PlaybackController; slot: PipelineSlot }>();

export const pipelineGeneration = (clock: PlaybackController) => clockGeneration.get(clock) ?? 1;

export function pipelineFor(clock: PlaybackController, client: DashboardClient, host: () => PipelineHost): Pipeline {
  let byClient = pipelines.get(clock);
  if (!byClient) pipelines.set(clock, byClient = new WeakMap());
  let slot = byClient.get(client);
  if (!slot) {
    const existing = byClock.get(clock);
    const generation = existing ? (clockGeneration.get(clock) ?? 1) + 1 : (clockGeneration.get(clock) ?? 1);
    clockGeneration.set(clock, generation);
    const pipeline = new Pipeline(clock, client, host(), generation);
    slot = { pipeline, client, host, listeners: new Set() };
    byClient.set(client, slot);
    ownerOf.set(pipeline, { clock, slot });
  }
  byClock.set(clock, slot);
  return slot.pipeline;
}

export function subscribeClockPipeline(clock: PlaybackController, listener: () => void): () => void {
  let set = clockListeners.get(clock);
  if (!set) clockListeners.set(clock, set = new Set());
  set.add(listener);
  return () => { set.delete(listener); };
}

export function restartPipeline(pipeline: Pipeline): Pipeline | null {
  const owned = ownerOf.get(pipeline);
  if (!owned) return null;
  const { clock, slot } = owned;
  if (slot.pipeline !== pipeline) return slot.pipeline;
  pipeline.close();
  const generation = (clockGeneration.get(clock) ?? 0) + 1;
  clockGeneration.set(clock, generation);
  const next = new Pipeline(clock, slot.client, slot.host(), generation);
  slot.pipeline = next;
  byClock.set(clock, slot);
  ownerOf.set(next, { clock, slot });
  for (const listener of [...slot.listeners]) listener();
  for (const listener of [...clockListeners.get(clock) ?? []]) listener();
  return next;
}
