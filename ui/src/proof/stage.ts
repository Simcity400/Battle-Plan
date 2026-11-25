import { SAME, type Ack, type Entry, type FrameRequest, type Refusal } from '../dashboard/follow/page.ts';
import type { Presented } from '../draw/lane.ts';
import type { FromLane, StageClock } from '../draw/laneMessages';

export interface StagePlacement { binding: number; owner?: string; offset: number; pin: number | null; route: number | null }
export interface StageLane { frame(request: FrameRequest): void; placement(widget: number): StagePlacement | null }

interface Shown {
  readonly entry: Entry<ImageBitmap>;
  readonly content: Readonly<Record<string, string>>;
  readonly fields: Readonly<Record<string, unknown>>;
  forwarded: boolean;
}

interface Held { canvas: OffscreenCanvas | null; context: ImageBitmapRenderingContext | null; shown: Shown | null; early: Entry<ImageBitmap>[] }

const MAX_EARLY = 4;
const MAX_AHEAD = 4;
const SERIALS = 1_000_000_000;

const sameContent = (a: Readonly<Record<string, string>>, b: Readonly<Record<string, string>>) => {
  const keys = Object.keys(a).filter(key => key !== 'hour');
  return keys.length === Object.keys(b).filter(key => key !== 'hour').length && keys.every(key => a[key] === b[key]);
};

const closeParts = (entry: Entry<ImageBitmap>) => {
  for (const part of entry.surfaces) if (part && part !== SAME) part.close();
};

export class Stage {
  private lane: StageLane | null = null;
  private laneIndex = 0;
  private readonly widgets = new Map<number, Held>();
  private clockState: StageClock | null = null;
  private seek = 0;
  private readonly resets = new Map<number, number>();
  private readonly confirmed = new Map<string, { hour: number; seek: number; reset: number; route: number; channel: 'followed' | 'panel' }>();
  private acks: Ack[] = [];
  private refusals: Refusal[] = [];
  private serial = SERIALS;
  private ticks = 0;
  private readonly sent = new Map<number, number>();
  private ahead = 1;
  private target = -Infinity;
  private lastHour: number | null = null;
  private step = 0;
  private looping = false;
  private readonly forward: (message: FromLane) => void;
  private readonly stretch: (ms: number) => void;

  constructor(forward: (message: FromLane) => void, stretch: (ms: number) => void) {
    this.forward = forward;
    this.stretch = stretch;
  }

  start(lane: StageLane, index: number) {
    this.lane = lane;
    this.laneIndex = index;
  }

  private held(widget: number): Held {
    let held = this.widgets.get(widget);
    if (!held) this.widgets.set(widget, held = { canvas: null, context: null, shown: null, early: [] });
    return held;
  }

  attach(widget: number, canvas: OffscreenCanvas) {
    const held = this.held(widget);
    held.canvas = canvas;
    held.context = canvas.getContext('bitmaprenderer');
  }

  detach(widget: number) {
    const held = this.widgets.get(widget);
    if (!held) return;
    for (const entry of held.early) closeParts(entry);
    this.widgets.delete(widget);
  }

  private playing() { return this.clockState?.playing ?? false; }

  private hourAt(now: number): number {
    const clock = this.clockState;
    if (!clock) return 0;
    if (!clock.playing) return Math.floor(clock.position);
    return Math.floor(Math.min(clock.last, clock.position + Math.max(0, now - clock.at) / 1000 * clock.speed));
  }

  private now() { return performance.timeOrigin + performance.now(); }

  clock(next: StageClock) {
    const began = performance.now();
    const previous = this.clockState;
    this.clockState = next;
    if (!previous || next.seek !== previous.seek) {
      this.seek = next.seek;
      this.target = -Infinity;
      this.lastHour = null;
      this.confirmed.clear();
      const hour = this.hourAt(this.now());
      for (const [widget, held] of this.widgets) {
        for (const entry of held.early) this.refuse(widget, entry, 'retired');
        held.early = [];
        const shown = held.shown?.entry, placement = this.lane?.placement(widget);
        if (!shown || !placement) continue;
        const at = placement.pin ?? hour + placement.offset;
        if (!(shown.h <= at && at <= shown.c && shown.lo <= at && at <= shown.hi)) this.clear(held);
      }
    }
    if (next.playing && !this.looping) {
      this.looping = true;
      requestAnimationFrame(() => this.tick());
    }
    if (!next.playing && previous?.playing) this.forwardShown();
    this.stretch(performance.now() - began);
  }

  exact(request: FrameRequest) {
    this.sent.set(request.serial, this.ticks);
    if (!this.playing()) {
      for (const held of this.widgets.values()) if (held.shown) held.shown.forwarded = false;
      this.forwardShown();
    }
    this.lane?.frame({ ...request, installed: [...request.installed, ...this.acks.splice(0)], refused: [...request.refused, ...this.refusals.splice(0)] });
  }

  private tick() {
    const clock = this.clockState;
    if (!clock?.playing || !this.lane) { this.looping = false; return; }
    const began = performance.now();
    this.ticks++;
    const hour = this.hourAt(this.now());
    const nominal = Math.max(1, Math.ceil(clock.speed / clock.frameRate * 1.5));
    if (this.lastHour !== null && hour >= this.lastHour) this.step = Math.min(hour - this.lastHour, nominal);
    this.lastHour = hour;
    let pictures = 0;
    for (const [widget, held] of this.widgets) if (held.early.length) pictures += this.settleEarly(widget, held, hour);
    const lead = Math.min(MAX_AHEAD, Math.max(1, Math.round(this.ahead)));
    const target = Math.max(this.target, hour + this.step * lead);
    this.target = target;
    const serial = ++this.serial;
    this.sent.set(serial, this.ticks);
    if (this.sent.size > 256) for (const key of [...this.sent.keys()].slice(0, 128)) this.sent.delete(key);
    if (pictures) performance.mark(`bp-wpic-${pictures}`);
    this.lane.frame({ type: 'frame', serial, now: hour, target, exact: false, seek: this.seek, speed: clock.speed,
      installed: this.acks.splice(0), refused: this.refusals.splice(0), inspect: [] });
    this.stretch(performance.now() - began);
    requestAnimationFrame(() => this.tick());
  }

  private settleEarly(widget: number, held: Held, hour: number): number {
    const list = held.early;
    held.early = [];
    const placement = this.lane?.placement(widget);
    if (!placement) { for (const entry of list) this.refuse(widget, entry, 'unmounted'); return 0; }
    const at = placement.pin ?? hour + placement.offset;
    const eligible = list.filter(entry => at >= entry.lo && at <= entry.hi).sort((a, b) => b.id - a.id);
    let pictures = 0;
    for (const entry of list) if (!eligible.includes(entry)) pictures += this.install(entry, hour);
    let installedOne = false;
    for (const entry of eligible) {
      if (installedOne) { this.refuse(widget, entry, 'superseded'); continue; }
      const why = this.reject(held, placement, entry, at);
      if (why === null) installedOne = true;
      pictures += this.install(entry, hour);
    }
    return pictures;
  }

  offer(presentation: Presented<ImageBitmap>) {
    const began = performance.now();
    const at = this.sent.get(presentation.serial);
    if (at !== undefined) {
      const sample = Math.min(MAX_AHEAD, Math.max(1, this.ticks + 1 - at));
      this.ahead = Math.min(MAX_AHEAD, Math.max(1, this.ahead * 0.8 + sample * 0.2));
    }
    for (const [route, reset] of presentation.resets) {
      if (reset <= (this.resets.get(route) ?? 0)) continue;
      this.resets.set(route, reset);
      for (const [widget, held] of this.widgets) {
        const shown = held.shown?.entry;
        if (shown?.follows && shown.gen.reset < reset && this.lane?.placement(widget)?.route === route) this.clear(held);
      }
    }
    for (const [route, channel, hour, seek, reset] of presentation.confirmed ?? []) {
      const key = `${route}|${channel}`, held = this.confirmed.get(key);
      if (seek === this.seek && (!held || hour > held.hour)) this.confirmed.set(key, { hour, seek, reset, route, channel });
    }
    const hour = this.hourAt(this.now());
    let pictures = 0;
    for (const entry of presentation.entries) pictures += this.install(entry, hour);
    if (pictures) performance.mark(`bp-wpic-${pictures}`);
    if (!this.playing()) this.forwardShown();
    this.stretch(performance.now() - began);
  }

  private effectiveC(placement: StagePlacement, entry: Entry<ImageBitmap>): number {
    if (!entry.open || placement.route === null || entry.gen.seek !== this.seek || entry.gen.reset !== (this.resets.get(placement.route) ?? 0)) return entry.c;
    return Math.max(entry.c, this.confirmed.get(`${placement.route}|${entry.open}`)?.hour ?? -Infinity);
  }

  private reject(held: Held, placement: StagePlacement, entry: Entry<ImageBitmap>, at: number): string | null {
    if (placement.pin === null) {
      if (entry.gen.seek !== this.seek) return 'stream';
      if (placement.route !== null && entry.gen.reset < (this.resets.get(placement.route) ?? 0)) return 'reset';
    }
    if (placement.owner !== undefined && entry.owner !== undefined && entry.owner !== placement.owner) return 'owner';
    if (entry.binding !== placement.binding) return 'binding';
    const shown = held.shown;
    if (shown && shown.entry.binding === entry.binding && entry.id < shown.entry.id) return 'superseded';
    if (at < entry.lo) return 'early';
    if (at > entry.hi) return 'superseded';
    if (!shown && Object.values(entry.fields).some(value => value === SAME)) return 'kept content';
    const c = this.effectiveC(placement, entry), playing = this.playing();
    if (entry.mode === 'refused' && !playing && shown && at > c && !sameContent(entry.content, shown.content)) return 'uncovered';
    if (entry.mode !== 'picture') return null;
    for (let k = 0; k < entry.surfaces.length; k++) {
      const part = entry.surfaces[k];
      if (part === SAME ? shown?.content[k] !== entry.content[k] : !part || part.width === 0) return 'kept content';
    }
    for (const [key, value] of Object.entries(entry.fields)) if (value === SAME && (!shown || shown.content[key] !== entry.content[key])) return 'kept content';
    if (!playing && shown && at > c && !sameContent(entry.content, shown.content)) return 'uncovered';
    return null;
  }

  private install(entry: Entry<ImageBitmap>, hour: number): number {
    const held = this.widgets.get(entry.widget), placement = this.lane?.placement(entry.widget);
    if (!held || !placement) { this.refuse(entry.widget, entry, 'unmounted'); return 0; }
    const at = placement.pin ?? hour + placement.offset;
    const why = this.reject(held, placement, entry, at);
    if (why === 'early') { this.park(held, entry); return 0; }
    if (why) { this.refuse(entry.widget, entry, why); return 0; }
    const fields: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(entry.fields)) fields[key] = value === SAME ? held.shown?.fields[key] : value;
    let pictures = 0;
    const part = entry.surfaces[0];
    if (entry.mode !== 'picture' || !part) this.clear(held);
    else if (part !== SAME) { this.draw(held, part); pictures++; }
    held.shown = { entry: { ...entry, surfaces: [] }, content: entry.mode === 'picture' ? { ...entry.content } : {}, fields, forwarded: false };
    this.acks.push({ widget: entry.widget, binding: placement.binding, id: entry.id });
    if (entry.mode !== 'picture') this.forwardShown(entry.widget);
    return pictures;
  }

  private park(held: Held, entry: Entry<ImageBitmap>) {
    const kept: Entry<ImageBitmap>[] = [];
    for (const parked of held.early) {
      if (parked.id > entry.id && parked.lo <= entry.lo && parked.hi >= entry.lo) { this.refuse(entry.widget, entry, 'displaced'); return; }
      if (entry.id >= parked.id && entry.lo <= parked.lo && entry.hi >= parked.lo) this.refuse(entry.widget, parked, 'displaced');
      else kept.push(parked);
    }
    kept.push(entry);
    kept.sort((a, b) => a.lo - b.lo || a.id - b.id);
    while (kept.length > MAX_EARLY) this.refuse(entry.widget, kept.pop()!, 'displaced');
    held.early = kept;
  }

  private refuse(widget: number, entry: Entry<ImageBitmap>, reason: string) {
    this.refusals.push({ widget, id: entry.id, reason });
    closeParts(entry);
  }

  private draw(held: Held, bitmap: ImageBitmap) {
    const canvas = held.canvas, context = held.context;
    if (!canvas || !context) { bitmap.close(); return; }
    if (canvas.width !== bitmap.width) canvas.width = bitmap.width;
    if (canvas.height !== bitmap.height) canvas.height = bitmap.height;
    context.transferFromImageBitmap(bitmap);
  }

  private clear(held: Held) {
    held.shown = null;
    held.context?.transferFromImageBitmap(null);
  }

  private forwardShown(only?: number) {
    const entries: Entry<ImageBitmap>[] = [];
    for (const [widget, held] of this.widgets) {
      if (only !== undefined && widget !== only) continue;
      const shown = held.shown;
      if (!shown || shown.forwarded) continue;
      shown.forwarded = true;
      entries.push({ ...shown.entry, surfaces: shown.entry.mode === 'picture' ? [true as unknown as ImageBitmap] : [],
        content: { ...shown.entry.content }, fields: shown.fields, forwarded: true });
    }
    if (!entries.length) return;
    const confirmed = [...this.confirmed.values()].map(box => [box.route, box.channel, box.hour, box.seek, box.reset] as [number, 'followed' | 'panel', number, number, number]);
    this.forward({ type: 'presentation', lane: this.laneIndex, serial: 0, resets: [...this.resets], ...confirmed.length ? { confirmed } : {}, entries });
  }
}
