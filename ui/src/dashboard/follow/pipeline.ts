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
import type { AuditRow } from '../../draw/notifications';

export type FollowQuery = Readonly<Record<string, unknown>>;

export interface LaneWorker { postMessage(message: ToLane, transfer?: Transferable[]): void; onmessage: ((event: MessageEvent<FromLane>) => void) | null }

export interface PipelineHost {
  lanes: readonly LaneWorker[];
  open(ports: readonly MessagePort[]): Promise<void>;
  channel(): { port1: MessagePort; port2: MessagePort };
  frameRate(): number;
  rate(): number;
}

export type SubStatus = 'pending' | 'followed' | 'refused';

interface SubState {
  id: number; key: string; lane: number; channel: Channel; route: RouteState; holders: number; status: SubStatus; reason?: string;
  rank: number; widgets: Set<number>; window: RankWindow | null; told: string | null;
}

export interface RankWindow { from: number; count: number; countries: readonly string[]; most?: number }

const EVERY: RankWindow = { from: 0, count: Number.MAX_SAFE_INTEGER, countries: [] };

interface RouteState { page: number; native: number | null; key: string; source: DashboardSource; branch: string; offset: number; serial: Promise<unknown>; told: string | null }

interface WidgetState {
  id: number; lane: number; subs: Map<object, number>; view: string; model?: unknown; mounted: boolean; route: RouteState | null;
  installListeners: Set<() => void>; dateListeners: Set<() => void>; binding: Binding; surfaces: readonly PageSurface<ImageBitmap>[];
  failure?: string; held: Map<string, unknown>;
}

export interface MountOptions { offset: number; pin: number | null; bounds?: readonly [number, number] | null; view?: string; model?: unknown; key?: string }

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

  close() { this.stopPublish?.(); this.stopClock?.(); }

  private post(lane: number | 'all', message: PageMessage) {
    if (lane === 'all') for (const worker of this.host.lanes) worker.postMessage(message);
    else this.host.lanes[lane]!.postMessage(message);
  }

  private open(): Promise<void> {
    this.opened ??= this.host.open(this.ports);
    return this.opened;
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

  private readonly audits = new Map<number, (fields: Readonly<Record<string, unknown>>) => readonly AuditRow[] | undefined>();

  audit(id: number, rows: ((fields: Readonly<Record<string, unknown>>) => readonly AuditRow[] | undefined) | null) {
    if (rows) this.audits.set(id, rows); else this.audits.delete(id);
  }

  private sample(seek: number) {
    const at = performance.timeOrigin + performance.now();
    for (const [key, held] of this.shared) {
      const view = this.page.view(held.id), widget = this.widgets.get(held.id), shown = this.page.widgets.get(held.id);
      if (!widget || !shown) continue;
      const offset = shown.offset, clock = (hour: number) => hour - offset;
      const hour = view.mode === 'picture' && view.through !== undefined ? clock(view.through) : null;
      const rows = view.mode === 'picture' && view.fields ? this.audits.get(held.id)?.(view.fields) : undefined;
      recordPicture({ t: 'picture', at, shownAt: at, seek, widget: key.includes('|') ? key.slice(0, key.lastIndexOf('|')) : key,
        surface: widget.view === 'table' || widget.view === 'race' ? widget.view : 'page', placement: held.id, owner: 0, renderer: 'page',
        head: clock(view.target), hour, offset, ...shown.pin !== null ? { fixedHour: clock(shown.pin) } : {},
        ...view.h !== undefined ? { since: clock(view.h) } : {}, ...shown.installed ? { through: clock(shown.installed.c) } : {}, eligibleHour: hour,
        drawMs: shown.installed?.ms ?? 0, ...rows ? { rows } : {},
        mode: view.mode, ...view.reason !== undefined ? { reason: view.reason } : {}, dim: view.dim, reset: shown.installed?.gen.reset ?? 0, binding: shown.binding,
        inspect: shown.inspect, tooltip: view.tooltip !== null });
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
      for (const widget of this.widgets.values()) if (message.lane === -1 || widget.lane === lane) this.page.name(widget.id, 'failed', message.reason);
      this.flush();
      return;
    }
    if (message.type === 'hit') { this.page.inspect(message.widget, message.key); this.flush(); return; }
    if (message.type === 'served') {
      if (auditing()) recordPicture({ t: 'workerFrame', at: performance.timeOrigin + performance.now(), lane, seek: message.seek, head: message.target,
        playing: !message.exact && this.page.playing, changed: message.changed, workMs: message.ms });
      return;
    }
    if (message.type === 'needs') { for (const need of message.needs) this.needs(message.widget, need.sub, need.countries); this.flush(); return; }
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

  private readonly shared = new Map<string, { id: number; holders: number }>();

  acquire(key: string, options: MountOptions): number {
    let held = this.shared.get(key);
    if (!held) this.shared.set(key, held = { id: this.mount({ ...options, key: options.key ?? key }), holders: 0 });
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

  failure(id: number) { return this.widgets.get(id)?.failure; }

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

  subscribe(widgetId: number, token: object, source: DashboardSource, branch: string, offset: number, query: FollowQuery, channel: Channel, product?: string,
    window?: RankWindow) {
    const widget = this.widgets.get(widgetId);
    if (!widget) return;
    const route = this.routeOf(source, branch, offset);
    const key = JSON.stringify([widget.lane, route.key, channel, canonical(query), window ? widgetId : null]);
    let sub = this.subs.get(key);
    if (!sub) {
      const id = this.nextSub[widget.lane]!++ * this.lanes + widget.lane;
      sub = { id, key, lane: widget.lane, channel, route, holders: 0, status: 'pending', rank: 0, widgets: new Set(), window: null, told: null };
      this.subs.set(key, sub); this.byId.set(id, sub);
      this.register(sub, query, product);
      if (window) this.setWindow(sub, window);
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
    this.host.lanes[sub.lane]!.postMessage({ type: 'unfollow', sub: id });
    if (sub.channel !== 'pinned') void this.ask('unfollow', { id }).catch(() => {});
  }

  private remount(widget: WidgetState) {
    const subs = [...new Set(widget.subs.values())];
    this.host.lanes[widget.lane]!.postMessage({ type: 'mount', widget: widget.id, subs, view: widget.view, model: widget.model });
    this.page.rebind(widget.id, { subscription: true, rank: this.rankOf(widget) });
    const followed = subs.map(id => this.byId.get(id)).find(sub => sub?.status === 'followed');
    if (followed) this.page.mountRoute(widget.id, followed.route.page);
    widget.binding = this.timed(widget, widget.binding);
    this.host.lanes[widget.lane]!.postMessage({ type: 'bind', widget: widget.id, binding: widget.binding });
    widget.mounted = true;
    const refused = subs.map(id => this.byId.get(id)).find(sub => sub?.status === 'refused');
    if (refused) this.page.name(widget.id, 'failed', refused.reason);
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

  pointer(id: number, surface: number, point: [number, number] | null) {
    const widget = this.widgets.get(id);
    if (widget) this.host.lanes[widget.lane]!.postMessage({ type: 'pointer', widget: id, surface, point });
  }

  exact(sub: number, hour: number, data: unknown, refused: string | null) {
    const held = this.byId.get(sub);
    if (held) this.host.lanes[held.lane]!.postMessage({ type: 'exact', sub, hour, data, refused });
  }

  window(widget: number, sub: number, window: RankWindow | null) {
    const held = this.byId.get(sub);
    if (!held || !held.widgets.has(widget) || held.widgets.size > 1) return;
    if (JSON.stringify(held.window) === JSON.stringify(window)) return;
    this.setWindow(held, window ?? EVERY);
    for (const id of held.widgets) { const owner = this.widgets.get(id); if (owner) this.retime(id, { rank: this.rankOf(owner) }); }
    if (held.status === 'followed') this.tellWindow(held);
  }

  needs(widget: number, sub: number, countries: readonly string[]) {
    const held = this.byId.get(sub);
    if (!held?.window) return;
    const now = held.window, grown = [...new Set([...now.countries, ...countries])].sort();
    this.window(widget, sub, { ...now, countries: grown.length > (now.most ?? Infinity) ? [...new Set(countries)].sort() : grown });
  }

  private setWindow(sub: SubState, window: RankWindow) {
    sub.window = window;
    sub.rank = (sub.rank + 1) & 0xffff || 1;
    const filters = window !== EVERY && window.count === 0;
    this.host.lanes[sub.lane]!.postMessage({ type: 'rank', sub: sub.id, version: sub.rank, ...filters ? { countries: window.countries } : {} });
  }

  private rankOf(widget: WidgetState) {
    let rank = 0;
    for (const id of widget.subs.values()) rank = Math.max(rank, this.byId.get(id)?.rank ?? 0);
    return rank;
  }

  private windowParams(sub: SubState) {
    const window = sub.window;
    return window && { from: window.from, count: window.count, countries: window.countries, version: sub.rank, ...window === EVERY ? { every: true } : {} };
  }

  private tellWindow(sub: SubState) {
    const params = this.windowParams(sub), text = JSON.stringify(params);
    if (!params || sub.told === text) return;
    sub.told = text;
    void this.ask('follow-window', { id: sub.id, ...params }, sub.route).then(reply => {
      const data = reply as { available?: boolean; reason?: string } | null;
      if (data?.available !== true) for (const widget of sub.widgets) this.page.name(widget, 'failed', data?.reason ?? 'The ranked window was not moved');
      this.flush();
    }, error => { for (const widget of sub.widgets) this.page.name(widget, 'failed', error instanceof Error ? error.message : String(error)); this.flush(); });
  }

  private ask(product: string, params: Record<string, unknown>, route?: RouteState) {
    const source = route?.source ?? this.routes.values().next().value?.source;
    if (!source) return Promise.resolve(null);
    return this.client.product({ sourceId: source.id, revision: source.revision, product, params }, new AbortController().signal).then(reply => reply.data);
  }

  private register(sub: SubState, query: FollowQuery, product?: string) {
    this.host.lanes[sub.lane]!.postMessage({ type: 'follow', sub: sub.id, channel: sub.channel, ...product !== undefined ? { product } : {} });
    if (sub.channel === 'pinned') { sub.status = 'followed'; return; }
    if (!this.client.openFollow) { this.refuse(sub, NO_CHANNEL); return; }
    void this.open().then(() => {
      sub.told = JSON.stringify(this.windowParams(sub));
      return this.ask('follow', { id: sub.id, query, branchId: sub.route.branch, clock: sub.route.offset, lane: sub.lane, lanes: this.lanes, window: this.windowParams(sub) }, sub.route);
    })
      .then(reply => {
        const data = reply as { available?: boolean; reason?: string; route?: number } | null;
        if (this.byId.get(sub.id) !== sub) return;
        if (data?.available !== true || !Number.isInteger(data.route)) { this.refuse(sub, data?.reason ?? 'The service did not say whether it follows this'); return; }
        sub.status = 'followed';
        sub.route.native = data.route!;
        this.host.lanes[sub.lane]!.postMessage({ type: 'route', sub: sub.id, route: sub.route.page, native: data.route!, seek: this.page.seek, offset: sub.route.offset });
        for (const widget of sub.widgets) this.page.mountRoute(widget, sub.route.page);
        sub.route.told = null;
        this.tellWindow(sub);
        this.anchor();
      }, error => this.refuse(sub, error instanceof Error ? error.message : String(error)));
  }

  private refuse(sub: SubState, reason: string) {
    sub.status = 'refused'; sub.reason = reason;
    for (const widget of sub.widgets) this.page.name(widget, 'failed', reason);
    this.flush();
  }

  private anchor() {
    const s = this.clock.getSnapshot();
    for (const route of this.routes.values()) {
      if (route.native === null) continue;
      const told = JSON.stringify([s.playing, s.hoursPerSecond, s.seekSequence, s.playing ? null : s.hour, this.host.frameRate()]);
      if (route.told === told) continue;
      const sought = route.told === null || JSON.parse(route.told)[2] !== s.seekSequence || !s.playing && JSON.parse(route.told)[3] !== s.hour;
      route.told = told;
      const command = this.page.command(route.page);
      const composedAt = performance.timeOrigin + (s.composedAt ?? performance.now());
      route.serial = route.serial.then(() => this.ask('follow-clock', { branchId: route.branch, clock: route.offset, hour: s.position + route.offset,
        playing: s.playing, speed: s.hoursPerSecond, frameRate: this.host.frameRate(), seek: s.seekSequence, sought, composedAt, sentAt: performance.timeOrigin + performance.now() }, route))
        .then(reply => {
          const data = reply as { available?: boolean; reason?: string } | null;
          if (data?.available !== true && this.page.fail(route.page, command, data?.reason ?? 'The followed stream is no longer available')) this.flush();
        }, error => {
          route.told = null;
          if (this.page.fail(route.page, command, error instanceof Error ? error.message : String(error))) this.flush();
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
