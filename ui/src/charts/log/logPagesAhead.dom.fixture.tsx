/** A paged log following the head at 1,440 h/s in a real browser, as dense as run 13's event feed (a row every three
 * hours, pages of 64) and its pages as slow (each reply lands 60 frames, 900 game hours, after it is asked): every page
 * request with the head it was sent at and the hour it asks from. */
import { Profiler, useMemo, useSyncExternalStore } from 'react';
import { createRoot } from 'react-dom/client';
import type { Root } from 'react-dom/client';
import LogList from './LogList';
import type { LogRow } from './logData';
import type { LogReplyRow } from './logSource';
import { useRevealClock } from '../reveal';
import { useLogPages } from '../../dashboard/useLogPages';
import type { LogCut, LogSpec } from '../../dashboard/useLogPages';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import { busyCount, busyKinds } from '../../shell/busy.ts';
import '../../styles.css';

const native = window.requestAnimationFrame.bind(window);
const frame = () => new Promise<void>(resolve => native(() => resolve()));
async function settle(frames = 8) { for (let at = 0; at < frames; at++) await frame(); }

const HOURS = 120_000;
const HEAD = 30_000;
const ROW_HOURS = 3;
const LATENCY_FRAMES = 60;
const source = { id: 'source', revision: '1', metadata: { countries: [], branches: [{ id: '1', firstHour: 0, lastHour: HOURS - 1, gaps: [] }] } } as unknown as DashboardSource;

class Clock {
  position = HEAD;
  playing = false;
  hoursPerSecond = 1440;
  private readonly frames = new Set<() => void>();
  private readonly anchors = new Set<() => void>();
  getSnapshot = () => ({ position: this.position, hour: Math.floor(this.position), fraction: 0, playing: this.playing, hoursPerSecond: this.hoursPerSecond,
    sequence: 0, seekSequence: 0, bounds: { firstHour: 0, lastHour: HOURS - 1 } });
  getHour = () => Math.floor(this.position);
  getMotion = () => this.playing ? this.hoursPerSecond : undefined;
  positionAt = () => this.position;
  subscribe = (listener: () => void) => { this.anchors.add(listener); return () => { this.anchors.delete(listener); }; };
  subscribeFrame = (listener: () => void) => { this.frames.add(listener); return () => { this.frames.delete(listener); }; };
  subscribeHour = () => () => {};
  step(hours: number) { this.position += hours; for (const listener of [...this.frames]) listener(); }
  anchor() { for (const listener of [...this.anchors]) listener(); }
}

/** A row every three hours: ordinal `n` at hour `3n`. */
const TOTAL = HOURS / ROW_HOURS;
function reply(product: string, params: Record<string, unknown>): unknown {
  if (product === 'log-counts') {
    const from = params.from as number, binHours = params.binHours as number, rows = binHours / ROW_HOURS;
    return { firstHour: from, binHours, missing: [], lines: [{ total: String(TOTAL), before: '0', bins: Array.from({ length: HOURS / binHours }, (_, bin) => [bin, rows]) }] };
  }
  if (product === 'log-page') {
    const cursor = Number(params.cursor), size = params.pageSize as number, count = Math.max(0, Math.min(size, TOTAL - cursor));
    return { columns: [], firstOrdinal: String(cursor), total: String(TOTAL),
      rows: Array.from({ length: count }, (_, at) => ({ key: String(cursor + at), hour: (cursor + at) * ROW_HOURS, ordinal: String(cursor + at), cells: [] })) };
  }
  throw new Error(`${product} is not served here`);
}

let clock = new Clock();
let frames = 0;
let commits = 0;
const requests: { cursor: number; from: number | null; head: number; playing: boolean; cancelled: boolean; frame: number }[] = [];
/** Pages fail to decode, though the counts are read. */
let failing = false;
/** What the log says beside its pages: its failure and how to try again. */
const said: { error: string | null; retry: (() => void) | null } = { error: null, retry: null };
const waiting: { due: number; land: () => void }[] = [];
let root: Root | null = null;
let host: HTMLDivElement | null = null;

const client = {
  available: true,
  product: (request: { sourceId: string; revision: string; product: string; params: Record<string, unknown>; asksFrom?: number }, signal?: AbortSignal) => {
    const answer = { sourceId: request.sourceId, revision: request.revision, product: request.product, data: reply(request.product, request.params) };
    if (request.product !== 'log-page') return Promise.resolve(answer);
    const asked = { cursor: Number(request.params.cursor), from: request.asksFrom ?? null, head: clock.getHour(), playing: clock.playing, cancelled: false, frame: frames };
    requests.push(asked);
    // A page lands LATENCY_FRAMES frames after it is asked, as run 13's pages of 64 did a second after.
    return new Promise((resolve, reject) => {
      signal?.addEventListener('abort', () => { asked.cancelled = true; reject(new DOMException('cancelled', 'AbortError')); });
      waiting.push({ due: frames + LATENCY_FRAMES, land: () => failing ? reject(new Error('The page block does not decode')) : resolve(answer) });
    });
  },
} as unknown as DashboardClient;

const toRow = (row: LogReplyRow): LogRow<string> => ({ key: row.key, hour: row.hour, item: `Row ${row.key}` });
const CUT: LogCut = { follow: true, offset: 0 };
/** Every row of the span, as a whole-campaign feed shows them. */
const CAMPAIGN: LogCut = { follow: false, hour: null };

function PagedLog({ at, active = true, density = 'comfortable', cut = CUT }: { at: Clock; active?: boolean; density?: 'comfortable' | 'compact'; cut?: LogCut }) {
  const spec = useMemo<LogSpec>(() => ({ source, branchId: '1', log: 'feed', firstHour: 0, lastHour: HOURS - 1 }), []);
  // As a widget does, it renders again when the clock starts or stops.
  useSyncExternalStore(at.subscribe, () => at.playing);
  const list = useLogPages(client, at as never, spec, cut, active, toRow, 64);
  said.error = list.error?.message ?? null; said.retry = list.retry;
  const reveal = useRevealClock(at as never, true);
  return <Profiler id="paged" onRender={() => { commits++; }}><LogList pages={list.pages} onWantedPages={list.onWantedPages} label="Paged log" density={density} months={false} animate
    playhead={cut.follow ? at.getHour() : null} reveal={reveal} columns="minmax(0, 1fr)" renderRow={row => <span>{row.item}</span>} describe={row => row.item} /></Profiler>;
}

/** Steps one frame: replies due land, then the frame renders. */
async function tick(hours: number) {
  frames++;
  for (const due of waiting.filter(item => item.due <= frames)) { waiting.splice(waiting.indexOf(due), 1); due.land(); }
  if (hours) clock.step(hours);
  await frame();
}

export const fixture = {
  async mount(cut: LogCut = CUT) {
    root?.unmount(); host?.remove();
    clock = new Clock(); requests.length = 0; waiting.length = 0; frames = 0; commits = 0; said.error = null; said.retry = null;
    host = document.body.appendChild(document.createElement('div'));
    host.style.cssText = 'width: 600px; height: 420px; display: flex; flex-direction: column;';
    root = createRoot(host);
    root.render(<PagedLog at={clock} cut={cut} />);
    for (let at = 0; at < LATENCY_FRAMES + 40; at++) await tick(0);
  },
  /** Plays `count` frames of `hours`: the commits of the list meanwhile and the time it took. */
  async play(count: number, hours: number) {
    clock.playing = true; clock.anchor();
    await settle(20);
    const before = commits, started = performance.now();
    for (let at = 0; at < count; at++) await tick(hours);
    const elapsed = performance.now() - started;
    await settle();
    return { commits: commits - before, elapsed };
  },
  pause() { clock.playing = false; clock.anchor(); },
  /** The log mounted again off screen (inactive) with its counts held from before, as the prepare lane holds them for a
   * widget below the fold, then played `count` frames: the pages asked meanwhile, and those asked once it is active. */
  async inactive(count: number, hours: number) {
    await fixture.mount();
    root!.render(null);
    await settle();
    requests.length = 0;
    root!.render(<PagedLog at={clock} active={false} />);
    await settle();
    clock.playing = true; clock.anchor();
    for (let at = 0; at < count; at++) await tick(hours);
    const offScreen = requests.length;
    clock.playing = false; clock.anchor();
    root!.render(<PagedLog at={clock} active />);
    for (let at = 0; at < 10; at++) await tick(0);
    return { offScreen, shown: requests.length - offScreen };
  },
  async leaving(hours: number) {
    root?.unmount(); host?.remove();
    clock = new Clock(); requests.length = 0; waiting.length = 0; frames = 0; commits = 0;
    host = document.body.appendChild(document.createElement('div'));
    host.style.cssText = 'width: 600px; height: 420px; display: flex; flex-direction: column;';
    root = createRoot(host);
    root.render(<PagedLog at={clock} />);
    for (let at = 0; at < LATENCY_FRAMES + 40; at++) await tick(0);
    clock.playing = true; clock.anchor();
    for (let at = 0; at < 30; at++) await tick(hours);
    const before = requests.length;
    root.render(null);
    for (let at = 0; at < LATENCY_FRAMES + 30; at++) await tick(hours);
    const askedAfter = requests.length - before;
    clock.playing = false; clock.anchor();
    await settle(4);
    const left = busyKinds();
    root.render(<PagedLog at={clock} />);
    for (let at = 0; at < LATENCY_FRAMES + 40 && busyCount() > 0; at++) await tick(0);
    const shown = busyCount();
    const committed = commits;
    root.render(<PagedLog at={clock} density="compact" />);
    for (let at = 0; at < 30 && commits === committed; at++) await tick(0);
    let settledIn = 0;
    while (settledIn < 30 && busyCount() > 0) { await tick(0); settledIn++; }
    const changed = { commits: commits - committed, frames: settledIn, busy: busyKinds() };
    root.unmount(); root = null;
    return { askedAfter, left, shown, changed };
  },
  /** Paused with every page failing for `count` frames: the pages asked, then those asked once retried. `campaign` shows
   * every row of the span, no cut waiting on a page. */
  async failed(count: number, campaign = false) {
    failing = true;
    try {
      await fixture.mount(campaign ? CAMPAIGN : CUT);
      for (let at = 0; at < count; at++) await tick(0);
      const asked = requests.map(request => request.cursor), error = said.error;
      requests.length = 0;
      said.retry?.();
      for (let at = 0; at < count; at++) await tick(0);
      return { asked, error, retried: requests.map(request => request.cursor) };
    } finally { failing = false; }
  },
  head: () => clock.getHour(),
  played: () => requests.filter(request => request.playing),
  all: () => [...requests],
  rows: () => host!.querySelectorAll('.log-row').length,
  loading: () => host!.querySelectorAll('.log-page').length,
};
(window as unknown as { pagesAhead: typeof fixture }).pagesAhead = fixture;
