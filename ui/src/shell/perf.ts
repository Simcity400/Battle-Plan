import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

/** What the transport saw of one reply, filled in only while measuring. */
export interface QueryTiming { received?: number; parsed?: number; bytes?: number }

interface MapStateSample { id: string; hour: number | null; fraction: number | null; sequence: number; requestedSequence: number; loading: boolean;
  error: string | null; pending?: string | null }

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

/** The hour a product's params ask for, if they name one. */
function paramsHour(params: unknown): number | null {
  if (!params || typeof params !== 'object') return null;
  const named = params as { hour?: unknown; lastHour?: unknown; to?: unknown; at?: unknown; reads?: unknown };
  for (const value of [named.hour, named.lastHour, named.to, named.at]) if (typeof value === 'number') return value;
  const first: unknown = Array.isArray(named.reads) ? named.reads[0] : undefined;
  const hour = first && typeof first === 'object' ? (first as { hour?: unknown }).hour : undefined;
  return typeof hour === 'number' ? hour : null;
}

export class PerfRecorder {
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
  private playing = false;
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
    const frame = (now: number) => {
      if (this.stopped) return;
      if (this.lastFrame !== null) this.push({ t: 'frame', at: epoch(now), dt: round(now - this.lastFrame), playing: this.playing });
      this.lastFrame = now;
      this.frameRequest = requestAnimationFrame(frame);
    };
    this.frameRequest = requestAnimationFrame(frame);
    this.timer = setInterval(() => { void this.flush(); }, FLUSH_MS);
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
  clock(position: number, playing: boolean, hoursPerSecond: number, sequence: number, seekSequence: number): void {
    this.playing = playing;
    this.push({ t: 'tick', at: epoch(performance.now()), hour: round(position), playing, speed: hoursPerSecond, sequence, seek: seekSequence });
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
        loading: state.loading, blank: state.error != null || state.pending != null, playing: this.playing,
        follow: expected?.follow ?? null, expected: expected ? round(expected.position) : null });
    }
  }

  /** One request to the host, from send to its parsed reply or failure. */
  query(command: string, body: Record<string, unknown>, id: string, started: number, timing: QueryTiming, outcome: 'ok' | 'cancelled' | 'error'): void {
    const query = body.query as { product?: unknown; params?: unknown } | undefined;
    const { received, parsed, bytes } = timing;
    this.push({ t: 'q', at: epoch(started), id, cmd: command, product: typeof query?.product === 'string' ? query.product : command,
      hour: paramsHour(query?.params), refresh: typeof body.refresh === 'boolean' ? body.refresh : null, outcome,
      ms: round(performance.now() - started), ipc_ms: received === undefined ? null : round(received - started),
      parse_ms: received === undefined || parsed === undefined ? null : round(parsed - received), bytes: bytes ?? null });
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

  /** Everything up to the stop reaches the host before it waits for the maps and summarizes. */
  async stop(): Promise<unknown> {
    this.push({ t: 'stop', at: epoch(performance.now()) }, true);
    await this.flush();
    return invoke('perf_stop');
  }

  /** Stops recording for good and writes what is held; true when nothing was lost doing so. */
  async finish(): Promise<boolean> {
    this.stopped = true;
    clearInterval(this.timer);
    cancelAnimationFrame(this.frameRequest);
    for (const observer of this.observers) observer.disconnect();
    await this.flush();
    return !this.lost.count && !this.lines.length;
  }
}

/** Set only in a run the host measures (`BATTLE_PLAN_PERF_LOG`); hot paths check it and do nothing else. */
export let perfLog: PerfRecorder | null = null;

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
  Object.assign(window, { __perf: { start: () => recorder.start(), stop: () => recorder.stop(), flush: () => recorder.flush() } });
}
