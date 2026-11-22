export const SAME = 'same' as const;
export type Same = typeof SAME;

export interface Bitmap { readonly width: number; close(): void }

export interface PageSurface<B extends Bitmap = Bitmap> {
  install(bitmap: B): void;
  clear(): void;
}

export type Mode = 'picture' | 'refused' | 'failed';
export type Shown = Mode | 'reading' | 'outside';

export interface Generation { seek: number; reset: number }

export interface Entry<B extends Bitmap = Bitmap> {
  widget: number;
  binding: number;
  id: number;
  h: number;
  c: number;
  lo: number;
  hi: number;
  gen: Generation;
  follows: boolean;
  rank?: number;
  mode: Mode;
  reason?: string;
  surfaces: readonly (B | Same | null)[];
  content: Readonly<Record<string, string>>;
  fields: Readonly<Record<string, unknown>>;
}

export interface Presentation<B extends Bitmap = Bitmap> {
  lane: number;
  serial: number;
  resets?: readonly (readonly [route: number, reset: number])[];
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
  speed: number;
  installed: Ack[];
  refused: Refusal[];
  inspect: [widget: number, key: string][];
}

export type PageMessage = FrameRequest
  | { type: 'stream'; route: number; seek: number; target: number }
  | { type: 'fail'; route: number; seek: number; command: number; reason: string };

export interface Installed {
  id: number; h: number; c: number; lo: number; hi: number; gen: Generation; binding: number; follows: boolean;
  mode: Mode; reason?: string; fields: Readonly<Record<string, unknown>>;
}

export interface WidgetView {
  mode: Shown;
  reason?: string;
  target: number;
  id?: number;
  h?: number;
  through?: number;
  dim: boolean;
  fields?: Readonly<Record<string, unknown>>;
  tooltip: unknown;
}

export interface Mount<B extends Bitmap = Bitmap> {
  route: number;
  lane: number;
  offset?: number;
  pin?: number | null;
  bounds?: readonly [first: number, last: number] | null;
  surfaces: readonly PageSurface<B>[];
  rank?: number;
  changed?: () => void;
}

interface Widget<B extends Bitmap> {
  id: number; route: number; lane: number; offset: number; pin: number | null; bounds: readonly [number, number] | null;
  surfaces: readonly PageSurface<B>[]; binding: number; installed: Installed | null; content: Record<string, string>;
  status: Shown; reason?: string; inspect: string | null; rank?: number; early: Entry<B> | null; changed?: () => void;
}

interface RouteState { reset: number; command: number; failed: { seek: number; reason: string } | null }

export interface RebindChange { pin?: number | null; offset?: number; subscription?: boolean; bounds?: readonly [number, number] | null; rank?: number }

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
  readonly stats = { installs: 0, rejected: {} as Record<string, number>, failed: 0 };
  readonly widgets = new Map<number, Widget<B>>();
  private serial = 0;
  private readonly post: (lane: number | 'all', message: PageMessage) => void;
  private readonly sent = new Map<number, number>();
  private readonly targets: number[];
  private readonly acks: Ack[][];
  private readonly refusals: Refusal[][];
  private readonly routes = new Map<number, RouteState>();
  private queue: Presentation<B>[] = [];
  private touched = new Set<Widget<B>>();

  constructor({ lanes = 1, post = () => {}, seek = 0 }: { lanes?: number; post?: (lane: number | 'all', message: PageMessage) => void; seek?: number } = {}) {
    this.lanes = lanes; this.post = post; this.seek = seek;
    this.ahead = Array.from({ length: lanes }, () => 1);
    this.targets = Array.from({ length: lanes }, () => -Infinity);
    this.acks = Array.from({ length: lanes }, () => []);
    this.refusals = Array.from({ length: lanes }, () => []);
  }

  private route(id: number): RouteState {
    let route = this.routes.get(id);
    if (!route) this.routes.set(id, route = { reset: 0, command: 0, failed: null });
    return route;
  }

  generation(route: number): Generation { return { seek: this.seek, reset: this.route(route).reset }; }

  mount(id: number, mount: Mount<B>) {
    this.route(mount.route);
    this.widgets.set(id, { id, route: mount.route, lane: mount.lane, offset: mount.offset ?? 0, pin: mount.pin ?? null, bounds: mount.bounds ?? null,
      surfaces: mount.surfaces, binding: 1, installed: null, content: {}, status: 'reading', inspect: null, rank: mount.rank, early: null, changed: mount.changed });
  }

  unmount(id: number) {
    const widget = this.widgets.get(id);
    if (!widget) return;
    if (widget.early) this.refuse(widget, widget.early, 'unmounted');
    this.widgets.delete(id);
  }

  target(widget: Pick<Widget<B>, 'pin' | 'offset'>) { return widget.pin ?? this.T + widget.offset; }

  publish(T: number, { playing = this.playing, seek = false, step = this.step, speed = this.speed }: { playing?: boolean; seek?: boolean | number; step?: number; speed?: number } = {}) {
    const sought = seek === true || (typeof seek === 'number' && seek !== this.seek);
    const wake = sought || playing !== this.playing || speed !== this.speed;
    const moved = T !== this.T;
    this.T = T; this.playing = playing; this.step = step; this.speed = speed; this.publishes++;
    if (sought) {
      this.seek = typeof seek === 'number' ? seek : this.seek + 1;
      for (const widget of this.widgets.values()) this.revalidate(widget);
      for (const route of this.routes.keys()) this.post('all', { type: 'stream', route, seek: this.seek, target: T });
    }
    this.commit();
    if (moved || sought) for (const widget of this.widgets.values()) if (widget.pin === null) this.touched.add(widget);
    this.tell();
    this.request(wake || !playing, wake);
  }

  frame() {
    this.commit();
    this.tell();
    if (this.acks.some(list => list.length) || this.refusals.some(list => list.length)) this.request(true, false);
  }

  private request(exact: boolean, retire: boolean) {
    if (exact && retire) for (const widget of this.widgets.values()) if (widget.early) { this.refuse(widget, widget.early, 'retired'); widget.early = null; }
    for (let lane = 0; lane < this.lanes; lane++) {
      const target = exact ? this.T : Math.max(this.targets[lane]!, this.T + this.step * Math.max(1, Math.round(this.ahead[lane]!)));
      this.targets[lane] = exact ? -Infinity : target;
      const serial = ++this.serial;
      this.sent.set(serial, this.publishes);
      const inspect: [number, string][] = [];
      for (const widget of this.widgets.values()) if (widget.lane === lane && widget.inspect !== null) inspect.push([widget.id, widget.inspect]);
      this.post(lane, { type: 'frame', serial, now: this.T, target, exact, speed: this.playing ? this.speed : 0, installed: this.acks[lane]!.splice(0), refused: this.refusals[lane]!.splice(0), inspect });
    }
    if (this.sent.size > 256) for (const key of [...this.sent.keys()].slice(0, 128)) this.sent.delete(key);
  }

  private clear(widget: Widget<B>, status: Shown, reason?: string) {
    widget.installed = null; widget.content = {}; widget.status = status; widget.reason = reason;
    for (const surface of widget.surfaces) surface.clear();
    this.touched.add(widget);
  }

  private revalidate(widget: Widget<B>) {
    const shown = widget.installed, at = this.target(widget);
    if (shown && !(shown.h <= at && at <= shown.c)) this.clear(widget, 'reading');
  }

  offer(presentation: Presentation<B>) {
    const at = this.sent.get(presentation.serial);
    if (at !== undefined) this.ahead[presentation.lane] = this.ahead[presentation.lane]! * 0.8 + (this.publishes + 1 - at) * 0.2;
    this.queue.push(presentation);
  }

  private commit() {
    const queue = this.queue;
    this.queue = [];
    for (const presentation of queue) for (const [id, reset] of presentation.resets ?? []) {
      const route = this.route(id);
      if (reset <= route.reset) continue;
      route.reset = reset;
      for (const widget of this.widgets.values())
        if (widget.route === id && widget.installed?.follows && widget.installed.gen.reset < reset) this.clear(widget, 'reading');
    }
    const early = [...this.widgets.values()].flatMap(widget => {
      const held = widget.early;
      widget.early = null;
      return held ? [held] : [];
    });
    for (const presentation of queue) for (const entry of presentation.entries) this.install(entry);
    for (const entry of early) this.install(entry);
  }

  private reject(widget: Widget<B>, entry: Entry<B>): string | null {
    const route = this.route(widget.route), at = this.target(widget);
    if (widget.pin === null) {
      if (entry.gen.seek !== this.seek) return 'stream';
      if (entry.gen.reset < route.reset) return 'reset';
      if (route.failed && route.failed.seek >= entry.gen.seek) return 'failed stream';
    }
    if (entry.binding !== widget.binding) return 'binding';
    if (entry.rank !== widget.rank) return 'rank window';
    const shown = widget.installed;
    if (shown && shown.binding === entry.binding && entry.id < shown.id) return 'superseded';
    if (at < entry.lo) return 'early';
    if (at > entry.hi) return 'superseded';
    if (entry.mode !== 'picture') return null;
    for (let k = 0; k < widget.surfaces.length; k++) {
      const part = entry.surfaces[k];
      if (part === SAME ? widget.content[k] !== entry.content[k] : !part || part.width === 0) return 'kept content';
    }
    for (const [key, value] of Object.entries(entry.fields)) if (value === SAME && (!shown || widget.content[key] !== entry.content[key])) return 'kept content';
    return null;
  }

  private refuse(widget: Widget<B>, entry: Entry<B>, reason: string) {
    this.stats.rejected[reason] = (this.stats.rejected[reason] ?? 0) + 1;
    this.refusals[widget.lane]!.push({ widget: widget.id, id: entry.id, reason });
    closeParts(entry);
  }

  private install(entry: Entry<B>) {
    const widget = this.widgets.get(entry.widget);
    if (!widget) { closeParts(entry); return; }
    const why = this.reject(widget, entry);
    if (why === 'early') {
      const held = widget.early;
      if (held && held !== entry && held.id > entry.id) { this.refuse(widget, entry, 'displaced'); return; }
      if (held && held !== entry) this.refuse(widget, held, 'displaced');
      widget.early = entry;
      return;
    }
    if (why) { this.refuse(widget, entry, why); return; }
    const route = this.route(widget.route);
    if (entry.gen.reset > route.reset) route.reset = entry.gen.reset;
    if (route.failed && entry.gen.seek > route.failed.seek) route.failed = null;
    try {
      for (let k = 0; k < widget.surfaces.length; k++) {
        const part = entry.surfaces[k];
        if (entry.mode !== 'picture' || !part) widget.surfaces[k]!.clear();
        else if (part !== SAME) widget.surfaces[k]!.install(part);
      }
    } catch {
      closeParts(entry);
      this.stats.failed++;
      this.clear(widget, 'failed', 'drawing failed');
      this.refusals[widget.lane]!.push({ widget: widget.id, id: entry.id, reason: 'drawing failed' });
      return;
    }
    const fields: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(entry.fields)) fields[key] = value === SAME ? widget.installed!.fields[key] : value;
    widget.content = entry.mode === 'picture' ? { ...entry.content } : {};
    widget.installed = { id: entry.id, h: entry.h, c: entry.c, lo: entry.lo, hi: entry.hi, gen: entry.gen, binding: entry.binding, follows: entry.follows,
      mode: entry.mode, reason: entry.reason, fields };
    widget.status = entry.mode; widget.reason = entry.reason;
    this.acks[widget.lane]!.push({ widget: widget.id, binding: widget.binding, id: entry.id });
    this.stats.installs++;
    this.touched.add(widget);
  }

  private tell() {
    const touched = this.touched;
    this.touched = new Set();
    for (const widget of touched) widget.changed?.();
  }

  view(id: number): WidgetView {
    const widget = this.widgets.get(id)!, at = this.target(widget), shown = widget.installed;
    if (widget.bounds && (at < widget.bounds[0] || at > widget.bounds[1])) return { mode: 'outside', target: at, dim: false, tooltip: null };
    if (!shown) return { mode: widget.status, reason: widget.reason, target: at, dim: false, tooltip: null };
    const tip = shown.fields.tooltip as { key: string; text: unknown } | undefined;
    return { mode: shown.mode, reason: shown.reason, id: shown.id, h: shown.h, through: Math.min(at, shown.c), target: at,
      dim: (!this.playing && at > shown.c) || shown.binding !== widget.binding, fields: shown.fields,
      tooltip: tip && widget.inspect !== null && tip.key === widget.inspect ? tip.text : null };
  }

  rebind(id: number, change: RebindChange = {}) {
    const widget = this.widgets.get(id);
    if (!widget) return;
    widget.binding++;
    if (change.pin !== undefined) widget.pin = change.pin;
    if (change.offset !== undefined) widget.offset = change.offset;
    if (change.bounds !== undefined) widget.bounds = change.bounds;
    if (change.rank !== undefined) widget.rank = change.rank;
    if (widget.early) { this.refuse(widget, widget.early, 'binding'); widget.early = null; }
    if (change.subscription) this.clear(widget, 'reading');
    else if (change.pin !== undefined || change.offset !== undefined) this.revalidate(widget);
    this.touched.add(widget);
    this.tell();
    this.request(true, false);
  }

  remount(id: number, surfaces: readonly PageSurface<B>[]) {
    const widget = this.widgets.get(id);
    if (!widget) return;
    widget.surfaces = surfaces; widget.binding++;
    if (widget.early) { this.refuse(widget, widget.early, 'binding'); widget.early = null; }
    this.clear(widget, 'reading');
    this.tell();
    this.request(true, false);
  }

  inspect(id: number, key: string | null) {
    const widget = this.widgets.get(id);
    if (!widget || widget.inspect === key) return;
    widget.inspect = key;
    this.touched.add(widget);
    this.tell();
    this.request(true, false);
  }

  name(id: number, status: 'failed' | 'reading', reason?: string) {
    const widget = this.widgets.get(id);
    if (!widget) return;
    if (widget.early) { this.refuse(widget, widget.early, 'binding'); widget.early = null; }
    this.clear(widget, status, reason);
    this.tell();
  }

  lane(id: number) { return this.widgets.get(id)?.lane; }

  mountRoute(id: number, route: number) {
    const widget = this.widgets.get(id);
    if (widget) { widget.route = route; this.route(route); }
  }

  command(route: number) { return ++this.route(route).command; }

  fail(route: number, command: number, reason: string): boolean {
    const at = this.route(route);
    if (command !== at.command) return false;
    at.failed = { seek: this.seek, reason };
    for (const widget of this.widgets.values()) if (widget.route === route && widget.pin === null) {
      if (widget.early) { this.refuse(widget, widget.early, 'failed stream'); widget.early = null; }
      this.clear(widget, 'failed', reason);
    }
    this.post('all', { type: 'fail', route, seek: this.seek, command, reason });
    this.tell();
    return true;
  }

  pending(lane: number) { return { installed: this.acks[lane]!.length, refused: this.refusals[lane]!.length }; }
}
