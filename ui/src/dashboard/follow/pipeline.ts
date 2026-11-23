import { flushSync } from 'react-dom';
import { engine } from '../../engine/frame.ts';
import type { DashboardClient, DashboardSource } from '../client';
import type { PlaybackController, PlaybackSnapshot } from '../playback';
import { canonical } from './canonical.ts';
import { Page, type PageMessage, type PageSurface, type RebindChange, type WidgetView } from './page.ts';
import type { Channel } from './facts';
import type { Binding } from '../../draw/lane';
import type { FromLane, ToLane } from '../../draw/laneMessages';
import { auditing, noteFrame, recordPicture } from '../pulled.ts';
import { perfLog } from '../../shell/perf.ts';
import type { AuditRow } from '../../draw/notifications';
import { productRequest } from '../request.ts';
import type { GatedParams } from '../regionTime';

export type FollowQuery = Readonly<Record<string, unknown>>;

export interface LaneWorker { postMessage(message: ToLane, transfer?: Transferable[]): void; onmessage: ((event: MessageEvent<FromLane>) => void) | null }

export interface PipelineHost {
  lanes: readonly LaneWorker[];
  open(ports: readonly MessagePort[]): Promise<string | void>;
  channel(): { port1: MessagePort; port2: MessagePort };
  frameRate(): number;
  rate(): number;
}

export type SubStatus = 'pending' | 'followed' | 'refused';

interface SubState {
  id: number; key: string; lane: number; channel: Channel; route: RouteState; holders: number; status: SubStatus; reason?: string;
  widgets: Set<number>; released: boolean;
}

interface RouteState { page: number; native: number | null; key: string; source: DashboardSource; branch: string; offset: number; serial: Promise<unknown>; told: string | null }

interface WidgetState {
  id: number; lane: number; subs: Map<object, number>; view: string; model?: unknown; mounted: boolean; route: RouteState | null;
  installListeners: Set<() => void>; dateListeners: Set<() => void>; binding: Binding; surfaces: readonly PageSurface<ImageBitmap>[];
  held: Map<string, unknown>;
}

export interface MountOptions { offset: number; pin: number | null; bounds?: readonly [number, number] | null; view?: string; model?: unknown; key?: string; widget?: string }

const NO_CHANNEL = 'This connection has no channel for pushed answers';

export class Pipeline {
  readonly page: Page<ImageBitmap>;
  private readonly host: PipelineHost;
  private readonly clock: PlaybackController;
  private readonly client: DashboardClient;
  private readonly subs = new Map<string, SubState>();
  private readonly byId = new Map<number, SubState>();
  private readonly routes = new Map<string, RouteState>();
  private readonly widgets = new Map<number, WidgetState>();
  private readonly loads: number[];
  private readonly placed = new Map<string, number>();
  private readonly nextSub: number[];
  private nextWidget = 1;
  private nextRoute = 1;
  private opened: Promise<void> | null = null;
  private readonly ports: readonly MessagePort[];
  private changed = new Set<WidgetState>();
  private redated = new Set<WidgetState>();
  private dated = false;
  private framing = false;
  private stopPublish: (() => void) | null = null;
  private stopClock: (() => void) | null = null;
  private rate: number;

  constructor(clock: PlaybackController, client: DashboardClient, host: PipelineHost) {
    this.clock = clock; this.client = client; this.host = host; this.rate = host.rate();
    const count = host.lanes.length;
    perfLog?.lanes(count);
    this.loads = Array.from({ length: count }, () => 0);
    this.nextSub = Array.from({ length: count }, () => 1);
    this.page = new Page<ImageBitmap>({ lanes: count, seek: clock.getSnapshot().seekSequence, post: (lane, message) => this.post(lane, message) });
    host.lanes.forEach((lane, index) => { lane.onmessage = ({ data }) => this.receive(index, data); });
    const channels = host.lanes.map(() => host.channel());
    channels.forEach((channel, lane) => host.lanes[lane]!.postMessage({ type: 'init', lane, lanes: count, port: channel.port1, rate: host.rate() }, [channel.port1]));
    this.ports = channels.map(channel => channel.port2);
    this.stopPublish = clock.onPublish((previous, next) => this.published(previous, next), true);
    this.stopClock = clock.subscribe(() => this.anchor());
    const s = clock.getSnapshot();
    this.page.publish(s.hour, { playing: s.playing, speed: s.hoursPerSecond, step: 0 });
  }

  get lanes() { return this.host.lanes.length; }

  close() { this.stopPublish?.(); this.stopClock?.(); for (const port of this.ports) port.close(); }

  private post(lane: number | 'all', message: PageMessage) {
    if (lane === 'all') for (const worker of this.host.lanes) worker.postMessage(message);
    else this.host.lanes[lane]!.postMessage(message);
  }

  private open(): Promise<void> {
    this.opened ??= this.host.open(this.ports).then(session => { if (typeof session === 'string') this.session = session; });
    return this.opened;
  }

  private session: string | undefined;

  private holds(route: RouteState) {
    for (const sub of this.subs.values()) if (sub.route === route && sub.status === 'followed' && sub.channel !== 'pinned') return true;
    return false;
  }

  private published(previous: PlaybackSnapshot, next: PlaybackSnapshot) {
    const rate = this.host.rate();
    if (rate !== this.rate) { this.rate = rate; for (const lane of this.host.lanes) lane.postMessage({ type: 'options', rate }); }
    const step = next.playing && previous.playing && next.seekSequence === previous.seekSequence ? Math.max(0, next.hour - previous.hour) : this.page.step;
    this.page.publish(next.hour, { playing: next.playing, seek: next.seekSequence, step, speed: next.hoursPerSecond });
    this.dated = true;
    this.flush();
    if (next.playing) noteFrame(performance.now());
    if (auditing()) this.sample(next.seekSequence);
  }

  private readonly audits = new Map<number, Map<number, (fields: Readonly<Record<string, unknown>>) => readonly AuditRow[] | undefined>>();

  audit(id: number, owner: number, rows: ((fields: Readonly<Record<string, unknown>>) => readonly AuditRow[] | undefined) | null) {
    let owners = this.audits.get(id);
    if (rows) { if (!owners) this.audits.set(id, owners = new Map()); owners.set(owner, rows); }
    else { owners?.delete(owner); if (owners && !owners.size) this.audits.delete(id); }
  }

  private readonly answered = new Map<number, Map<number, { hour: number; entry: number; rows: () => readonly AuditRow[] | undefined }>>();
  private answers = 0;

  answer(id: number, owner: number, shown: { hour: number; rows: () => readonly AuditRow[] | undefined } | null) {
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
        recordPicture({ t: 'picture', at, shownAt: at, seek, widget: name, surface, placement: held.id, owner, renderer: 'page', head: clock(view.target), hour, offset, fixedHour: hour,
          since: hour, through: hour, eligibleHour: hour, entry: answer.entry, parts: [], drawMs: 0, ...rows ? { rows } : {},
          mode: 'picture', dim: false, reset: 0, binding: shown.binding, inspect: shown.inspect, tooltip: false });
      }
      const hour = view.mode === 'picture' && view.through !== undefined ? clock(view.through) : null;
      const owners = [...this.audits.get(held.id) ?? []];
      if (!owners.length && answers.length) continue;
      for (const [owner, audit] of owners.length ? owners : [[0, undefined] as const]) {
      const rows = view.mode === 'picture' && view.fields ? audit?.(view.fields) : undefined;
      recordPicture({ t: 'picture', at, shownAt: at, seek, widget: name,
        surface, placement: held.id, owner, renderer: 'page',
        head: clock(view.target), hour, offset, ...shown.pin !== null ? { fixedHour: clock(shown.pin) } : {},
        ...view.h !== undefined ? { since: clock(view.h) } : {}, ...shown.installed ? { through: clock(shown.installed.c) } : {},
        eligibleHour: shown.installed && view.mode === 'picture' ? clock(Math.min(view.target, shown.installed.c)) : null,
        ...shown.installed ? { entry: shown.installed.id, parts: Object.values(shown.content),
          face: Object.entries(shown.content).filter(([part]) => part !== 'hour').map(([part, id]) => `${part}=${id}`).join('|') } : {},
        drawMs: shown.installed?.ms ?? 0, ...rows ? { rows } : {},
        mode: view.mode, ...view.reason !== undefined ? { reason: view.reason } : {}, dim: view.dim, reset: shown.installed?.gen.reset ?? 0, binding: shown.installed?.binding ?? shown.binding,
        inspect: shown.inspect, tooltip: view.tooltip !== null });
      }
    }
  }

  private flush() {
    const changed = this.changed, dated = this.dated, redated = this.redated;
    this.changed = new Set(); this.redated = new Set();
    this.dated = false;
    if (changed.size) flushSync(() => { for (const widget of changed) for (const listener of [...widget.installListeners]) listener(); });
    for (const widget of dated ? this.widgets.values() : redated) for (const listener of [...widget.dateListeners]) listener();
  }

  private receive(lane: number, message: FromLane) {
    if (message.type === 'failure') {
      const routes = new Set<number>();
      for (const widget of this.widgets.values()) if (message.lane === -1 || widget.lane === lane) routes.add(this.page.widgets.get(widget.id)!.route);
      for (const route of routes) this.page.fail(route, this.page.command(route), message.reason);
      this.flush();
      return;
    }
    if (message.type === 'hit') {
      const told = this.hits.get(message.widget)?.shift();
      this.page.inspect(message.widget, message.key); this.flush();
      told?.(message.key);
      return;
    }
    if (message.type === 'served') {
      if (auditing()) recordPicture({ t: 'workerFrame', at: performance.timeOrigin + performance.now(), lane, lanes: this.lanes, seek: message.seek, head: message.target,
        playing: !message.exact && this.page.playing, changed: message.changed, workMs: message.ms });
      return;
    }
    this.page.offer(message);
    if (!this.page.playing) this.commitSoon();
  }

  private commitSoon() {
    if (this.framing) return;
    this.framing = true;
    engine.next('commit', () => { this.framing = false; this.page.frame(); this.flush(); if (auditing()) this.sample(this.page.seek); });
  }

  private place(key: string | undefined) {
    const average = this.loads.reduce((sum, load) => sum + load, 0) / this.loads.length;
    const held = key === undefined ? undefined : this.placed.get(key);
    if (held !== undefined && this.loads[held]! <= average + 1) return held;
    let lane = 0;
    for (let at = 1; at < this.loads.length; at++) if (this.loads[at]! < this.loads[lane]!) lane = at;
    if (key !== undefined) this.placed.set(key, lane);
    return lane;
  }

  mount(options: MountOptions) {
    const lane = this.place(options.key), id = this.nextWidget++;
    this.loads[lane]!++;
    const widget: WidgetState = { id, lane, subs: new Map(), view: options.view ?? 'fields', model: options.model, mounted: false, route: null,
      installListeners: new Set(), dateListeners: new Set(), binding: { generation: 1, surfaces: [], offset: options.offset, pin: options.pin }, surfaces: [], held: new Map() };
    this.widgets.set(id, widget);
    this.page.mount(id, { route: 0, lane, offset: options.offset, pin: options.pin, bounds: options.bounds, surfaces: [], changed: () => this.changed.add(widget),
      dated: () => this.redated.add(widget) });
    return id;
  }

  private readonly shared = new Map<string, { id: number; holders: number; widget: string }>();

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
    const widget = this.widgets.get(id);
    if (!widget) return;
    this.audits.delete(id);
    this.hits.delete(id);
    for (const sub of widget.subs.values()) this.release(sub, id);
    this.widgets.delete(id);
    this.loads[widget.lane]!--;
    this.page.unmount(id);
    this.host.lanes[widget.lane]!.postMessage({ type: 'unmount', widget: id });
  }

  lane(id: number) { return this.widgets.get(id)?.lane ?? 0; }

  status(sub: number): { status: SubStatus; reason?: string } {
    const held = this.byId.get(sub);
    return held ? { status: held.status, ...held.reason !== undefined ? { reason: held.reason } : {} } : { status: 'pending' };
  }

  view(id: number): WidgetView | null { return this.widgets.has(id) ? this.page.view(id) : null; }

  onInstall(id: number, listener: () => void) {
    const widget = this.widgets.get(id);
    widget?.installListeners.add(listener);
    return () => { widget?.installListeners.delete(listener); };
  }

  onDate(id: number, listener: () => void) {
    const widget = this.widgets.get(id);
    widget?.dateListeners.add(listener);
    return () => { widget?.dateListeners.delete(listener); };
  }

  private routeOf(source: DashboardSource, branch: string, offset: number): RouteState {
    const key = JSON.stringify([source.id, source.revision, branch, offset]);
    let route = this.routes.get(key);
    if (!route) this.routes.set(key, route = { page: this.nextRoute++, native: null, key, source, branch, offset, serial: Promise.resolve(), told: null });
    return route;
  }

  subscribe(widgetId: number, token: object, source: DashboardSource, branch: string, offset: number, query: FollowQuery, channel: Channel, product?: string) {
    const widget = this.widgets.get(widgetId);
    if (!widget) return;
    const route = this.routeOf(source, branch, offset);
    const key = JSON.stringify([widget.lane, route.key, channel, canonical(query)]);
    let sub = this.subs.get(key);
    if (!sub) {
      const id = this.nextSub[widget.lane]!++ * this.lanes + widget.lane;
      sub = { id, key, lane: widget.lane, channel, route, holders: 0, status: 'pending', widgets: new Set(), released: false };
      this.subs.set(key, sub); this.byId.set(id, sub);
      this.register(sub, query, product);
    }
    sub.holders++;
    sub.widgets.add(widgetId);
    const was = widget.subs.get(token);
    widget.subs.set(token, sub.id);
    if (was !== undefined) this.release(was, widgetId);
    widget.route = route;
    this.remount(widget);
    return sub.id;
  }

  unsubscribe(widgetId: number, token: object) {
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
    sub.widgets.delete(widget);
    if (--sub.holders > 0) return;
    this.subs.delete(sub.key); this.byId.delete(id);
    sub.released = true;
    this.host.lanes[sub.lane]!.postMessage({ type: 'unfollow', sub: id });
    if (sub.channel !== 'pinned' && sub.status === 'followed') void this.ask('unfollow', { id }, sub.route).catch(() => {});
    if (!this.holds(sub.route)) { sub.route.told = null; const page = this.nextRoute++; this.page.carry(sub.route.page, page); sub.route.page = page; }
  }

  private remount(widget: WidgetState) {
    const subs = [...new Set(widget.subs.values())];
    this.host.lanes[widget.lane]!.postMessage({ type: 'mount', widget: widget.id, subs, view: widget.view, model: widget.model });
    this.page.rebind(widget.id, { subscription: true });
    const followed = subs.map(id => this.byId.get(id)).find(sub => sub?.status === 'followed');
    if (followed) this.page.mountRoute(widget.id, followed.route.page);
    widget.binding = this.timed(widget, widget.binding);
    this.host.lanes[widget.lane]!.postMessage({ type: 'bind', widget: widget.id, binding: widget.binding });
    widget.mounted = true;
    const refused = subs.map(id => this.byId.get(id)).find(sub => sub?.status === 'refused');
    if (refused) this.page.name(widget.id, 'refused', refused.reason);
    this.flush();
  }

  private timed(widget: WidgetState, binding: Omit<Binding, 'generation' | 'offset' | 'pin'>): Binding {
    const shown = this.page.widgets.get(widget.id);
    return { ...binding, generation: shown?.binding ?? 1, offset: shown?.offset ?? 0, pin: shown?.pin ?? null };
  }

  bind(id: number, surfaces: readonly PageSurface<ImageBitmap>[], binding: Omit<Binding, 'generation' | 'offset' | 'pin'>, change: RebindChange = {}, model?: unknown) {
    const widget = this.widgets.get(id);
    if (!widget) return;
    const same = surfaces.length === widget.surfaces.length && surfaces.every((surface, at) => surface === widget.surfaces[at]);
    if (!same) { widget.surfaces = surfaces; this.page.remount(id, surfaces); }
    else this.page.rebind(id, change);
    if (model !== undefined) widget.model = model;
    widget.binding = this.timed(widget, binding);
    this.host.lanes[widget.lane]!.postMessage({ type: 'bind', widget: id, binding: widget.binding, ...model !== undefined ? { model } : {} });
    this.flush();
  }

  present(id: number, view: string, model?: unknown) {
    const widget = this.widgets.get(id);
    if (!widget || widget.view === view && model === undefined) return;
    widget.view = view;
    if (model !== undefined) widget.model = model;
    this.host.lanes[widget.lane]!.postMessage({ type: 'mount', widget: id, subs: [...new Set(widget.subs.values())], view, model: widget.model });
    this.page.rebind(id);
    widget.binding = this.timed(widget, widget.binding);
    this.host.lanes[widget.lane]!.postMessage({ type: 'bind', widget: id, binding: widget.binding });
    this.flush();
  }

  held<T>(id: number, kind: string, make: () => T): T | null {
    const widget = this.widgets.get(id);
    if (!widget) return null;
    if (!widget.held.has(kind)) widget.held.set(kind, make());
    return widget.held.get(kind) as T;
  }

  retime(id: number, change: RebindChange) {
    const widget = this.widgets.get(id);
    if (!widget) return;
    this.page.rebind(id, change);
    widget.binding = this.timed(widget, widget.binding);
    this.host.lanes[widget.lane]!.postMessage({ type: 'bind', widget: id, binding: widget.binding });
    this.flush();
  }

  inspect(id: number, key: string | null) { this.page.inspect(id, key); this.flush(); }

  inspecting(id: number) { return this.page.widgets.get(id)?.inspect ?? null; }

  private readonly hits = new Map<number, (((key: string | null) => void) | undefined)[]>();

  pointer(id: number, surface: number, point: [number, number] | null, hit?: (key: string | null) => void) {
    const widget = this.widgets.get(id);
    if (!widget) return;
    let waiting = this.hits.get(id);
    if (!waiting) this.hits.set(id, waiting = []);
    waiting.push(hit);
    this.host.lanes[widget.lane]!.postMessage({ type: 'pointer', widget: id, surface, point });
  }

  exact(sub: number, hour: number, data: unknown, refused: string | null) {
    const held = this.byId.get(sub);
    if (held) this.host.lanes[held.lane]!.postMessage({ type: 'exact', sub, hour, data, refused });
  }

  private ask(product: 'follow' | 'unfollow' | 'follow-clock', params: Record<string, unknown>, route: RouteState) {
    const sessioned = this.session ? { ...params, session: this.session } : params;
    return this.client.product(productRequest({ sourceId: route.source.id, revision: route.source.revision, product, params: sessioned as GatedParams }),
      new AbortController().signal).then(reply => reply.data);
  }

  private register(sub: SubState, query: FollowQuery, product?: string) {
    this.host.lanes[sub.lane]!.postMessage({ type: 'follow', sub: sub.id, channel: sub.channel, ...product !== undefined ? { product } : {} });
    if (sub.channel === 'pinned') { sub.status = 'followed'; return; }
    if (!this.client.openFollow) { this.refuse(sub, NO_CHANNEL); return; }
    void this.open().then(() => sub.released ? null
      : this.ask('follow', { id: sub.id, query, branchId: sub.route.branch, clock: sub.route.offset, lane: sub.lane, lanes: this.lanes }, sub.route))
      .then(reply => {
        const data = reply as { available?: boolean; reason?: string; route?: number } | null;
        if (sub.released) { if (data?.available === true) void this.ask('unfollow', { id: sub.id }, sub.route).catch(() => {}); return; }
        if (data?.available !== true || !Number.isInteger(data.route)) { this.refuse(sub, data?.reason ?? 'The service did not say whether it follows this'); return; }
        const first = !this.holds(sub.route), moved = sub.route.native !== null && sub.route.native !== data.route;
        if (moved) sub.route.page = this.nextRoute++;
        sub.status = 'followed';
        sub.route.native = data.route!;
        this.host.lanes[sub.lane]!.postMessage({ type: 'route', sub: sub.id, route: sub.route.page, native: data.route!, seek: this.page.seek, offset: sub.route.offset });
        for (const widget of sub.widgets) this.page.mountRoute(widget, sub.route.page);
        if (first || moved) sub.route.told = null;
        this.anchor();
      }, error => this.refuse(sub, error instanceof Error ? error.message : String(error)));
  }

  private refuse(sub: SubState, reason: string) {
    sub.status = 'refused'; sub.reason = reason;
    for (const widget of sub.widgets) this.page.name(widget, 'refused', reason);
    this.flush();
  }

  private anchor() {
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
        if (!this.holds(route) || route.page !== page) return;
        let reason: string | null = null;
        try {
          const data = await this.ask('follow-clock', { branchId: route.branch, clock: route.offset, hour: s.position + route.offset, playing: s.playing, speed: s.hoursPerSecond,
            frameRate: this.host.frameRate(), seek: s.seekSequence, sought, composedAt, sentAt: performance.timeOrigin + performance.now() }, route) as { available?: boolean; reason?: string } | null;
          if (data?.available !== true) reason = data?.reason ?? 'The followed stream is no longer available';
        } catch (error) { if (route.page === page) route.told = null; reason = error instanceof Error ? error.message : String(error); }
        if (reason !== null && this.holds(route) && route.page === page && this.page.fail(page, command, reason)) this.flush();
      });
    }
  }
}

const pipelines = new WeakMap<PlaybackController, WeakMap<DashboardClient, Pipeline>>();
const byClock = new WeakMap<PlaybackController, Pipeline>();

export const pipelineOf = (clock: PlaybackController) => byClock.get(clock) ?? null;

export function pipelineFor(clock: PlaybackController, client: DashboardClient, host: () => PipelineHost): Pipeline {
  let byClient = pipelines.get(clock);
  if (!byClient) pipelines.set(clock, byClient = new WeakMap());
  let pipeline = byClient.get(client);
  if (!pipeline) byClient.set(client, pipeline = new Pipeline(clock, client, host()));
  byClock.set(clock, pipeline);
  return pipeline;
}
