import type { PictureSample } from '../draw/notifications';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { attributeCommits, takeCommits } from './commitCount.ts';
import type { CommitCauses } from './commitCount';

export interface QueryTiming { received?: number; parsed?: number; bytes?: number; through?: number }

interface MapStateSample { id: string; hour: number | null; fraction: number | null; sequence: number; requestedSequence: number; loading: boolean;
  behind?: number | null; behindReason?: string | null; error: string | null; mapMode?: string | null; drawnAt?: number | null; hover?: string | null; zoom?: number | null;
  tooltip?: string | null; tooltipDelayMs?: number | null; center?: [number, number] | null }

export interface MapExpectation { position: number; follow: boolean }

interface MapViewSample { id: string; visible: boolean; seekSequence?: number }

interface LongFrameEntry extends PerformanceEntry {
  blockingDuration: number;
  renderStart: number;
  scripts: readonly { duration: number; invoker?: string; sourceURL?: string }[];
}

type PerfRecord = { t: string; at: number; [field: string]: unknown };

export type WidgetWork = 'render' | 'hour' | 'held' | 'unheld' | 'read' | 'ahead' | 'reply' | 'same';

interface Tally { count: number; bytes: number; from: number; to: number }

const FLUSH_MS = 500;
const FLUSH_BYTES = 2 << 20;
const PENDING_BYTES = 16 << 20;
const MEASURE_UNGATED = typeof __BATTLEPLAN_MEASURE_UNGATED__ !== 'undefined' && __BATTLEPLAN_MEASURE_UNGATED__;
const encoder = new TextEncoder();
const round = (value: number) => Math.round(value * 1000) / 1000;
const epoch = (now: number) => round(performance.timeOrigin + now);
const empty = (): Tally => ({ count: 0, bytes: 0, from: Infinity, to: -Infinity });
function tally(into: Tally, count: number, bytes: number, from: number, to: number): void {
  into.count += count; into.bytes += bytes; into.from = Math.min(into.from, from); into.to = Math.max(into.to, to);
}

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

function paramsHour(params: unknown): number | null {
  if (!params || typeof params !== 'object') return null;
  const named = params as { hour?: unknown; lastHour?: unknown; to?: unknown; at?: unknown; reads?: unknown };
  for (const value of [named.hour, named.lastHour, named.to, named.at]) if (typeof value === 'number') return value;
  const first: unknown = Array.isArray(named.reads) ? named.reads[0] : undefined;
  const hour = first && typeof first === 'object' ? (first as { hour?: unknown }).hour : undefined;
  return typeof hour === 'number' ? hour : null;
}

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

const MOVING = new Set(['window', 'index', 'cursor', 'pageSize', 'from', 'to', 'firstHour', 'lastHour', 'hour', 'at', 'places', 'states', 'reads', 'battles']);
function laneOf(product: unknown, params: unknown): string {
  const kept = params && typeof params === 'object' ? Object.entries(params as Record<string, unknown>).filter(([name]) => !MOVING.has(name)).sort(([a], [b]) => a < b ? -1 : 1) : [];
  return JSON.stringify([product, kept]);
}

function tileOf(product: unknown, params: unknown): { key: string; bins: unknown; index: unknown } | undefined {
  if (product !== 'series-tiles' || !params || typeof params !== 'object') return undefined;
  const named = params as { branchId?: unknown; family?: unknown; countries?: unknown; binHours?: unknown; index?: unknown };
  return { key: JSON.stringify([named.branchId, named.family, named.countries]), bins: named.binHours, index: named.index };
}

export interface ExactRead { values: ReadonlyMap<string, string>; ranked?: boolean; ordered?: boolean }
export type ExactPicture = (hour: number) => Promise<ExactRead | null>;
type ShownRow = NonNullable<PictureSample['rows']>[number];

export function pictureError(surface: string, rows: readonly ShownRow[], exact: ExactRead | null | undefined): string | null {
  if (!exact || (!rows.length && exact.values.size)) return 'Displayed rows or exact read missing';
  if (exact.ranked && JSON.stringify(rows.map(r => r.row + ':' + r.key)) !== JSON.stringify([...exact.values.keys()])) return 'Ranked row identities differ';
  if (exact.ordered && JSON.stringify(rows.map(r => r.key)) !== JSON.stringify([...exact.values.keys()])) return 'Displayed rows are not in the exact read\'s order';
  for (const row of rows) {
    const key = exact.ranked ? row.row + ':' + row.key : row.key;
    if (row.overflow) return key + ' exceeds its slot by ' + row.overflow.toFixed(1) + ' px';
    if (!exact.ranked && row.row !== key && !key.startsWith(row.row + ':')) return 'Wrong row identity for ' + key;
    if (!exact.values.has(key) || exact.values.get(key) !== row.value) return key + ': displayed ' + row.value + ', expected ' + exact.values.get(key);
  }
  const shown = new Set(rows.map(row => exact.ranked ? row.row + ':' + row.key : row.key));
  for (const key of exact.values.keys()) if (!shown.has(key)) return key + ': in the exact read, not shown';
  if (surface === 'table' && shown.size !== exact.values.size)
    return 'Displayed ' + rows.length + ' rows of the ' + exact.values.size + ' the exact read holds';
  return null;
}
export interface StructureRead { hour: number; rows: () => readonly string[]; exact: () => Promise<readonly string[]> }
interface RowChange { widget: string; source: string; previous: StructureRead; current: StructureRead; placement?: number }
type AuditedPicture = Omit<PictureSample, 'hour' | 'rows'> & { hour: number; rows: NonNullable<PictureSample['rows']> };
type PictureAuditResult = { widget: string; surface: string; placement: number; owner?: number; at: number; hour: number; error: string | null };

const READERS_HELD = 1024;
const EXACT_READS_HELD = 64;

export class PerfRecorder {
  private readers = new Map<string, ExactPicture>();
  private auditReaders?: Map<string, ExactPicture>;
  private commitSequence = 0;
  private commitAudit?: { at: number; serial: number; widgets: string[]; causes: object; changes: RowChange[]; input?: string }[];
  private rowChanges: RowChange[] = [];
  private pictureAudit?: { readers?: Map<string, ExactPicture>; pictures: AuditedPicture[]; done: boolean };

  structure(widget: string, source: string, previous: StructureRead, current: StructureRead, placement?: number) {
    if (this.commitAudit && JSON.stringify(previous.rows()) !== JSON.stringify(current.rows()))
      this.rowChanges.push({ widget, source, previous, current, ...placement !== undefined ? { placement } : {} });
  }

  reader(id: number, owner: number, exact: ExactPicture) {
    const key = id + '/' + owner;
    this.readers.delete(key);
    this.readers.set(key, exact);
    if (this.readers.size > READERS_HELD) this.readers.delete(this.readers.keys().next().value!);
    this.auditReaders?.set(key, exact);
  }
  private exactReads = new Map<string, Promise<unknown>>();
  exactOnce<T>(key: string, read: () => Promise<T>): Promise<T> {
    let held = this.exactReads.get(key) as Promise<T> | undefined;
    if (held) {
      this.exactReads.delete(key);
      this.exactReads.set(key, held);
      return held;
    }
    held = read();
    this.exactReads.set(key, held);
    if (this.exactReads.size > EXACT_READS_HELD) this.exactReads.delete(this.exactReads.keys().next().value!);
    held.catch(() => this.exactReads.delete(key));
    return held;
  }
  ink(id: number, visible: boolean) {
    if (this.auditReaders) this.push({ t: 'ink', at: epoch(performance.now()), placement: id, visible });
  }

  async beginAudit() {
    this.rowChanges = [];
    this.commitAudit = [];
    this.auditReaders = new Map(this.readers);
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
        const verified = await auditInBatches(changes, AUDIT_CONCURRENCY, async c => ({ widget: c.widget, source: c.source, ...c.placement !== undefined ? { placement: c.placement } : {}, hour: c.current.hour, previousHour: c.previous.hour,
          before: c.previous.rows(), after: c.current.rows(), verified: await verify(c.previous) && await verify(c.current) }));
        this.push({ t: 'commit', ...commit, changes: verified });
        if (this.buffered.bytes >= FLUSH_BYTES) await this.flush();
      }
      const first = await capturePicturesBatch(256);
      const pictures: AuditedPicture[] = [];
      for (const record of first.records) {
        const logged: PerfRecord = { ...record };
        delete logged.rows;
        this.push(logged);
        if (record.t === 'picture' && record.rows && record.hour !== null) pictures.push(record as AuditedPicture);
      }
      if (this.buffered.bytes >= FLUSH_BYTES) await this.flush();
      this.pictureAudit = { readers, pictures, done: first.done };
    }
    const work = this.pictureAudit;
    const count = Math.max(1, Math.floor(limit));
    while (!work.done && work.pictures.length < count) {
      const { capturePicturesBatch } = await import('../dashboard/pulled.ts');
      const next = await capturePicturesBatch(256);
      for (const record of next.records) {
        const logged: PerfRecord = { ...record };
        delete logged.rows;
        this.push(logged);
        if (record.t === 'picture' && record.rows && record.hour !== null) work.pictures.push(record as AuditedPicture);
      }
      if (this.buffered.bytes >= FLUSH_BYTES) await this.flush();
      work.done = next.done;
    }
    const batch = work.pictures.splice(0, count);
    const results = await auditInBatches(batch, AUDIT_CONCURRENCY, async record => {
      const { widget, surface, placement, owner, rows } = record;
      const hour = record.hour + record.offset;
      let error: string | null;
      try { error = pictureError(surface, rows, await work.readers?.get(placement + '/' + owner)?.(hour)); }
      catch (cause) { error = String(cause); }
      return { widget, surface, placement, owner, at: record.at, hour, error };
    });
    const done = work.done && work.pictures.length === 0;
    if (done) { this.pictureAudit = undefined; this.exactReads.clear(); }
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
  private buffered = empty();
  private sending = 0;
  private lost = empty();
  private stopped = false;
  private writing: Promise<void> = Promise.resolve();
  private queued: Promise<void> | null = null;
  private observers: PerformanceObserver[] = [];
  private frameRequest = 0;
  private timer: ReturnType<typeof setInterval>;
  private lastFrame: number | null = null;
  private waiting: { record: PerfRecord; of: number }[] = [];
  private work = new Map<string, Partial<Record<WidgetWork, number>>>();
  private zones = new Map<string, number>();
  private causes = new Map<string, Map<string, number>>();
  private paints = new Map<number, Map<string, number>>();
  private playing = false;
  private position: number | null = null;
  private views = '';
  private laneCountValue: number | null = null;

  constructor() {
    const types = PerformanceObserver.supportedEntryTypes;
    const longtask = types.includes('longtask'), loaf = types.includes('long-animation-frame');
    this.push({ t: 'page', at: epoch(performance.now()), dpr: devicePixelRatio, width: innerWidth, height: innerHeight, longtask, loaf,
      hardwareConcurrency: navigator.hardwareConcurrency, ...(MEASURE_UNGATED && { ungated: true }) }, true);
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

  private push(record: PerfRecord, kept = false): void {
    if (this.stopped) return;
    const line = JSON.stringify(record);
    if (!kept && this.buffered.bytes + this.sending + line.length + 1 > PENDING_BYTES) {
      tally(this.lost, 1, encoder.encode(line).length + 1, record.at, record.at);
      return;
    }
    this.lines.push(line);
    tally(this.buffered, 1, line.length + 1, record.at, record.at);
    if (!kept && this.buffered.bytes >= FLUSH_BYTES && !this.queued) void this.flush();
  }

  get hour(): number | null { return this.position === null ? null : Math.floor(this.position); }
  get laneCount(): number | null { return this.laneCountValue; }
  lanes(count: number): void { this.laneCountValue = count; }

  clock(position: number, playing: boolean, hoursPerSecond: number, sequence: number, seekSequence: number, composedAt?: number): void {
    this.playing = playing;
    this.position = position;
    this.push({ t: 'tick', at: epoch(performance.now()), hour: round(position), composedAt: composedAt === undefined ? undefined : epoch(composedAt), playing, speed: hoursPerSecond, sequence, seek: seekSequence });
  }

  widget(id: string, kind: string, title: string): void {
    this.push({ t: 'widget', at: epoch(performance.now()), id, kind, title }, true);
  }

  widgetError(widget: string | null, message: string, stack: string | null): void {
    this.push({ t: 'widget_error', at: epoch(performance.now()), widget, message, stack }, true);
  }

  count(widget: string, work: WidgetWork): void {
    let counts = this.work.get(widget);
    if (!counts) this.work.set(widget, counts = {});
    counts[work] = (counts[work] ?? 0) + 1;
  }

  paint(widget: string, ms: number, frame: number): void {
    let widgets = this.paints.get(frame);
    if (!widgets) this.paints.set(frame, widgets = new Map());
    widgets.set(widget, (widgets.get(widget) ?? 0) + ms);
  }

  shown(widget: string, hour: number | null): void {
    this.push({ t: 'shown', at: epoch(performance.now()), widget, hour });
  }

  follow(id: number, status: 'pending' | 'followed' | 'unavailable' | 'failed' | 'released', product?: string | null, reason?: string): void {
    this.push({ t: 'follow', at: epoch(performance.now()), id, status, ...(product !== undefined && { product }), ...(reason !== undefined && { reason }) });
  }

  followWidget(widget: string, id: number): void {
    this.push({ t: 'follow_widget', at: epoch(performance.now()), widget, id });
  }

  mapViews(views: readonly MapViewSample[]): void {
    let key = '';
    for (const view of views) key += `${view.id}\u0000${view.visible ? 1 : 0}${view.seekSequence === undefined ? 0 : 1}\u0000`;
    if (key === this.views) return;
    this.views = key;
    this.push({ t: 'map_views', at: epoch(performance.now()),
      views: views.map(view => ({ id: view.id, visible: view.visible, follow: view.seekSequence !== undefined })) });
  }

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

  head(): number | null {
    return this.position === null ? null : Math.floor(this.position);
  }

  ahead(request: { params: unknown; asksFrom?: number }): boolean {
    const from = request.asksFrom ?? paramsFrom(request.params), head = this.head();
    return from !== null && head !== null && from > head;
  }

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

  cached(request: { product: string; params: unknown }, cache: 'hit' | 'joined'): void {
    this.push({ t: 'q', at: epoch(performance.now()), product: request.product, hour: paramsHour(request.params), cache });
  }

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

  start(): Promise<void> {
    this.push({ t: 'start', at: epoch(performance.now()) }, true);
    return this.flush();
  }

  private async settleFrames(): Promise<void> {
    await new Promise(resolve => requestAnimationFrame(() => setTimeout(resolve, 0)));
    for (const { record } of this.waiting.splice(0)) this.push({ ...record, main: null });
  }

  async stop(): Promise<unknown> {
    await this.settleFrames();
    this.push({ t: 'stop', at: epoch(performance.now()) }, true);
    await this.flush();
    return invoke('perf_stop');
  }

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

export let perfLog: PerfRecorder | null = null;

export const OUTSIDE = -Infinity;
export const PAGE_PAINT = 'page';

export function timePaint(widget: string | null, frame: number, paint: () => void): void {
  const recorder = perfLog;
  if (!recorder) { paint(); return; }
  const started = performance.now();
  try { paint(); } finally { recorder.paint(widget ?? PAGE_PAINT, performance.now() - started, frame); }
}

export function measureWith(recorder: PerfRecorder | null): void { perfLog = recorder; }

export async function startPerf(): Promise<void> {
  if (!isTauri() || perfLog || !await invoke<boolean>('perf_enabled')) return;
  const recorder = new PerfRecorder();
  perfLog = recorder;
  await listen('perf-finish', () => {
    void recorder.finish().then(complete => invoke('perf_finished', { complete }))
      .catch(error => console.error('Measurement finish failed:', error));
  });
  Object.assign(window, { __perf: { start: () => recorder.start(), stop: () => recorder.stop(), flush: () => recorder.flush(),
    beginAudit: () => recorder.beginAudit(), auditPictures: () => recorder.auditPictures(), auditPicturesBatch: (limit: number) => recorder.auditPicturesBatch(limit),
    pressureMerges: () => invoke('playback_pressure_merges'), hour: () => recorder.hour, lanes: () => recorder.laneCount, ungated: MEASURE_UNGATED } });
}
