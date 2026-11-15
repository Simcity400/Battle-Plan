/** The army size widget mounted in a real browser on a clock the test steps: its race fed followed answers as the
 * service writes them (or, not followed, window replies), its mix and a pinned breakdown fed window replies by unit
 * group; counting its renders, requests and paint passes. */
// First, so it is installed before react-dom loads: each commit's causes, named for a wait that never ends.
import { attributeCommits } from '../../shell/commitCount.ts';
import { Component, Profiler } from 'react';
import type { ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import type { Root } from 'react-dom/client';
import ArmySizeWidget from './ArmySizeWidget';
import { CELL } from '../../dashboard/follow/message.ts';
import { TILE_BINS } from '../../dashboard/tiles/tile.ts';
import { message } from '../../dashboard/follow/messages.fixture.ts';
import type { WidgetProps } from '../../dashboard/definitions';
// The app's tokens: without them the breakdown chart's min-height is invalid and its canvas grows every frame.
import '../../styles.css';
import { setPageClock } from '../../dashboard/structurePace.ts';
import { busyCount, busyKinds } from '../../shell/busy.ts';
import { MeasuredWidget } from '../../dashboard/widgetPerf.ts';

// Every animation frame's callbacks timed, summed by frame: what the widget's frame costs.
const passes: number[] = [];
let passFrame = -1;
const native = window.requestAnimationFrame.bind(window);
window.requestAnimationFrame = callback => native(now => {
  const started = performance.now();
  callback(now);
  const spent = performance.now() - started;
  if (now === passFrame) passes[passes.length - 1]! += spent;
  else { passes.push(spent); passFrame = now; }
});
const frame = () => new Promise<void>(resolve => native(() => resolve()));
// Short timeouts pending in the page (the time chart sends its view to the tiles on one): `quiet` waits for none. A
// longer one (a flag's retry, seconds away) is no work in flight.
const timers = new Set<number>();
const setNative = window.setTimeout.bind(window), clearNative = window.clearTimeout.bind(window);
window.setTimeout = ((handler: TimerHandler, delay?: number, ...rest: unknown[]) => {
  const id = setNative(() => { timers.delete(id); if (typeof handler === 'function') handler(...rest); }, delay) as unknown as number;
  if ((delay ?? 0) <= 1000) timers.add(id);
  return id;
}) as typeof setTimeout;
window.clearTimeout = (id => { timers.delete(id as unknown as number); clearNative(id); }) as typeof clearTimeout;
// A race chart draws by clearing its canvas first, on the page or handed to an offscreen canvas: its draws, counted.
let draws = 0;
for (const context of [CanvasRenderingContext2D, OffscreenCanvasRenderingContext2D]) {
  const clear = context.prototype.clearRect;
  context.prototype.clearRect = function (this: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D, ...args: Parameters<typeof clear>) {
    draws++; return clear.apply(this, args);
  };
}
async function settle(frames = 4) { for (let at = 0; at < frames; at++) await frame(); }
/** Frames until no update waits for a frame or the page's structure slot (one slot at most). */
async function slotted(limit = 60) { for (let at = 0; at < limit && busyKinds().updates; at++) await frame(); }

const HOURS = 7350, WINDOW = 490;
const windows = Array.from({ length: HOURS / WINDOW }, (_, at) => ({ firstHour: at * WINDOW, lastHour: at * WINDOW + WINDOW - 1 }));
const synthetic = { id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: { id: '1', firstHour: 0, lastHour: HOURS - 1, gaps: [] } };
let track = synthetic;
// A recording's own army replies, by their params, served instead of the synthetic ones (`serve`).
let real: Map<string, unknown> | null = null;
const canonical = (params: Record<string, unknown>) => JSON.stringify(Object.fromEntries(Object.entries(params).sort(([a], [b]) => a < b ? -1 : 1)));

/** A failed render shown as its error, so a test reads it rather than an empty page. */
class Failed extends Component<{ children: ReactNode }, { error: Error | null }> {
  state = { error: null as Error | null };
  static getDerivedStateFromError(error: Error) { return { error }; }
  render() { return this.state.error ? <pre className="fixture-error">{this.state.error.stack}</pre> : this.props.children; }
}

/** A clock the test moves: its frames and anchors are told when the test says. */
class Clock {
  position = 1000;
  playing = true;
  hoursPerSecond = 1440;
  private readonly frames = new Set<() => void>();
  private readonly anchors = new Set<() => void>();
  getSnapshot = () => ({ position: this.position, hour: Math.floor(this.position), fraction: this.position % 1, playing: this.playing,
    hoursPerSecond: this.hoursPerSecond, sequence: 0, seekSequence: 0, bounds: { firstHour: track.branch.firstHour, lastHour: track.branch.lastHour } });
  getHour = () => Math.floor(this.position);
  getMotion = () => this.playing ? this.hoursPerSecond : undefined;
  positionAt = () => this.position;
  subscribe = (listener: () => void) => { this.anchors.add(listener); return () => { this.anchors.delete(listener); }; };
  subscribeFrame = (listener: () => void) => { this.frames.add(listener); return () => { this.frames.delete(listener); }; };
  subscribeHour = () => () => {};
  step(hours: number) { this.position += hours; for (const listener of [...this.frames]) listener(); }
}

/** Each tag's divisions at `hour` in the fallback's window replies. */
const divisions = (tag: number, hour: number) => 10 + ((tag * 7 + Math.floor(hour / 24)) % 50);
/** Each tag's battalions of each unit group at `hour`, by day. */
const TYPES = [{ key: 'infantry', name: 'Infantry' }, { key: 'artillery', name: 'Artillery' }];
const battalions = (tag: number, hour: number) => [40 + ((tag * 7 + Math.floor(hour / 24)) % 50), 5 + ((tag * 3 + Math.floor(hour / 24)) % 7)];
const GROUPS = { branchId: '1', humans: [], factions: [], wars: [], warStarts: [], identityChanges: [], warnings: [], countryIdentities: [],
  capitulations: [], unobserved: { humans: [], factions: [], wars: [] } };

let clock = new Clock();
/** The causes of the widget's last commits, newest last. */
const causes: string[] = [];
attributeCommits((_zones, named) => {
  for (const [, byCause] of named) for (const cause of byCause.keys()) causes.push(cause);
  if (causes.length > 12) causes.splice(0, causes.length - 12);
});
let available = true;
let receive: ((bytes: ArrayBuffer | Uint8Array) => void) | null = null;
const requests: { product: string; params: Record<string, unknown> }[] = [];
let renders = 0;
let root: Root | null = null;
let host: HTMLDivElement | null = null;

function reply(product: string, params: Record<string, unknown>): unknown {
  if (product === 'follow') return available ? { available: true } : { available: false, reason: 'ArmyTotals is not in this playhead' };
  if (product === 'unfollow' || product === 'follow-clock') return {};
  if (product === 'country-groups') return GROUPS;
  // Looks that name no country: answered, so nothing is asked again on a timer while a test counts.
  if (product === 'country-styles') return { branchId: '1', hour: params.hour, styles: [], warnings: [] };
  if (product === 'country-facts') return { branchId: '1', known: [[track.branch.firstHour, track.branch.lastHour]], names: [], reasons: [], flags: [], countries: [], warnings: [] };
  if (product === 'series-tiles') {
    // Tiles that hold no value: the lines view's bins, not what it is tested on.
    const { binHours, index, tags } = params as { binHours: number; index: number; tags: string[] };
    const first = index * TILE_BINS * binHours, last = first + TILE_BINS * binHours - 1;
    return { branchId: '1', measure: 'divisions', decimals: 0, binHours, index, firstHour: first, immutable: true, filled: last, rawValues: true, hourBytes: 1,
      lines: tags.map(tag => ({ tag, missing: [{ firstHour: first, lastHour: last, reason: 'not recorded' }], parts: [] })), bins: btoa('\0'.repeat(TILE_BINS * tags.length)) };
  }
  if (product === 'army-size' && real) {
    const data = real.get(canonical(params));
    if (data !== undefined) return data;
    throw new Error(`No reply was recorded for ${canonical(params)}`);
  }
  if (product === 'template-roster') return { sections: real ? [] : ['Infantry division'], warnings: [] };
  if (product === 'army-size') {
    const base = { branchId: '1', available: true, measures: ['divisions', 'battalions'], types: [], warnings: [], windows, window: null, interval: null, series: [] };
    if (params.window == null) return base;
    const window = windows[params.window as number]!;
    const asked = (params.tags as string[] | undefined) ?? [];
    const tags = asked.length ? asked : Array.from({ length: (params.top as number | null) ?? 0 }, (_, at) => `T${String(at).padStart(3, '0')}`);
    const typed = params.measure === 'battalions';
    return { ...base, windows: [], types: typed ? TYPES : [], window: params.window, interval: window, series: tags.map(tag => {
      const at = Number(tag.slice(1));
      const runs: [number, number, number][] = [], mix: { firstHour: number; lastHour: number; values: number[] }[] = [];
      for (let day = window.firstHour; day <= window.lastHour; day += 24) {
        const last = Math.min(window.lastHour, day + 23), groups = battalions(at, day);
        runs.push([day, typed ? groups[0]! + groups[1]! : divisions(at, day), last]);
        if (typed && params.byType) mix.push({ firstHour: day, lastHour: last, values: groups });
      }
      // One unbroken stretch from the branch's start, as the service names a window's part of it.
      return { tag, membership: null, segments: [{ firstHour: 0, lastHour: window.lastHour }], runs, identities: [{ ...window, slot: at, countryId: at }], ...(typed && params.byType ? { mix } : {}) };
    }) };
  }
  throw new Error(`${product} is not served here`);
}

// Window replies held back while the test says, so what a crossing renders is told from what a reply renders.
let held: (() => void)[] | null = null;

/** A client of its own each mount: nothing asked by an earlier mount is held in the product cache. */
const makeClient = () => ({
  available: true,
  product: async (request: { sourceId: string; revision: string; product: string; params: Record<string, unknown> }) => {
    requests.push({ product: request.product, params: request.params });
    if (held && request.product === 'army-size' && request.params.window != null) await new Promise<void>(resolve => held!.push(resolve));
    return { sourceId: request.sourceId, revision: request.revision, product: request.product, data: reply(request.product, request.params) };
  },
  onFollowMessages: async (handler: (bytes: ArrayBuffer | Uint8Array) => void) => { receive = handler; return () => { if (receive === handler) receive = null; }; },
  setMapViews: async () => {}, onMapStates: async () => () => {}, releaseSource: async () => {}, openSource: async () => { throw new Error('none'); },
});
let client = makeClient();

/** The id the store followed the army by. */
function armyId(): number {
  const asked = [...requests].reverse().find(request => request.product === 'follow');
  if (!asked) throw new Error('The army is not followed');
  return asked.params.id as number;
}

export const fixture = {
  /** Mounts the top `count` (a race, followed when `followed`, or their leader's mix), the lines of the first `count`,
   * or a breakdown of `tag` from `origin` pinned and following, on a fresh clock at `at`, at the top of the page. */
  async mount(count: number, followed = true, view: 'race' | 'mix' | 'breakdown' | 'lines' = 'race', at = 1000, tag = 'T000', origin = 1000) {
    fixture.unmount();
    available = followed; clock = new Clock(); clock.position = at; client = makeClient(); requests.length = 0; renders = 0; held = null;
    // As the grid does: the page's clock owns when structure is taken while it plays.
    setPageClock(clock as never);
    host = document.body.appendChild(document.createElement('div'));
    host.style.cssText = 'width: 640px; height: 420px; display: flex; flex-direction: column;';
    root = createRoot(host);
    // A recording's own replies are served near the hours asked: a week's view asks only those windows.
    const near = real ? { playback: 'window', windowDays: 7 } : {};
    const widget = view === 'breakdown'
      ? { id: 'army', kind: 'army-size', trackIds: ['main'], countries: { kind: 'tags', tags: [tag] }, time: { mode: 'follow' },
        options: { view: 'breakdown', measure: 'battalions', drillOriginHour: origin, ...near } }
      : view === 'lines' ? { id: 'army', kind: 'army-size', trackIds: ['main'], countries: { kind: 'tags', tags: Array.from({ length: count }, (_, at) => `T${String(at).padStart(3, '0')}`) },
        time: { mode: 'follow' }, options: { view, ...near } }
      : { id: 'army', kind: 'army-size', trackIds: ['main'], countries: { kind: 'group', group: { type: 'top', count } },
        time: { mode: 'follow' }, options: { view, ...near } };
    const runtime = { tracks: [track], clock, client, addWidgetBelow() {}, campaigns: [], recordings: [] };
    root.render(<Failed><MeasuredWidget value="army"><Profiler id="army" onRender={() => { renders++; }}>
      <ArmySizeWidget {...{ widget, runtime, active: true, onDiagnostics() {} } as unknown as WidgetProps} />
    </Profiler></MeasuredWidget></Failed>);
    await settle(8);
    await slotted();
  },
  unmount() {
    root?.unmount(); root = null;
    host?.remove(); host = null;
  },
  /** One followed answer of the top: (row key, divisions), greatest first as the service orders them. */
  async deliver(hour: number, rows: [number, number][], frames = 2) {
    const ordered = [...rows].sort((a, b) => b[1] - a[1] || a[0] - b[0]);
    receive?.(message(hour, [{ id: armyId(), answer: { keys: ordered.map(([key]) => BigInt(key)), columns: 1,
      values: ordered.map(([, value]) => BigInt(value)), states: ordered.map(() => CELL.value), names: ordered.map(([key]) => `T${String(key).padStart(3, '0')}`) } }]));
    await settle(frames);
  },
  /** Moves the clock `hours` a frame, `frames` times. */
  async play(hours: number, frames: number) { for (let at = 0; at < frames; at++) { clock.step(hours); await frame(); } await settle(); },
  renders: () => renders,
  /** Data requests: everything but follow's own messages. */
  asked: () => requests.filter(request => !['follow', 'unfollow', 'follow-clock'].includes(request.product)).map(request => `${request.product} ${JSON.stringify(request.params.window ?? null)}`),
  sent: () => requests.filter(request => ['follow', 'unfollow'].includes(request.product)).map(request => `${request.product} ${request.params.id}`),
  /** The race as drawn: its spoken ranking, value labels in order. */
  drawn: () => (document.querySelector('.race-chart p[id$="-ranking"]')?.textContent ?? '').split('; ').map(row => row.split(' ').at(-1) ?? ''),
  passes: () => { const out = [...passes]; passes.length = 0; return out; },
  draws: () => { const out = draws; draws = 0; return out; },
  /** Three answers inside one frame, then `frames` frames: the race's draws meanwhile. */
  async burst(hour: number, frames: number) {
    draws = 0;
    for (let at = 0; at < 3; at++) {
      const rows: [number, number][] = [[1, 10 + at], [2, 20 + at], [3, 30 + at]];
      receive?.(message(hour + at, [{ id: armyId(), answer: { keys: rows.map(([key]) => BigInt(key)).reverse(), columns: 1,
        values: rows.map(([, value]) => BigInt(value)).reverse(), states: rows.map(() => CELL.value), names: rows.map(([key]) => `T${String(key).padStart(3, '0')}`).reverse() } }]));
    }
    await settle(frames);
    return draws;
  },
  /** Serves a recording's own army replies (each `[params, data]`) over `branch`; the manifest's reply names the windows. */
  serve(branch: { firstHour: number; lastHour: number }, replies: [Record<string, unknown>, unknown][]) {
    real = new Map(replies.map(([params, data]) => [canonical(params), data]));
    track = { ...synthetic, branch: { id: '1', firstHour: branch.firstHour, lastHour: branch.lastHour, gaps: [] } };
  },
  /** Frames until no short timeout is pending, nothing is in flight (a reply, or what waits for the page's structure
   * slot) and three pass without a render: whatever was started has landed. */
  async quiet(limit = 600) {
    let still = 0, seen = renders;
    for (let at = 0; at < limit; at++) {
      await frame();
      if (timers.size || busyCount() || renders !== seen) { still = 0; seen = renders; } else if (++still === 3) return;
    }
    throw new Error(`Not quiet after ${limit} frames: ${timers.size} timeouts pending, ${busyCount()} in flight; last renders: ${causes.join(' | ')}`);
  },
  /** Frames until three pass in which nothing runs: the race has come to rest. */
  async still(limit = 600) {
    let quietFor = 0, seen = passes.length;
    for (let at = 0; at < limit; at++) {
      await frame();
      if (passes.length !== seen) { quietFor = 0; seen = passes.length; } else if (++quietFor === 3) return;
    }
    throw new Error(`Still running after ${limit} frames`);
  },
  /** Holds window replies back until `release`. */
  hold() { held ??= []; },
  async release() { const waiting = held ?? []; held = null; for (const resolve of waiting) resolve(); await settle(); },
  /** The texts of every element `selector` finds, in document order. */
  texts: (selector: string) => [...document.querySelectorAll(selector)].map(node => node.textContent ?? ''),
  async offScreen() { host!.style.marginTop = '5000px'; await settle(); },
  async onScreen() { host!.style.marginTop = '0'; await settle(); },
  paused(value: boolean) { clock.playing = !value; },
};
(window as unknown as { armyRace: typeof fixture }).armyRace = fixture;
