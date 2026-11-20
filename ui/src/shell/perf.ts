import type { PictureInterval } from '../draw/pictureDate';
import type { PictureSample } from '../draw/notifications';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { attributeCommits, takeCommits } from './commitCount.ts';
import type { CommitCauses } from './commitCount';

/** What the transport saw of one reply, filled in only while measuring. */
export interface QueryTiming { received?: number; parsed?: number; bytes?: number; through?: number }

interface MapStateSample { id: string; hour: number | null; fraction: number | null; sequence: number; requestedSequence: number; loading: boolean;
  behind?: number | null; behindReason?: string | null; error: string | null; mapMode?: string | null; drawnAt?: number | null; hover?: string | null; zoom?: number | null;
  tooltip?: string | null; tooltipDelayMs?: number | null; center?: [number, number] | null }

/** Where a dashboard map should be now, in its own recording's hours. */
export interface MapExpectation { position: number; follow: boolean }

/** A laid-out map view; pinned targets omit `seekSequence`. */
interface MapViewSample { id: string; visible: boolean; seekSequence?: number }

/** A long-animation-frame entry, which the DOM typings may lack. */
interface LongFrameEntry extends PerformanceEntry {
  blockingDuration: number;
  renderStart: number;
  scripts: readonly { duration: number; invoker?: string; sourceURL?: string }[];
}

type PerfRecord = { t: string; at: number; [field: string]: unknown };

/** A widget's work, counted per frame:
 * - `render`: a render of one of its components that reads the clock hour (a discarded transition render counts too);
 * - `hour`: an hour tick handed to it; `held`: a tick its shown answer's hold kept from it; `unheld`: the tick that ended a hold;
 * - `read`: a read it asked for, `ahead` one for a block wholly ahead of the head; `reply`: a landed reply handed to its state, counted in the frame it renders in (a playing
 *   clock's tick); `same`: a landed reply equal to the one shown, dropped then. */
export type WidgetWork = 'render' | 'hour' | 'held' | 'unheld' | 'read' | 'ahead' | 'reply' | 'same';

/** Records counted: how many, their bytes and their time span. */
interface Tally { count: number; bytes: number; from: number; to: number }

const FLUSH_MS = 500;
/** Held for the host at most, buffered and in flight; past it records are dropped and counted. */
const PENDING_BYTES = 16 << 20;
const encoder = new TextEncoder();
const round = (value: number) => Math.round(value * 1000) / 1000;
/** Milliseconds since the Unix epoch, the clock the host and the map write too. */
const epoch = (now: number) => round(performance.timeOrigin + now);
const empty = (): Tally => ({ count: 0, bytes: 0, from: Infinity, to: -Infinity });
function tally(into: Tally, count: number, bytes: number, from: number, to: number): void {
  into.count += count; into.bytes += bytes; into.from = Math.min(into.from, from); into.to = Math.max(into.to, to);
}

/** Runs a large audit in bounded batches while retaining the input order in its result. */
export async function auditInBatches<T, R>(items: readonly T[], concurrency: number, operation: (item: T, index: number) => Promise<R>): Promise<R[]> {
  const results = new Array<R>(items.length), width = Math.max(1, Math.floor(concurrency));
  let next = 0;
  const worker = async () => {
    while (true) {
      const index = next++;
      if (index >= items.length) return;
      results[index] = await operation(items[index]!, index);
    }
  };
  await Promise.all(Array.from({ length: Math.min(width, items.length) }, worker));
  return results;
}

const AUDIT_CONCURRENCY = 8;

/** The hour a product's params ask for, if they name one. */
function paramsHour(params: unknown): number | null {
  if (!params || typeof params !== 'object') return null;
  const named = params as { hour?: unknown; lastHour?: unknown; to?: unknown; at?: unknown; reads?: unknown };
  for (const value of [named.hour, named.lastHour, named.to, named.at]) if (typeof value === 'number') return value;
  const first: unknown = Array.isArray(named.reads) ? named.reads[0] : undefined;
  const hour = first && typeof first === 'object' ? (first as { hour?: unknown }).hour : undefined;
  return typeof hour === 'number' ? hour : null;
}

/** The first hour a product's params ask from, if they name one: a span's start, the hour, or the earliest of the hours
 * listed (places, states, reads, battles). */
export function paramsFrom(params: unknown): number | null {
  if (!params || typeof params !== 'object') return null;
  const named = params as Record<string, unknown>;
  for (const value of [named.from, named.firstHour, named.hour, named.at]) if (typeof value === 'number') return value;
  let first: number | null = null;
  for (const list of [named.places, named.states, named.reads, named.battles]) if (Array.isArray(list)) for (const item of list) {
    const hour = item && typeof item === 'object' ? (item as { hour?: unknown }).hour : undefined;
    if (typeof hour === 'number' && (first === null || hour < first)) first = hour;
  }
  return first;
}

/** Which list a request feeds: its product and params less what moves from one block to the next (the window or page,
 * and the hours), so a list's blocks share it and two lists never do. */
const MOVING = new Set(['window', 'index', 'cursor', 'pageSize', 'from', 'to', 'firstHour', 'lastHour', 'hour', 'at', 'places', 'states', 'reads', 'battles']);
function laneOf(product: unknown, params: unknown): string {
  const kept = params && typeof params === 'object' ? Object.entries(params as Record<string, unknown>).filter(([name]) => !MOVING.has(name)).sort(([a], [b]) => a < b ? -1 : 1) : [];
  return JSON.stringify([product, kept]);
}

/** A chart tile request's address: its chart (branch, family, tags), bin hours and index on the grid. */
function tileOf(product: unknown, params: unknown): { key: string; bins: unknown; index: unknown } | undefined {
  if (product !== 'series-tiles' || !params || typeof params !== 'object') return undefined;
  const named = params as { branchId?: unknown; family?: unknown; tags?: unknown; binHours?: unknown; index?: unknown };
  return { key: JSON.stringify([named.branchId, named.family, named.tags]), bins: named.binHours, index: named.index };
}

export type ExactPicture = (hour: number) => Promise<{ values: ReadonlyMap<string, string>; ranked?: boolean } | null>;
export interface StructureRead { hour: number; rows: () => readonly string[]; exact: () => Promise<readonly string[]> }
interface RowChange { widget: string; source: string; previous: StructureRead; current: StructureRead }
type AuditedPicture = Omit<PictureSample, 'hour' | 'rows'> & { hour: number; rows: NonNullable<PictureSample['rows']> };
type PictureAuditResult = { widget: string; surface: string; placement: number; hour: number; error: string | null };

export class PerfRecorder {
  private readers = new Map<number, { owner: number; exact: ExactPicture }>();
  private auditReaders?: Map<string, ExactPicture>;
  private commitSequence = 0;
  private commitAudit?: { at: number; serial: number; widgets: string[]; causes: object; changes: RowChange[]; input?: string }[];
  private rowChanges: RowChange[] = [];
  private pictureAudit?: { readers?: Map<string, ExactPicture>; pictures: AuditedPicture[]; done: boolean };

  structure(widget: string, source: string, previous: StructureRead, current: StructureRead) {
    if (this.commitAudit && JSON.stringify(previous.rows()) !== JSON.stringify(current.rows()))
      this.rowChanges.push({ widget, source, previous, current });
  }

  reader(id: number, owner: number, exact: ExactPicture) {
    this.readers.set(id, { owner, exact });
    this.auditReaders?.set(id + '/' + owner, exact);
  }
  releaseReader(id: number) { this.readers.delete(id); }
  ink(id: number, visible: boolean) {
    if (this.auditReaders) this.push({ t: 'ink', at: epoch(performance.now()), placement: id, visible });
  }

  async beginAudit() {
    this.rowChanges = [];
    this.commitAudit = [];
    this.auditReaders = new Map();
    for (const [id, { owner, exact }] of this.readers) this.auditReaders.set(id + '/' + owner, exact);
    await (await import('../dashboard/pulled.ts')).capturePictures(true);
  }

  async auditPicturesBatch(limit: number): Promise<{ done: boolean; results: PictureAuditResult[] }> {
    if (!this.pictureAudit) {
      const { capturePicturesBatch } = await import('../dashboard/pulled.ts');
      const readers = this.auditReaders;
      this.auditReaders = undefined;
      const commits = this.commitAudit ?? [];
      this.commitAudit = undefined;
      const checks = new WeakMap<StructureRead, Promise<boolean>>();
      const verify = async (read: StructureRead) => {
        let checked = checks.get(read);
        if (!checked) {
          checked = (async () => {
            try { return JSON.stringify(read.rows()) === JSON.stringify(await read.exact()); }
            catch { return false; }
          })();
          checks.set(read, checked);
        }
        return checked;
      };
      for (const { changes, ...commit } of commits) {
        const verified = await auditInBatches(changes, AUDIT_CONCURRENCY, async c => ({ widget: c.widget, source: c.source, hour: c.current.hour, previousHour: c.previous.hour,
          before: c.previous.rows(), after: c.current.rows(), verified: await verify(c.previous) && await verify(c.current) }));
        this.push({ t: 'commit', ...commit, changes: verified });
      }
      const first = await capturePicturesBatch(2048);
      const pictures: AuditedPicture[] = [];
      for (const record of first.records) {
        this.push({ ...record });
        if (record.t === 'picture' && record.rows && record.hour !== null) pictures.push(record as AuditedPicture);
      }
      this.pictureAudit = { readers, pictures, done: first.done };
    }
    const work = this.pictureAudit;
    const count = Math.max(1, Math.floor(limit));
    if (!work.done && work.pictures.length < count) {
      const { capturePicturesBatch } = await import('../dashboard/pulled.ts');
      const next = await capturePicturesBatch(2048);
      for (const record of next.records) {
        this.push({ ...record });
        if (record.t === 'picture' && record.rows && record.hour !== null) work.pictures.push(record as AuditedPicture);
      }
      work.done = next.done;
    }
    const batch = work.pictures.splice(0, count);
    const results = await auditInBatches(batch, AUDIT_CONCURRENCY, async record => {
      const { widget, surface, placement, owner, rows } = record;
      const hour = record.hour + record.offset;
      let error: string | null = null;
      try {
        const exact = await work.readers?.get(placement + '/' + owner)?.(hour);
        if (!exact || !rows.length) error = 'Displayed rows or exact read missing';
        else if (exact.ranked && JSON.stringify(rows.map(r => r.row + ':' + r.key)) !== JSON.stringify([...exact.values.keys()])) error = 'Ranked row identities differ';
        else for (const row of rows) {
          const key = exact.ranked ? row.row + ':' + row.key : row.key;
          if (row.overflow) { error = key + ' exceeds its slot by ' + row.overflow.toFixed(1) + ' px'; break; }
          if (!exact.ranked && row.row !== key && !key.startsWith(row.row + ':')) { error = 'Wrong row identity for ' + key; break; }
          if (!exact.values.has(key) || exact.values.get(key) !== row.value) {
            error = key + ': displayed ' + row.value + ', expected ' + exact.values.get(key); break;
          }
        }
      } catch (cause) { error = String(cause); }
      return { widget, surface, placement, hour, error };
    });
    const done = work.done && work.pictures.length === 0;
    if (done) this.pictureAudit = undefined;
    return { done, results };
  }
  async auditPictures(): Promise<PictureAuditResult[]> {
    const results: PictureAuditResult[] = [];
    do {
      const batch = await this.auditPicturesBatch(Number.MAX_SAFE_INTEGER);
      results.push(...batch.results);
      if (batch.done) return results;
    } while (true);
  }
  private lines: string[] = [];
  /** `bytes` here are UTF-16 units, the budget's measure. */
  private buffered = empty();
  private sending = 0;
  private lost = empty();
  private stopped = false;
  private writing: Promise<void> = Promise.resolve();
  /** The write waiting behind the one in flight; every flush until it starts shares it. */
  private queued: Promise<void> | null = null;
  private observers: PerformanceObserver[] = [];
  private frameRequest = 0;
  private timer: ReturnType<typeof setInterval>;
  private lastFrame: number | null = null;
  /** Frame records waiting for their frame's rendering to end, which gives their `main`. */
  private waiting: { record: PerfRecord; of: number }[] = [];
  /** Each widget's work since the last frame record. */
  private work = new Map<string, Partial<Record<WidgetWork, number>>>();
  /** Commits since the last frame record by the parts of the page that rendered in them. */
  private zones = new Map<string, number>();
  /** Why each widget's commits since the last frame record began (`CommitCauses`), counted. */
  private causes = new Map<string, Map<string, number>>();
  /** Milliseconds each widget spent painting, by the animation frame's time it painted for (`OUTSIDE`: no frame's). */
  private paints = new Map<number, Map<string, number>>();
  private playing = false;
  private position: number | null = null;
  private views = '';

  constructor() {
    const types = PerformanceObserver.supportedEntryTypes;
    const longtask = types.includes('longtask'), loaf = types.includes('long-animation-frame');
    this.push({ t: 'page', at: epoch(performance.now()), dpr: devicePixelRatio, width: innerWidth, height: innerHeight, longtask, loaf }, true);
    if (longtask) this.observe('longtask', list => {
      for (const entry of list.getEntries()) this.push({ t: 'longtask', at: epoch(entry.startTime), dur: round(entry.duration) });
    });
    if (loaf) this.observe('long-animation-frame', list => {
      for (const entry of list.getEntries() as LongFrameEntry[]) {
        let script = 0, top: LongFrameEntry['scripts'][number] | undefined;
        for (const each of entry.scripts) { script += each.duration; if (!top || each.duration > top.duration) top = each; }
        this.push({ t: 'loaf', at: epoch(entry.startTime), dur: round(entry.duration), blocking: round(entry.blockingDuration), script: round(script),
          render: entry.renderStart > 0 ? round(entry.startTime + entry.duration - entry.renderStart) : null,
          top: top ? `${top.invoker ?? ''} ${top.sourceURL ?? ''}`.trim() : null });
      }
    });
    // A message posted from a frame's callback runs once the main thread has rendered that frame; it names its frame, so a
    // frame whose rendering ends after the next frame began still gets its own time.
    const rendered = new MessageChannel();
    const mains = new Map<number, number>();
    rendered.port1.onmessage = ({ data }: MessageEvent<number>) => {
      const main = round(performance.now() - data);
      const at = this.waiting.findIndex(entry => entry.of === data);
      if (at < 0) { mains.set(data, main); return; }
      const [entry] = this.waiting.splice(at, 1);
      this.push({ ...entry.record, main });
    };
    const frame = (now: number) => {
      if (this.stopped) return;
      if (this.lastFrame !== null) {
        const work = this.work.size ? Object.fromEntries(this.work) : undefined;
        const zones = this.zones.size ? Object.fromEntries(this.zones) : undefined;
        const why = this.causes.size ? Object.fromEntries([...this.causes].map(([zone, causes]) => [zone, Object.fromEntries(causes)])) : undefined;
        this.work.clear();
        this.zones.clear();
        this.causes.clear();
        const painted = this.takePaints(this.lastFrame);
        const record: PerfRecord = { t: 'frame', at: epoch(now), dt: round(now - this.lastFrame), playing: this.playing,
          late: round(performance.now() - now), commits: takeCommits(), commitSequence: this.commitSequence, ...(work && { work }), ...(zones && { zones }), ...(why && { why }), ...painted };
        const main = mains.get(this.lastFrame);
        if (main !== undefined) { mains.delete(this.lastFrame); this.push({ ...record, main }); } else this.waiting.push({ record, of: this.lastFrame });
      } else { takeCommits(); this.zones.clear(); this.causes.clear(); this.paints.clear(); }
      this.lastFrame = now;
      rendered.port2.postMessage(now);
      this.frameRequest = requestAnimationFrame(frame);
    };
    this.frameRequest = requestAnimationFrame(frame);
    this.timer = setInterval(() => { void this.flush(); }, FLUSH_MS);
    attributeCommits((rendered, causes) => {
      this.commitSequence++;
      if (this.commitAudit) {
        const event = window.event;
        const input = event?.isTrusted && /^(click|key(down|up)|pointer(down|up)|input|change)$/.test(event.type) ? event.type : undefined;
        this.commitAudit.push({ at: epoch(performance.now()), serial: this.commitSequence, widgets: [...rendered].map(z => z.replace(/^widget:/, '')),
          causes: Object.fromEntries([...causes].map(([z, keys]) => [z, Object.fromEntries(keys)])), changes: this.rowChanges, input });
        this.rowChanges = [];
      }
      for (const zone of rendered) this.zones.set(zone, (this.zones.get(zone) ?? 0) + 1);
      this.addCauses(causes);
    });
  }

  private addCauses(causes: CommitCauses): void {
    for (const [zone, named] of causes) {
      let held = this.causes.get(zone);
      if (!held) this.causes.set(zone, held = new Map());
      for (const [cause, count] of named) held.set(cause, (held.get(cause) ?? 0) + count);
    }
  }

  /** The painting of the frame begun at `frame` and of every earlier or frameless paint, for its record: `paint`, ms by
   * widget, and `paint_ms`, their sum (what is left of `main` is the browser's rendering and other tasks). */
  private takePaints(frame: number): { paint?: Record<string, number>; paint_ms?: number } {
    if (!this.paints.size) return {};
    const by = new Map<string, number>();
    for (const [at, widgets] of this.paints) {
      if (at > frame) continue;
      this.paints.delete(at);
      for (const [widget, ms] of widgets) by.set(widget, (by.get(widget) ?? 0) + ms);
    }
    if (!by.size) return {};
    let total = 0;
    const paint: Record<string, number> = {};
    for (const [widget, ms] of by) { total += ms; paint[widget] = Math.round(ms * 100) / 100; }
    return { paint, paint_ms: Math.round(total * 100) / 100 };
  }

  private observe(type: string, callback: PerformanceObserverCallback): void {
    const observer = new PerformanceObserver(callback);
    observer.observe({ type });
    this.observers.push(observer);
  }

  /** Buffers a record; `kept` records (markers) never count against the budget. */
  private push(record: PerfRecord, kept = false): void {
    if (this.stopped) return;
    const line = JSON.stringify(record);
    if (!kept && this.buffered.bytes + this.sending + line.length + 1 > PENDING_BYTES) {
      tally(this.lost, 1, encoder.encode(line).length + 1, record.at, record.at);
      return;
    }
    this.lines.push(line);
    tally(this.buffered, 1, line.length + 1, record.at, record.at);
  }

  /** The playback clock published a position. */
  clock(position: number, playing: boolean, hoursPerSecond: number, sequence: number, seekSequence: number, composedAt?: number): void {
    this.playing = playing;
    this.position = position;
    this.push({ t: 'tick', at: epoch(performance.now()), hour: round(position), composedAt: composedAt === undefined ? undefined : epoch(composedAt), playing, speed: hoursPerSecond, sequence, seek: seekSequence });
  }

  /** A widget on the dashboard, named so its counted work can be read by title. */
  widget(id: string, kind: string, title: string): void {
    this.push({ t: 'widget', at: epoch(performance.now()), id, kind, title }, true);
  }

  /** A widget that failed to render, with the components it failed in. */
  widgetError(widget: string | null, message: string, stack: string | null): void {
    this.push({ t: 'widget_error', at: epoch(performance.now()), widget, message, stack }, true);
  }

  /** One piece of a widget's work, written with the next frame record. */
  count(widget: string, work: WidgetWork): void {
    let counts = this.work.get(widget);
    if (!counts) this.work.set(widget, counts = {});
    counts[work] = (counts[work] ?? 0) + 1;
  }

  /** `ms` of `widget`'s painting for the animation frame whose callbacks were given `frame`; `OUTSIDE` for painting in
   * no frame (a message landing, a timer), written with the next frame record. */
  paint(widget: string, ms: number, frame: number): void {
    let widgets = this.paints.get(frame);
    if (!widgets) this.paints.set(frame, widgets = new Map());
    widgets.set(widget, (widgets.get(widget) ?? 0) + ms);
  }

  /** The hour whose values a widget now shows; null while it shows none (loading, or values still read). */
  present(widget: string, surface: string, date: PictureInterval | null, head: number, renderer: 'worker' | 'dom' | 'race',
    evidence?: { id?: number; seek?: number; rows?: readonly { row: string; key: string; value?: string }[]; follow?: number; readHour?: number; cellHour?: number; at?: number; offset?: number; fixedHour?: number }) {
    const hour = date === null ? null : Math.max(date.since, Math.min(date.through, Math.floor(head)));
    this.push({ t: 'picture', at: epoch(performance.now()), widget, surface, renderer, hour, head,
      placement: evidence?.id, seek: evidence?.seek, rows: evidence?.rows, drawnAt: evidence?.at, fixedHour: evidence?.fixedHour, offset: evidence?.offset, follow: evidence?.follow, readHour: evidence?.readHour, cellHour: evidence?.cellHour });
  }

  shown(widget: string, hour: number | null): void {
    this.push({ t: 'shown', at: epoch(performance.now()), widget, hour });
  }

  /** A followed query's state: asked (`pending`, naming its product), answered, or let go. */
  follow(id: number, status: 'pending' | 'followed' | 'unavailable' | 'released', product?: string | null, reason?: string): void {
    this.push({ t: 'follow', at: epoch(performance.now()), id, status, ...(product !== undefined && { product }), ...(reason !== undefined && { reason }) });
  }

  /** A widget holds the followed query `id`. */
  followWidget(widget: string, id: number): void {
    this.push({ t: 'follow_widget', at: epoch(performance.now()), widget, id });
  }

  /** The dashboard maps laid out, recorded when which are visible or following changes. */
  mapViews(views: readonly MapViewSample[]): void {
    let key = '';
    for (const view of views) key += `${view.id}\u0000${view.visible ? 1 : 0}${view.seekSequence === undefined ? 0 : 1}\u0000`;
    if (key === this.views) return;
    this.views = key;
    this.push({ t: 'map_views', at: epoch(performance.now()),
      views: views.map(view => ({ id: view.id, visible: view.visible, follow: view.seekSequence !== undefined })) });
  }

  /** The native map acknowledged what it presents, beside where each map should be now. */
  mapStates(states: readonly MapStateSample[], expect: (id: string) => MapExpectation | null): void {
    const at = epoch(performance.now());
    for (const state of states) {
      const expected = expect(state.id);
      this.push({ t: 'map_state', at, view: state.id, hour: state.hour, fraction: state.fraction, seq: state.sequence, req: state.requestedSequence,
        loading: state.loading, behind: state.behind ?? null, behind_reason: state.behindReason ?? null,
        mode: state.mapMode ?? null, drawn_at: state.drawnAt ?? null, hover: state.hover ?? null, zoom: state.zoom ?? null,
        tooltip: state.tooltip ?? null, tooltip_delay_ms: state.tooltipDelayMs ?? null, center: state.center ?? null,
        blank: state.error != null, playing: this.playing,
        follow: expected?.follow ?? null, expected: expected ? round(expected.position) : null });
    }
  }

  /** The hour of the clock's last published position: the head a request is sent at. */
  head(): number | null {
    return this.position === null ? null : Math.floor(this.position);
  }

  /** Whether `request` asks for a block wholly ahead of the head now: the first hour it asks from is past it. */
  ahead(request: { params: unknown; asksFrom?: number }): boolean {
    const from = request.asksFrom ?? paramsFrom(request.params), head = this.head();
    return from !== null && head !== null && from > head;
  }

  /** One request to the host, from send to its parsed reply or failure, with the failure's kind and text. A product names
   * the first hour it asks from (`from`: its own `asksFrom`, else its params') and the head it was sent at (`head`), and the
   * list it feeds (`lane`), so a request for a block wholly ahead of the head (`from` > `head`) is told from one at it. */
  query(command: string, body: Record<string, unknown>, id: string, started: number, timing: QueryTiming, outcome: 'ok' | 'cancelled' | 'error', failure?: string,
    asked: { from: number | null; head: number | null } = { from: null, head: null }): void {
    const query = body.query as { product?: unknown; params?: unknown } | undefined;
    const { received, parsed, bytes, through } = timing;
    const tile = tileOf(query?.product, query?.params);
    const from = query ? asked.from ?? paramsFrom(query.params) : null;
    this.push({ t: 'q', at: epoch(started), id, cmd: command, product: typeof query?.product === 'string' ? query.product : command,
      hour: paramsHour(query?.params), ...(from !== null && { from }), ...(query && asked.head !== null && { head: asked.head }),
      ...(query && { lane: laneOf(query.product, query.params) }), ...(tile && { tile }), refresh: typeof body.refresh === 'boolean' ? body.refresh : null, outcome,
      ms: round(performance.now() - started), ipc_ms: received === undefined ? null : round(received - started),
      parse_ms: received === undefined || parsed === undefined ? null : round(parsed - received), bytes: bytes ?? null,
      ...(through !== undefined && { through }), ...(failure !== undefined && { failure }) });
  }

  /** A product answered by the page's own cache (`hit`) or by joining a read in flight (`joined`). */
  cached(request: { product: string; params: unknown }, cache: 'hit' | 'joined'): void {
    this.push({ t: 'q', at: epoch(performance.now()), product: request.product, hour: paramsHour(request.params), cache });
  }

  /** Writes what is buffered once the write in flight ends; resolves when it is with the host. */
  flush(): Promise<void> {
    if (this.queued) return this.queued;
    const queued = this.writing.then(() => { this.queued = null; return this.send(); });
    this.queued = queued;
    this.writing = queued;
    return queued;
  }

  private async send(): Promise<void> {
    const carried = this.lost.count ? this.lost : null;
    if (carried) this.lost = empty();
    if (!this.lines.length && !carried) return;
    const batch = this.buffered, records = this.lines.join('\n');
    this.lines = []; this.buffered = empty(); this.sending = batch.bytes;
    const loss = carried && JSON.stringify({ t: 'lost', at: epoch(performance.now()), source: 'ui', count: carried.count, bytes: carried.bytes,
      from: round(carried.from), to: round(carried.to) });
    try {
      await invoke<void>('perf_write', { lines: loss && records ? `${loss}\n${records}` : loss || records });
    } catch (error) {
      console.error('Measurement write failed:', error);
      if (batch.count) tally(this.lost, batch.count, encoder.encode(records).length + 1, batch.from, batch.to);
      if (carried) tally(this.lost, carried.count, carried.bytes, carried.from, carried.to);
    } finally { this.sending = 0; }
  }

  /** Summaries cover from the last start to their stop. */
  start(): Promise<void> {
    this.push({ t: 'start', at: epoch(performance.now()) }, true);
    return this.flush();
  }

  /** Every frame before now gets its main-thread time, or says it has none: the rendered message of a frame comes after
   * its rendering, so a frame and a task are enough for every one begun. */
  private async settleFrames(): Promise<void> {
    await new Promise(resolve => requestAnimationFrame(() => setTimeout(resolve, 0)));
    for (const { record } of this.waiting.splice(0)) this.push({ ...record, main: null });
  }

  /** Everything up to the stop reaches the host before it waits for the maps and summarizes. */
  async stop(): Promise<unknown> {
    await this.settleFrames();
    this.push({ t: 'stop', at: epoch(performance.now()) }, true);
    await this.flush();
    return invoke('perf_stop');
  }

  /** Stops recording for good and writes what is held; true when nothing was lost doing so. */
  async finish(): Promise<boolean> {
    await this.settleFrames();
    this.stopped = true;
    clearInterval(this.timer);
    attributeCommits(null);
    cancelAnimationFrame(this.frameRequest);
    for (const observer of this.observers) observer.disconnect();
    await this.flush();
    return !this.lost.count && !this.lines.length;
  }
}

/** Set only in a run the host measures (`BATTLE_PLAN_PERF_LOG`); hot paths check it and do nothing else. */
export let perfLog: PerfRecorder | null = null;

/** The frame time of painting done in no animation frame. */
export const OUTSIDE = -Infinity;
/** What painting no widget owns is counted as. */
export const PAGE_PAINT = 'page';

/** Runs `paint` as `widget`'s painting for the frame begun at `frame`, timed only while measuring. */
export function timePaint(widget: string | null, frame: number, paint: () => void): void {
  const recorder = perfLog;
  if (!recorder) { paint(); return; }
  const started = performance.now();
  try { paint(); } finally { recorder.paint(widget ?? PAGE_PAINT, performance.now() - started, frame); }
}

/** A page measured by something other than the host (a DOM test reading what widgets report): its recorder. */
export function measureWith(recorder: PerfRecorder | null): void { perfLog = recorder; }

export async function startPerf(): Promise<void> {
  if (!isTauri() || perfLog || !await invoke<boolean>('perf_enabled')) return;
  const recorder = new PerfRecorder();
  perfLog = recorder;
  // The host's exit asks for the last records once its service and viewers have stopped.
  await listen('perf-finish', () => {
    void recorder.finish().then(complete => invoke('perf_finished', { complete }))
      .catch(error => console.error('Measurement finish failed:', error));
  });
  // Driven from the debugger: `await window.__perf.start()`, `await window.__perf.stop()`.
  Object.assign(window, { __perf: { start: () => recorder.start(), stop: () => recorder.stop(), flush: () => recorder.flush(),
    beginAudit: () => recorder.beginAudit(), auditPictures: () => recorder.auditPictures(), auditPicturesBatch: (limit: number) => recorder.auditPicturesBatch(limit),
    pressureMerges: () => invoke('playback_pressure_merges') } });
}
