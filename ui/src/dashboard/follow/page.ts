import { answerShape } from './message.ts';
import type { InstalledViewport } from '../../families/plan.ts';

export const SAME = 'same' as const;
export type Same = typeof SAME;

export interface Bitmap { readonly width: number; close(): void }

export interface PageSurface<B extends Bitmap = Bitmap> {
  install(bitmap: B): void;
  clear(): void;
}

export type Mode = 'picture' | 'refused' | 'failed';
export type FailureOrigin = 'stream' | 'local';
export type Shown = Mode | 'reading' | 'outside';

export interface Generation { seek: number; reset: number }

export interface Entry<B extends Bitmap = Bitmap> {
  widget: number;
  binding: number;
  owner?: string;
  id: number;
  h: number;
  c: number;
  lo: number;
  hi: number;
  gen: Generation;
  follows: boolean;
  mode: Mode;
  reason?: string;
  open?: 'followed' | 'panel';
  failureOrigin?: FailureOrigin;
  viewport?: InstalledViewport;
  repainted?: boolean;
  surfaces: readonly (B | Same | null)[];
  content: Readonly<Record<string, string>>;
  fields: Readonly<Record<string, unknown>>;
  ms?: number;
  stamp?: number;
}

export interface Presentation<B extends Bitmap = Bitmap> {
  lane: number;
  serial: number;
  resets?: readonly (readonly [route: number, reset: number])[];
  closed?: readonly (readonly [widget: number, lo: number, hi: number])[];
  withdrawn?: readonly (readonly [widget: number, binding: number])[];
  confirmed?: readonly (readonly [route: number, channel: 'followed' | 'panel', hour: number, seek: number, reset: number])[];
  entries: readonly Entry<B>[];
}

export interface Ack { widget: number; binding: number; id: number }
export interface Refusal { widget: number; id: number; reason: string }

export interface FrameRequest {
  type: 'frame';
  serial: number;
  now: number;
  target: number;
  exact: boolean;
  seek: number;
  speed: number;
  installed: Ack[];
  refused: Refusal[];
  inspect: [widget: number, key: string][];
}

export type PageMessage = FrameRequest
  | { type: 'stream'; route: number; seek: number; target: number }
  | { type: 'fail'; route: number; seek: number; command: number; reason: string };

export interface Installed {
  id: number; owner?: string; h: number; c: number; lo: number; hi: number; gen: Generation; binding: number; follows: boolean;
  open?: 'followed' | 'panel';
  viewport?: InstalledViewport;
  freeze?: (limit?: number) => void;
  mode: Mode; reason?: string; failureOrigin?: FailureOrigin; fields: Readonly<Record<string, unknown>>; ms: number; kept: readonly string[]; structure?: string;
}

export interface WidgetView {
  mode: Shown;
  reason?: string;
  failureOrigin?: FailureOrigin;
  target: number;
  id?: number;
  owner?: string;
  installedOwner?: string;
  binding?: number;
  installedBinding?: number;
  gen?: Generation;
  validInterval?: readonly [lo: number, hi: number];
  viewport?: InstalledViewport;
  h?: number;
  through?: number;
  painted?: number;
  structure?: string;
  dim: boolean;
  waiting?: true;
  expired?: boolean;
  fields?: Readonly<Record<string, unknown>>;
  tooltip: unknown;
}

export interface Mount<B extends Bitmap = Bitmap> {
  route: number;
  lane: number;
  owner?: string;
  offset?: number;
  pin?: number | null;
  bounds?: readonly [first: number, last: number] | null;
  surfaces: readonly PageSurface<B>[];
  changed?: () => void;
  dated?: () => void;
}

interface Widget<B extends Bitmap> {
  id: number; route: number; lane: number; owner?: string; offset: number; pin: number | null; bounds: readonly [number, number] | null;
  surfaces: readonly PageSurface<B>[]; binding: number; installed: Installed | null; content: Record<string, string>;
  status: Shown; reason?: string; failureOrigin?: FailureOrigin; inspect: string | null; early: Entry<B> | null; earlyMore: Entry<B>[];
  changed?: () => void; dated?: () => void;
  failed: boolean; face: string; drawn: number; waiting?: boolean;
}

interface RouteState { reset: number; command: number; failed: { seek: number; reason: string } | null }

export interface RebindChange { pin?: number | null; offset?: number; owner?: string; subscription?: boolean; bounds?: readonly [number, number] | null }

const STRUCTURAL = /^(model|refused):|^(rows|failed)$/;
const MAX_EARLY = 4;
const MAX_AHEAD = 4;
const structureOf = (content: Readonly<Record<string, string>>, fields: Readonly<Record<string, unknown>>) => [
  ...Object.entries(content).filter(([key]) => STRUCTURAL.test(key)).map(([key, id]) => `${key}=${id}`),
  ...Object.entries(fields).flatMap(([key, value]) => key.startsWith('answer:') && value instanceof Uint8Array ? [`${key}~${answerShape(value)}`] : []),
].join('|');

const sameContent = (a: Readonly<Record<string, string>>, b: Readonly<Record<string, string>>) => {
  const keys = Object.keys(a).filter(key => key !== 'hour');
  return keys.length === Object.keys(b).filter(key => key !== 'hour').length && keys.every(key => a[key] === b[key]);
};

const closeParts = <B extends Bitmap>(entry: Entry<B>) => {
  for (const part of entry.surfaces) if (part && part !== SAME) part.close();
};

export class Page<B extends Bitmap = Bitmap> {
  T = 0;
  seek = 0;
  playing = false;
  step = 0;
  speed = 0;
  publishes = 0;
  readonly lanes: number;
  readonly ahead: number[];
  readonly stats = { installs: 0, rejected: {} as Record<string, number>, failed: 0, visited: 0 };
  readonly widgets = new Map<number, Widget<B>>();
  private serial = 0;
  private readonly post: (lane: number | 'all', message: PageMessage) => void;
  private readonly sent = new Map<number, number>();
  private readonly targets: number[];
  private readonly acks: Ack[][];
  private readonly refusals: Refusal[][];
  private failures: [widget: number, stamp: number | undefined][] = [];
  private readonly routes = new Map<number, RouteState>();
  private readonly deadLanes = new Map<number, string>();
  private readonly laneConfirmed: Map<number, { followed: { hour: number }; panel: { hour: number } }>[];
  private readonly byRoute = new Map<number, Set<Widget<B>>>();
  private readonly earlyWidgets = new Set<Widget<B>>();
  private readonly inspectedByLane: Set<Widget<B>>[];
  private readonly laneWidgetCounts: number[];
  private readonly bounded = new Set<Widget<B>>();
  private safeLo = -Infinity;
  private safeHi = Infinity;
  private safeDirty = false;
  private readonly kept = new Map<string, { value: unknown; holders: number }>();
  private readonly holding = new WeakMap<Entry<B>, string[]>();
  private readonly earlyClosed = new WeakMap<Entry<B>, number>();
  private queue: Presentation<B>[] = [];
  private touched = new Set<Widget<B>>();
  private redated = new Set<Widget<B>>();
  private pictures = 0;

  constructor({ lanes = 1, post = () => {}, seek = 0 }: { lanes?: number; post?: (lane: number | 'all', message: PageMessage) => void; seek?: number } = {}) {
    this.lanes = lanes; this.post = post; this.seek = seek;
    this.ahead = Array.from({ length: lanes }, () => 1);
    this.targets = Array.from({ length: lanes }, () => -Infinity);
    this.acks = Array.from({ length: lanes }, () => []);
    this.refusals = Array.from({ length: lanes }, () => []);
    this.laneConfirmed = Array.from({ length: lanes }, () => new Map());
    this.inspectedByLane = Array.from({ length: lanes }, () => new Set<Widget<B>>());
    this.laneWidgetCounts = Array.from({ length: lanes }, () => 0);
  }

  private confirmedBox(lane: number, route: number, channel: 'followed' | 'panel'): { hour: number } {
    const map = this.laneConfirmed[lane]!;
    let held = map.get(route);
    if (!held) map.set(route, held = { followed: { hour: -Infinity }, panel: { hour: -Infinity } });
    return held[channel];
  }

  private route(id: number): RouteState {
    let route = this.routes.get(id);
    if (!route) this.routes.set(id, route = { reset: 0, command: 0, failed: null });
    return route;
  }

  private routeSet(id: number): Set<Widget<B>> {
    let set = this.byRoute.get(id);
    if (!set) this.byRoute.set(id, set = new Set());
    return set;
  }

  private trackBounds(widget: Widget<B>) {
    if (widget.bounds && widget.pin === null) this.bounded.add(widget);
    else this.bounded.delete(widget);
    this.safeDirty = true;
  }

  private refreshSafeBounds() {
    let lo = -Infinity, hi = Infinity;
    for (const widget of this.bounded) {
      if (!widget.bounds) continue;
      const b0 = widget.bounds[0] - widget.offset;
      const b1 = widget.bounds[1] - widget.offset;
      if (this.T < b0) hi = Math.min(hi, b0 - 1);
      else if (this.T > b1) lo = Math.max(lo, b1 + 1);
      else { lo = Math.max(lo, b0); hi = Math.min(hi, b1); }
    }
    this.safeLo = lo;
    this.safeHi = hi;
    this.safeDirty = false;
  }

  private retireEarly(widget: Widget<B>, reason: string) {
    if (!widget.early && !widget.earlyMore.length) return;
    if (widget.early) { this.refuse(widget, widget.early, reason); widget.early = null; }
    for (const entry of widget.earlyMore) this.refuse(widget, entry, reason);
    widget.earlyMore = [];
    this.earlyWidgets.delete(widget);
  }

  private parkEarly(widget: Widget<B>, entry: Entry<B>) {
    const list: Entry<B>[] = [];
    if (widget.early) list.push(widget.early);
    if (widget.earlyMore.length) list.push(...widget.earlyMore);
    const kept: Entry<B>[] = [];
    for (const held of list) {
      if (held === entry) continue;
      if (held.id > entry.id && held.lo <= entry.lo && held.hi >= entry.lo) {
        this.refuse(widget, entry, 'displaced');
        return;
      }
      if (entry.id >= held.id && entry.lo <= held.lo && entry.hi >= held.lo) {
        this.refuse(widget, held, 'displaced');
      } else {
        kept.push(held);
      }
    }
    kept.push(entry);
    kept.sort((a, b) => a.lo - b.lo || a.id - b.id);
    while (kept.length > MAX_EARLY) this.refuse(widget, kept.pop()!, 'displaced');
    widget.early = kept[0] ?? null;
    widget.earlyMore = kept.slice(1);
    if (widget.early) this.earlyWidgets.add(widget);
    else this.earlyWidgets.delete(widget);
  }

  private freezeEarly(widget: Widget<B>, entries: readonly Entry<B>[], limit = Infinity) {
    for (const entry of entries) {
      if (!entry.open || this.earlyClosed.has(entry)) continue;
      const box = this.confirmedBox(widget.lane, widget.route, entry.open);
      this.earlyClosed.set(entry, Math.min(Math.max(entry.c, box.hour), limit));
    }
  }

  generation(route: number): Generation { return { seek: this.seek, reset: this.route(route).reset }; }

  mount(id: number, mount: Mount<B>) {
    this.route(mount.route);
    const dead = this.deadLanes.get(mount.lane);
    const widget: Widget<B> = { id, route: mount.route, lane: mount.lane, owner: mount.owner, offset: mount.offset ?? 0, pin: mount.pin ?? null, bounds: mount.bounds ?? null,
      surfaces: mount.surfaces, binding: 1, installed: null, content: {}, status: dead !== undefined ? 'failed' : 'reading',
      ...(dead !== undefined ? { reason: dead, failureOrigin: 'stream' as const } : {}), inspect: null, early: null, earlyMore: [],
      changed: mount.changed, dated: mount.dated, failed: dead !== undefined, face: '', drawn: 0 };
    this.widgets.set(id, widget);
    this.laneWidgetCounts[mount.lane] = (this.laneWidgetCounts[mount.lane] ?? 0) + 1;
    this.routeSet(mount.route).add(widget);
    this.trackBounds(widget);
    if (dead !== undefined) { this.touched.add(widget); this.tell(); }
  }

  unmount(id: number) {
    const widget = this.widgets.get(id);
    if (!widget) return;
    this.retireEarly(widget, 'unmounted');
    this.inspectedByLane[widget.lane]?.delete(widget);
    this.byRoute.get(widget.route)?.delete(widget);
    this.bounded.delete(widget);
    this.safeDirty = true;
    this.drop(widget.installed);
    this.laneWidgetCounts[widget.lane] = Math.max(0, (this.laneWidgetCounts[widget.lane] ?? 0) - 1);
    this.widgets.delete(id);
  }

  private drop(installed: Installed | null) {
    this.letGo(installed?.kept);
  }

  target(widget: Pick<Widget<B>, 'pin' | 'offset'>) { return widget.pin ?? this.T + widget.offset; }

  publish(T: number, { playing = this.playing, seek = false, step = this.step, speed = this.speed }: { playing?: boolean; seek?: boolean | number; step?: number; speed?: number } = {}) {
    const sought = seek === true || (typeof seek === 'number' && seek !== this.seek);
    const playingChanged = playing !== this.playing;
    const wake = sought || playingChanged || speed !== this.speed;
    const moved = T !== this.T;
    this.T = T; this.playing = playing; this.step = step; this.speed = speed; this.publishes++;
    if (sought) {
      this.seek = typeof seek === 'number' ? seek : this.seek + 1;
      for (const map of this.laneConfirmed) map.clear();
      for (const widget of this.widgets.values()) {
        this.stats.visited++;
        widget.installed?.freeze?.();
        this.revalidate(widget);
      }
      for (const route of this.routes.keys()) this.post('all', { type: 'stream', route, seek: this.seek, target: T });
    }
    this.commit();
    if (sought || playingChanged) {
      for (const widget of this.widgets.values()) {
        this.stats.visited++;
        if (this.face(widget) !== widget.face) this.touched.add(widget);
      }
      if (this.bounded.size) this.refreshSafeBounds();
    } else if (moved) {
      if (!this.playing) {
        for (const widget of this.widgets.values()) {
          this.stats.visited++;
          if (this.face(widget) !== widget.face) this.touched.add(widget);
        }
        if (this.bounded.size) this.safeDirty = true;
      } else if (this.bounded.size) {
        if (this.safeDirty || T < this.safeLo || T > this.safeHi) {
          let lo = -Infinity, hi = Infinity;
          for (const widget of this.bounded) {
            this.stats.visited++;
            if (widget.bounds) {
              const b0 = widget.bounds[0] - widget.offset;
              const b1 = widget.bounds[1] - widget.offset;
              if (T < b0) hi = Math.min(hi, b0 - 1);
              else if (T > b1) lo = Math.max(lo, b1 + 1);
              else { lo = Math.max(lo, b0); hi = Math.min(hi, b1); }
            }
            if (this.face(widget) !== widget.face) this.touched.add(widget);
          }
          this.safeLo = lo;
          this.safeHi = hi;
          this.safeDirty = false;
        }
      }
    }
    this.tell();
    this.request(wake || !playing, wake);
  }

  frame() {
    this.commit();
    this.tell();
    if (this.acks.some(list => list.length) || this.refusals.some(list => list.length)) this.request(true, false);
  }

  private request(exact: boolean, retire: boolean) {
    if (exact && retire && this.earlyWidgets.size) {
      for (const widget of [...this.earlyWidgets]) {
        this.stats.visited++;
        this.retireEarly(widget, 'retired');
      }
    }
    for (let lane = 0; lane < this.lanes; lane++) {
      if (this.laneWidgetCounts[lane] === 0 && this.acks[lane]!.length === 0 && this.refusals[lane]!.length === 0) continue;
      const lead = Math.min(MAX_AHEAD, Math.max(1, Math.round(this.ahead[lane]!)));
      const target = exact ? this.T : Math.max(this.targets[lane]!, this.T + this.step * lead);
      this.targets[lane] = exact ? -Infinity : target;
      const serial = ++this.serial;
      this.sent.set(serial, this.publishes);
      const inspect: [number, string][] = [];
      for (const widget of this.inspectedByLane[lane]!) {
        this.stats.visited++;
        if (widget.inspect !== null) inspect.push([widget.id, widget.inspect]);
      }
      this.post(lane, { type: 'frame', serial, now: this.T, target, exact, seek: this.seek, speed: this.playing ? this.speed : 0, installed: this.acks[lane]!.splice(0), refused: this.refusals[lane]!.splice(0), inspect });
    }
    if (this.sent.size > 256) for (const key of [...this.sent.keys()].slice(0, 128)) this.sent.delete(key);
  }

  private clear(widget: Widget<B>, status: Shown, reason?: string, failureOrigin?: FailureOrigin) {
    this.drop(widget.installed);
    widget.installed = null; widget.content = {}; widget.status = status; widget.reason = reason;
    widget.failureOrigin = status === 'failed' ? failureOrigin : undefined;
    for (const surface of widget.surfaces) surface.clear();
    this.touched.add(widget);
  }

  private failedRoute(widget: Widget<B>) { return widget.pin === null && this.route(widget.route).failed !== null; }

  private reading(widget: Widget<B>) {
    const dead = this.deadLanes.get(widget.lane);
    if (dead !== undefined) { widget.failed = true; this.clear(widget, 'failed', dead, 'stream'); return; }
    const failed = widget.pin === null ? this.route(widget.route).failed : null;
    this.clear(widget, failed ? 'failed' : 'reading', failed?.reason, failed ? 'stream' : undefined);
  }

  private revalidate(widget: Widget<B>) {
    const shown = widget.installed, at = this.target(widget);
    if (shown && !(shown.h <= at && at <= shown.c && shown.lo <= at && at <= shown.hi)) this.clear(widget, 'reading');
  }

  offer(presentation: Presentation<B>) {
    const at = this.sent.get(presentation.serial);
    if (at !== undefined) {
      const sample = Math.min(MAX_AHEAD, Math.max(1, this.publishes + 1 - at));
      this.ahead[presentation.lane] = Math.min(MAX_AHEAD, Math.max(1, this.ahead[presentation.lane]! * 0.8 + sample * 0.2));
    }
    this.queue.push({ ...presentation, entries: presentation.entries.map(entry => this.own(presentation.lane, entry)) });
  }

  private own(lane: number, entry: Entry<B>): Entry<B> {
    const fields: Record<string, unknown> = {}, keys: string[] = [];
    for (const [key, value] of Object.entries(entry.fields)) {
      const content = entry.content[key];
      if (value === SAME || content === undefined) { fields[key] = value; continue; }
      const at = `${lane}|${key}|${content}`;
      let held = this.kept.get(at);
      if (!held) this.kept.set(at, held = { value, holders: 0 });
      held.holders++;
      keys.push(at);
      fields[key] = held.value;
    }
    const owned = { ...entry, fields };
    this.holding.set(owned, keys);
    return owned;
  }

  private letGo(keys: readonly string[] | undefined) {
    for (const key of keys ?? []) {
      const held = this.kept.get(key)!;
      if (--held.holders === 0) this.kept.delete(key);
    }
  }

  private disown(entry: Entry<B>) {
    this.letGo(this.holding.get(entry));
    this.holding.delete(entry);
  }

  private resetFollowed(id: number, reset: number) {
    const route = this.route(id);
    if (reset <= route.reset) return;
    route.reset = reset;
    for (const map of this.laneConfirmed) {
      const held = map.get(id);
      if (held) held.followed = { hour: -Infinity };
    }
    for (const widget of this.byRoute.get(id) ?? []) {
      this.stats.visited++;
      if (widget.installed?.follows && widget.installed.gen.reset < reset) this.clear(widget, 'reading');
    }
  }

  private commit() {
    const queue = this.queue;
    this.queue = [];
    for (const presentation of queue) for (const [id, reset] of presentation.resets ?? []) this.resetFollowed(id, reset);
    const earlyByWidget = new Map<number, Entry<B>[]>();
    if (this.earlyWidgets.size) {
      for (const widget of this.earlyWidgets) {
        this.stats.visited++;
        const list: Entry<B>[] = [];
        if (widget.early) { list.push(widget.early); widget.early = null; }
        if (widget.earlyMore.length) { list.push(...widget.earlyMore); widget.earlyMore = []; }
        if (list.length) earlyByWidget.set(widget.id, list);
      }
      this.earlyWidgets.clear();
    }
    for (const presentation of queue) {
      for (const [widgetId, lo, hi] of presentation.closed ?? []) {
        const widget = this.widgets.get(widgetId);
        if (!widget) continue;
        const shown = widget.installed;
        if (shown?.freeze) shown.freeze(hi);
        else if (shown && shown.c > hi) shown.c = hi;
        if (shown && Number.isFinite(hi) && Math.min(shown.c, shown.hi) < Math.max(shown.h, lo)) this.clear(widget, 'reading');
        const prev = earlyByWidget.get(widgetId);
        if (prev) this.freezeEarly(widget, prev, hi);
        if (widget.early) this.freezeEarly(widget, [widget.early, ...widget.earlyMore], hi);
      }
      for (const [widgetId, binding] of presentation.withdrawn ?? []) {
        const widget = this.widgets.get(widgetId);
        if (!widget) continue;
        if (widget.early && widget.early.binding <= binding) {
          this.refuse(widget, widget.early, 'binding');
          widget.early = null;
        }
        if (widget.earlyMore.length) {
          const keep: Entry<B>[] = [];
          for (const entry of widget.earlyMore) {
            if (entry.binding <= binding) this.refuse(widget, entry, 'binding');
            else keep.push(entry);
          }
          widget.earlyMore = keep;
          if (!widget.early && widget.earlyMore.length) widget.early = widget.earlyMore.shift()!;
        }
        if (!widget.early) this.earlyWidgets.delete(widget);
        const pending = earlyByWidget.get(widgetId);
        if (pending) {
          const keep: Entry<B>[] = [];
          for (const entry of pending) {
            if (entry.binding <= binding) this.refuse(widget, entry, 'binding');
            else keep.push(entry);
          }
          if (keep.length) earlyByWidget.set(widgetId, keep);
          else earlyByWidget.delete(widgetId);
        }
        if (widget.installed && widget.installed.binding <= binding) this.clear(widget, 'reading');
      }
      for (const entry of presentation.entries) {
        this.stats.visited++;
        this.install(entry, presentation.lane);
      }
      for (const [routeId, channel, hour, seek, reset] of presentation.confirmed ?? []) {
        if (seek !== this.seek || reset !== this.route(routeId).reset) continue;
        const box = this.confirmedBox(presentation.lane, routeId, channel);
        if (hour > box.hour) {
          box.hour = hour;
          if (!this.playing) {
            for (const widget of this.byRoute.get(routeId) ?? []) {
              if (widget.lane === presentation.lane && widget.installed?.open === channel && this.face(widget) !== widget.face) {
                this.touched.add(widget);
              }
            }
          }
        }
      }
    }
    for (const [widgetId, list] of earlyByWidget) {
      const widget = this.widgets.get(widgetId);
      if (!widget) { for (const entry of list) this.install(entry, -1, true); continue; }
      const at = this.target(widget);
      const eligible: Entry<B>[] = [];
      for (const entry of list) {
        if (at >= entry.lo && at <= entry.hi) eligible.push(entry);
        else this.install(entry, -1, true);
      }
      if (eligible.length > 1) {
        eligible.sort((a, b) => b.id - a.id);
        let installedOne = false;
        for (const entry of eligible) {
          if (installedOne) this.refuse(widget, entry, 'superseded');
          else if (this.reject(widget, entry) === null) { this.install(entry, -1, true); installedOne = true; }
          else this.install(entry, -1, true);
        }
      } else if (eligible.length === 1) {
        this.install(eligible[0]!, -1, true);
      }
    }
  }

  private effectiveC(widget: Widget<B>, entry: Entry<B>): number {
    const closedAt = this.earlyClosed.get(entry);
    if (closedAt !== undefined) return closedAt;
    if (entry.open && entry.gen.seek === this.seek && entry.gen.reset === this.route(widget.route).reset) {
      return Math.max(entry.c, this.confirmedBox(widget.lane, widget.route, entry.open).hour);
    }
    return entry.c;
  }

  private reject(widget: Widget<B>, entry: Entry<B>): string | null {
    const route = this.route(widget.route), at = this.target(widget);
    if (widget.pin === null) {
      if (entry.gen.seek !== this.seek) return 'stream';
      if (entry.gen.reset < route.reset) return 'reset';
      if (route.failed && route.failed.seek >= entry.gen.seek) return 'failed stream';
    }
    if (widget.owner !== undefined && entry.owner !== undefined && entry.owner !== widget.owner) return 'owner';
    if (entry.binding !== widget.binding) return 'binding';
    if (widget.failed) return 'failed';
    const shown = widget.installed;
    if (shown && shown.binding === entry.binding && entry.id < shown.id) return 'superseded';
    if (at < entry.lo) return 'early';
    if (at > entry.hi && (!this.playing || (shown && shown.binding === entry.binding && shown.lo >= entry.lo))) return 'superseded';
    if (!shown && Object.values(entry.fields).some(value => value === SAME)) return 'kept content';
    const c = this.effectiveC(widget, entry);
    if (entry.mode === 'refused' && !this.playing && shown && at > c && !sameContent(entry.content, widget.content)) return 'uncovered';
    if (entry.mode !== 'picture') return null;
    for (let k = 0; k < widget.surfaces.length; k++) {
      const part = entry.surfaces[k];
      if (part === SAME ? widget.content[k] !== entry.content[k] : !part || part.width === 0) return 'kept content';
    }
    for (const [key, value] of Object.entries(entry.fields)) if (value === SAME && (!shown || widget.content[key] !== entry.content[key])) return 'kept content';
    if (!this.playing && shown && at > c && !sameContent(entry.content, widget.content)) return 'uncovered';
    return null;
  }

  private refuse(widget: Widget<B>, entry: Entry<B>, reason: string) {
    this.stats.rejected[reason] = (this.stats.rejected[reason] ?? 0) + 1;
    this.refusals[widget.lane]!.push({ widget: widget.id, id: entry.id, reason });
    this.earlyClosed.delete(entry);
    closeParts(entry);
    this.disown(entry);
  }

  private install(entry: Entry<B>, lane: number, fromEarly = false) {
    const widget = this.widgets.get(entry.widget);
    if (!widget) {
      closeParts(entry);
      this.disown(entry);
      if (lane >= 0) { this.stats.rejected.unmounted = (this.stats.rejected.unmounted ?? 0) + 1; this.refusals[lane]!.push({ widget: entry.widget, id: entry.id, reason: 'unmounted' }); }
      return;
    }
    const route = this.route(widget.route);
    let why = this.reject(widget, entry);
    if (why === null && entry.gen.reset > route.reset) {
      this.resetFollowed(widget.route, entry.gen.reset);
      why = this.reject(widget, entry);
    }
    if (why === 'early') {
      this.parkEarly(widget, entry);
      return;
    }
    if (why) { this.refuse(widget, entry, why); return; }
    if (route.failed && entry.gen.seek > route.failed.seek) route.failed = null;
    try {
      if (entry.mode !== 'picture') {
        closeParts(entry);
        for (let k = 0; k < widget.surfaces.length; k++) widget.surfaces[k]!.clear();
      } else {
        for (let k = 0; k < widget.surfaces.length; k++) {
          const part = entry.surfaces[k];
          if (!part) widget.surfaces[k]!.clear();
          else if (part !== SAME) widget.surfaces[k]!.install(part);
        }
        for (let k = widget.surfaces.length; k < entry.surfaces.length; k++) {
          const extra = entry.surfaces[k];
          if (extra && extra !== SAME) extra.close();
        }
      }
    } catch {
      closeParts(entry);
      this.disown(entry);
      this.stats.failed++;
      widget.failed = true;
      this.failures.push([widget.id, entry.stamp]);
      this.clear(widget, 'failed', 'drawing failed', 'local');
      this.refusals[widget.lane]!.push({ widget: widget.id, id: entry.id, reason: 'drawing failed' });
      return;
    }
    if (entry.mode === 'picture' && (entry.repainted || entry.surfaces.some(part => part && part !== SAME))) this.pictures++;
    const fields: Record<string, unknown> = {}, kept = this.holding.get(entry) ?? [];
    this.holding.delete(entry);
    for (const [key, value] of Object.entries(entry.fields)) {
      const content = entry.content[key];
      if (value !== SAME) { fields[key] = value; continue; }
      const at = `${widget.lane}|${key}|${content}`, held = content === undefined ? undefined : this.kept.get(at);
      if (held) { held.holders++; kept.push(at); fields[key] = held.value; }
      else fields[key] = widget.installed!.fields[key];
    }
    const drew = !widget.installed || widget.installed.mode !== entry.mode || entry.reason !== widget.installed.reason
      || entry.repainted === true || entry.surfaces.some(part => part !== SAME) || Object.values(entry.fields).some(value => value !== SAME)
      || Object.keys(widget.installed.fields).length !== Object.keys(entry.fields).length;
    widget.content = entry.mode === 'picture' ? { ...entry.content } : {};
    this.drop(widget.installed);
    const closedAt = this.earlyClosed.get(entry);
    this.earlyClosed.delete(entry);
    const keepOpen = closedAt === undefined && entry.open !== undefined
      && (!fromEarly || (entry.gen.seek === this.seek && entry.gen.reset === route.reset));
    const open = keepOpen ? entry.open : undefined;
    const box = open ? this.confirmedBox(widget.lane, widget.route, open) : null;
    let baseC = closedAt ?? (box ? Math.max(entry.c, box.hour) : entry.c), live = box !== null;
    const installed: Installed = { id: entry.id, ...(entry.owner !== undefined ? { owner: entry.owner } : {}), h: entry.h, c: baseC, lo: entry.lo, hi: entry.hi, gen: entry.gen, binding: entry.binding, follows: entry.follows,
      ...open ? { open } : {},
      ...(entry.viewport ? { viewport: entry.viewport } : {}),
      mode: entry.mode, reason: entry.reason, failureOrigin: entry.failureOrigin, fields, ms: entry.ms ?? 0, kept, structure: structureOf(widget.content, fields) };
    if (box) {
      Object.defineProperty(installed, 'c', { enumerable: true, configurable: true, get: () => live ? Math.max(baseC, box.hour) : baseC });
      installed.freeze = (limit = Infinity) => {
        if (live) { baseC = Math.max(baseC, box.hour); live = false; installed.open = undefined; }
        baseC = Math.min(baseC, limit);
      };
    }
    widget.installed = installed;
    widget.status = entry.mode; widget.reason = entry.reason;
    widget.failureOrigin = entry.mode === 'failed' ? entry.failureOrigin : undefined;
    this.acks[widget.lane]!.push({ widget: widget.id, binding: widget.binding, id: entry.id });
    this.stats.installs++;
    if (drew) { widget.drawn++; this.touched.add(widget); } else this.redated.add(widget);
  }

  private face(widget: Widget<B>) {
    const view = this.view(widget.id);
    return `${view.mode}|${view.dim}|${view.expired === true}|${widget.drawn}|${view.reason ?? ''}`;
  }

  private tell() {
    const touched = this.touched, redated = this.redated;
    this.touched = new Set(); this.redated = new Set();
    for (const widget of touched) {
      if (!this.widgets.has(widget.id)) continue;
      widget.face = this.face(widget);
      widget.changed?.();
    }
    for (const widget of redated) if (!touched.has(widget) && this.widgets.has(widget.id)) {
      if (this.face(widget) !== widget.face) { widget.face = this.face(widget); widget.changed?.(); }
      else widget.dated?.();
    }
  }

  view(id: number): WidgetView {
    const widget = this.widgets.get(id)!, at = this.target(widget), shown = widget.installed;
    if (!this.deadLanes.has(widget.lane) && widget.bounds && (at < widget.bounds[0] || at > widget.bounds[1])) return { mode: 'outside', target: at, ...(widget.owner !== undefined ? { owner: widget.owner } : {}), binding: widget.binding, dim: false, tooltip: null };
    if (!shown) return { mode: widget.status, reason: widget.reason, ...(widget.waiting && widget.status === 'reading' ? { waiting: true as const } : {}), ...(widget.failureOrigin ? { failureOrigin: widget.failureOrigin } : {}), ...(widget.owner !== undefined ? { owner: widget.owner } : {}), target: at, binding: widget.binding, dim: false, tooltip: null };
    const tip = shown.fields.tooltip as { key: string; text: unknown } | undefined;
    const until = Math.min(shown.c, shown.hi), expired = !this.playing && at > until;
    return { mode: shown.mode, reason: shown.reason, ...(shown.failureOrigin ? { failureOrigin: shown.failureOrigin } : {}), id: shown.id, ...(widget.owner !== undefined ? { owner: widget.owner } : {}), ...(shown.owner !== undefined ? { installedOwner: shown.owner } : {}), binding: widget.binding, installedBinding: shown.binding, gen: shown.gen, validInterval: [shown.lo, shown.hi] as const, ...(shown.viewport ? { viewport: shown.viewport } : {}), h: shown.h, through: Math.min(at, until), painted: Math.min(Math.max(shown.h, shown.lo), at, until), structure: shown.structure ?? structureOf(widget.content, shown.fields), target: at,
      dim: expired || shown.binding !== widget.binding, expired, fields: shown.fields,
      tooltip: tip && widget.inspect !== null && tip.key === widget.inspect ? tip.text : null };
  }

  rebind(id: number, change: RebindChange = {}) {
    const widget = this.widgets.get(id);
    if (!widget) return;
    widget.binding++;
    const ownerChanged = change.owner !== undefined && change.owner !== widget.owner;
    if (change.owner !== undefined) widget.owner = change.owner;
    if (change.pin !== undefined) widget.pin = change.pin;
    if (change.offset !== undefined) widget.offset = change.offset;
    if (change.bounds !== undefined) widget.bounds = change.bounds;
    if (change.pin !== undefined || change.offset !== undefined || change.bounds !== undefined) this.trackBounds(widget);
    this.retireEarly(widget, 'binding');
    if (this.deadLanes.has(widget.lane)) {
      this.reading(widget);
    } else if (ownerChanged) {
      widget.failed = false;
      widget.inspect = null;
      this.inspectedByLane[widget.lane]?.delete(widget);
      this.clear(widget, 'reading');
    } else if (change.subscription) { widget.failed = false; if (this.failedRoute(widget) || widget.status === 'failed' || widget.status === 'refused') this.reading(widget); }
    else if (change.pin !== undefined || change.offset !== undefined) this.revalidate(widget);
    this.touched.add(widget);
    this.tell();
    this.request(true, false);
  }

  remount(id: number, surfaces: readonly PageSurface<B>[]) {
    const widget = this.widgets.get(id);
    if (!widget) return;
    const first = widget.surfaces.length === 0;
    widget.surfaces = surfaces; widget.binding++;
    this.retireEarly(widget, 'binding');
    if (this.deadLanes.has(widget.lane)) this.reading(widget);
    else if (first) this.touched.add(widget);
    else { widget.failed = false; this.reading(widget); }
    this.tell();
    this.request(true, false);
  }

  inspect(id: number, key: string | null) {
    const widget = this.widgets.get(id);
    if (!widget || widget.inspect === key) return;
    widget.inspect = key;
    if (key === null) this.inspectedByLane[widget.lane]?.delete(widget);
    else this.inspectedByLane[widget.lane]?.add(widget);
    this.touched.add(widget);
    this.tell();
    this.request(true, false);
  }

  waiting(id: number, waiting: boolean) {
    const widget = this.widgets.get(id);
    if (!widget || !widget.waiting === !waiting) return;
    widget.waiting = waiting;
    if (!widget.installed) { this.touched.add(widget); this.tell(); }
  }

  name(id: number, status: 'failed' | 'refused' | 'reading', reason?: string, failureOrigin?: FailureOrigin) {
    const widget = this.widgets.get(id);
    if (!widget) return;
    if (this.deadLanes.has(widget.lane)) {
      this.retireEarly(widget, 'failed');
      this.reading(widget);
      this.tell();
      return;
    }
    if (status === 'failed' || status === 'refused') widget.failed = true;
    this.retireEarly(widget, status === 'reading' ? 'binding' : status);
    this.clear(widget, status, reason, status === 'failed' ? (failureOrigin ?? 'stream') : undefined);
    this.tell();
  }

  takeFailures(): readonly [widget: number, stamp: number | undefined][] {
    const failures = this.failures;
    this.failures = [];
    return failures;
  }

  retryLocal(id: number) {
    const widget = this.widgets.get(id);
    if (!widget || this.deadLanes.has(widget.lane)) return;
    widget.failed = false;
    widget.binding++;
    this.retireEarly(widget, 'binding');
    this.clear(widget, 'reading');
    this.tell();
    this.request(true, false);
  }

  failLane(lane: number, reason: string) {
    this.deadLanes.set(lane, reason);
    for (const widget of this.widgets.values()) {
      if (widget.lane !== lane) continue;
      widget.failed = true;
      this.retireEarly(widget, 'failed');
      this.clear(widget, 'failed', reason, 'stream');
    }
    this.tell();
  }

  takePictures(): number {
    const pictures = this.pictures;
    this.pictures = 0;
    return pictures;
  }

  expired(): number[] {
    const out: number[] = [];
    for (const widget of this.widgets.values()) {
      const shown = widget.installed, at = this.target(widget);
      if (shown && at > Math.min(shown.hi, shown.c) && !(widget.bounds && (at < widget.bounds[0] || at > widget.bounds[1]))) out.push(widget.id);
    }
    return out;
  }

  lane(id: number) { return this.widgets.get(id)?.lane; }

  mountRoute(id: number, route: number) {
    const widget = this.widgets.get(id);
    if (!widget) return;
    if (widget.route !== route) {
      this.byRoute.get(widget.route)?.delete(widget);
      widget.route = route;
      this.routeSet(route).add(widget);
    }
    const failed = this.route(route).failed;
    if (!this.deadLanes.has(widget.lane) && widget.pin === null && failed && failed.seek === this.seek && widget.status !== 'failed') this.reading(widget);
  }

  command(route: number) { return ++this.route(route).command; }

  carry(from: number, to: number) {
    const failed = this.routes.get(from)?.failed;
    if (failed) this.route(to).failed = failed;
  }

  fail(route: number, command: number, reason: string): boolean {
    const at = this.route(route);
    if (command !== at.command) return false;
    at.failed = { seek: this.seek, reason };
    for (const widget of this.byRoute.get(route) ?? []) if (widget.pin === null && !this.deadLanes.has(widget.lane)) {
      this.retireEarly(widget, 'failed stream');
      this.clear(widget, 'failed', reason, 'stream');
    }
    this.post('all', { type: 'fail', route, seek: this.seek, command, reason });
    this.tell();
    return true;
  }

  dispose() {
    for (const presentation of this.queue) for (const entry of presentation.entries) { this.earlyClosed.delete(entry); closeParts(entry); this.disown(entry); }
    this.queue = [];
    for (const widget of this.widgets.values()) {
      if (widget.early) { this.earlyClosed.delete(widget.early); closeParts(widget.early); this.disown(widget.early); widget.early = null; }
      for (const entry of widget.earlyMore) { this.earlyClosed.delete(entry); closeParts(entry); this.disown(entry); }
      widget.earlyMore = [];
      this.drop(widget.installed);
      widget.installed = null;
      widget.surfaces = [];
      widget.changed = undefined;
      widget.dated = undefined;
    }
    this.widgets.clear(); this.routes.clear(); this.deadLanes.clear(); this.byRoute.clear(); this.earlyWidgets.clear(); this.bounded.clear();
    this.kept.clear(); this.sent.clear(); this.touched.clear(); this.redated.clear();
    this.safeLo = -Infinity; this.safeHi = Infinity; this.safeDirty = false;
    for (const map of this.laneConfirmed) map.clear();
    for (const set of this.inspectedByLane) set.clear();
    for (const list of this.acks) list.length = 0;
    for (const list of this.refusals) list.length = 0;
    for (let i = 0; i < this.targets.length; i++) this.targets[i] = -Infinity;
  }

  pending(lane: number) { return { installed: this.acks[lane]!.length, refused: this.refusals[lane]!.length }; }
}
