/** World tension following the head in a real browser, as the 13-year recording pushes it: thirty contributing countries,
 * the panel pushed four times a second at a new hour with the contributions in the service's order (greatest first, so
 * the countries below the five layers trade places at every push), the tension, the sum and every contribution live
 * cells a frame at a time at 15 hours a frame; its commits and why, what it asked, and the hours it reported shown. */
// Before react-dom: a production-shaped hook it reports each commit to, for why a commit happened.
import { attributeCommits } from '../../shell/commitCount.ts';
import { Profiler } from 'react';
import { createRoot } from 'react-dom/client';
import type { Root } from 'react-dom/client';
import WorldTensionWidget from './WorldTensionWidget';
import { CELLS } from '../../dashboard/follow/liveCells';
import { message } from '../../dashboard/follow/messages.fixture.ts';
import { MeasuredWidget } from '../../dashboard/widgetPerf.ts';
import { measureWith } from '../../shell/perf.ts';
import type { PerfRecorder } from '../../shell/perf.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import '../../styles.css';
import { setPageClock } from '../../dashboard/structurePace.ts';

const native = window.requestAnimationFrame.bind(window);
const frame = () => new Promise<void>(resolve => native(() => resolve()));
async function settle(frames = 8) { for (let at = 0; at < frames; at++) await frame(); }

const shown: (number | null)[] = [];
measureWith(new Proxy({ shown: (_widget: string, hour: number | null) => { shown.push(hour); } }, {
  get: (target, name) => (target as Record<string | symbol, unknown>)[name] ?? (() => {}),
}) as unknown as PerfRecorder);

const HOURS = 120_000;
const TILE_BINS = 256;
const track = { id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: { id: '1', firstHour: 0, lastHour: HOURS - 1, gaps: [] } };
const TAGS = Array.from({ length: 30 }, (_, at) => `C${String(at).padStart(2, '0')}`);
const SLOTS = TAGS.map((_, at) => at + 1);
const WORLD_GROUP = CELLS + 11, PARTS_GROUP = CELLS + 12;

class Clock {
  position = 30_000;
  playing = false;
  private readonly frames = new Set<() => void>();
  private readonly anchors = new Set<() => void>();
  getSnapshot = () => ({ position: this.position, hour: Math.floor(this.position), fraction: 0, playing: this.playing, hoursPerSecond: 1440, sequence: 0, seekSequence: 0,
    bounds: { firstHour: 0, lastHour: HOURS - 1 } });
  getHour = () => Math.floor(this.position);
  getMotion = () => this.playing ? 1440 : undefined;
  positionAt = () => this.position;
  subscribe = (listener: () => void) => { this.anchors.add(listener); return () => { this.anchors.delete(listener); }; };
  subscribeFrame = (listener: () => void) => { this.frames.add(listener); return () => { this.frames.delete(listener); }; };
  subscribeHour = () => () => {};
  step(hours: number) { this.position += hours; for (const listener of [...this.frames]) listener(); }
  anchor() { for (const listener of [...this.anchors]) listener(); }
}

let clock = new Clock();
let receive: ((bytes: ArrayBuffer | Uint8Array) => void) | null = null;
let panel: number | null = null;
const requests: string[] = [];
let commits = 0;
const why = new Map<string, number>();
attributeCommits((_zones, causes) => { for (const named of causes.values()) for (const [cause, count] of named) why.set(cause, (why.get(cause) ?? 0) + count); });
let root: Root | null = null;
let host: HTMLDivElement | null = null;
/** Whether the five greatest trade places too: off, only the countries below them do. */
let topMoves = false;

/** Country `at`'s contribution at `hour`, in contribution units: the first five always greatest, in their order unless
 * `topMoves`; the rest trading places every hour. */
function contribution(at: number, hour: number): number {
  if (at < 5) return (topMoves && hour % 2 ? 10 + at : 15 - at) * 1_000_000 + (hour % 50) * 100;
  return 100_000 + ((at * 37 + hour) % 23) * 1_000;
}
const order = (hour: number) => TAGS.map((_, at) => at).sort((a, b) => contribution(b, hour) - contribution(a, hour) || a - b);

function reply(product: string, params: Record<string, unknown>): unknown {
  if (product === 'follow') {
    const query = params.query as { panel?: string };
    if (query.panel === 'world-tension') panel = params.id as number;
    return { available: true };
  }
  if (product === 'unfollow' || product === 'follow-clock') return {};
  if (product === 'world-tension') return panelAt(params.hour as number);
  if (product === 'threat-events') return { events: [], missing: [], warnings: [] };
  if (product === 'country-styles') return { branchId: '1', hour: params.hour, styles: [], warnings: [] };
  if (product === 'country-facts') return { branchId: '1', known: [[0, HOURS - 1]], names: [], reasons: [], flags: [], countries: [], warnings: [] };
  if (product === 'country-names') return { branchId: '1', names: [], warnings: [] };
  if (product === 'series-tiles') {
    const { binHours, index, tags } = params as { binHours: number; index: number; tags: string[] };
    const first = index * TILE_BINS * binHours, last = first + TILE_BINS * binHours - 1;
    return { branchId: '1', measure: 'contribution', decimals: 5, binHours, index, firstHour: first, immutable: last < HOURS - 1, filled: Math.min(last, HOURS - 1),
      rawValues: true, hourBytes: 1, lines: tags.map(tag => ({ tag, missing: [{ firstHour: first, lastHour: last, reason: 'not recorded' }], parts: [] })),
      bins: btoa('\0'.repeat(TILE_BINS * tags.length)) };
  }
  throw new Error(`${product} is not served here`);
}

const client = {
  available: true,
  product: async (request: { sourceId: string; revision: string; product: string; params: Record<string, unknown> }) => {
    requests.push(request.product);
    return { sourceId: request.sourceId, revision: request.revision, product: request.product, data: reply(request.product, request.params) };
  },
  onFollowMessages: async (handler: (bytes: ArrayBuffer | Uint8Array) => void) => { receive = handler; return () => { if (receive === handler) receive = null; }; },
  setMapViews: async () => {}, onMapStates: async () => () => {}, releaseSource: async () => {}, openSource: async () => { throw new Error('none'); },
};

/** The panel at `hour` as the service builds it: every contribution greatest first, the world's and each contribution's
 * cells followed in two groups. */
function panelAt(hour: number) {
  const ranked = order(hour);
  const kinds = [{ id: 'tension', op: 'value', scale: 1 }, { id: 'sum', op: 'value', scale: 1 }, { id: 'value', op: 'value', scale: 1 }];
  const rows = ['world', ...ranked.map(at => String(SLOTS[at]))];
  const cells = [0, 0, 1, 0, 0, 0, 0, 1, 1, 0, 0, 1, ...ranked.flatMap((at, row) => [row + 1, 2, 1, 1, at, 0])];
  const data = { tensionScale: 1_000, contributionScale: 100_000, ceilingRaw: '100000', unnamed: false, warnings: [], missing: null,
    contributions: ranked.map(at => ({ slot: SLOTS[at], tag: TAGS[at], remembered: false })), sources: [],
    live: { groups: [{ table: 90, columns: [1, 2], keys: ['0'], follow: WORLD_GROUP }, { table: 91, columns: [1], keys: SLOTS.map(String), follow: PARTS_GROUP }],
      rows, kinds, cells } };
  return data;
}

function push(hour: number) {
  receive?.(new TextEncoder().encode(`PNL1${JSON.stringify({ panels: [{ id: panel, hour, data: panelAt(hour), refused: null }] })}`));
}

/** The cells at `hour`: the tension, the sum and every contribution. */
function values(hour: number) {
  const sum = TAGS.reduce((total, _, at) => total + contribution(at, hour), 0);
  receive?.(message(hour, [
    { id: WORLD_GROUP, answer: { keys: [0n], columns: 2, values: [BigInt(40_000 + hour % 100), BigInt(sum)] } },
    { id: PARTS_GROUP, answer: { keys: SLOTS.map(BigInt), columns: 1, values: TAGS.map((_, at) => BigInt(contribution(at, hour))) } },
  ]));
}

export const fixture = {
  async mount(moving = false) {
    root?.unmount(); host?.remove();
    clock = new Clock(); requests.length = 0; shown.length = 0; commits = 0; panel = null; topMoves = moving;
    // As the grid does: the page's clock owns when structure is taken while it plays.
    setPageClock(clock as never);
    host = document.body.appendChild(document.createElement('div'));
    host.style.cssText = 'width: 640px; height: 360px; display: flex; flex-direction: column;';
    root = createRoot(host);
    const runtime = { tracks: [track], clock, client, addWidgetBelow() {}, campaigns: [], recordings: [] };
    const widget = { id: 'tension', kind: 'world-tension', trackIds: ['main'], countries: { kind: 'all' }, time: { mode: 'follow' }, options: {} };
    root.render(<MeasuredWidget value="tension"><Profiler id="tension" onRender={() => { commits++; }}>
      <WorldTensionWidget {...({ widget, runtime, active: true, onDiagnostics() {}, setOption() {} } as unknown as WidgetProps)} />
    </Profiler></MeasuredWidget>);
    for (let at = 0; at < 120 && panel === null; at++) await frame();
    if (panel === null) throw new Error(`the panel was not followed; asked ${requests.join(', ')}`);
    push(clock.getHour());
    await settle(12);
    values(clock.getHour());
    await settle(12);
    return panel;
  },
  /** Plays `frames` frames of `hours` each: the cells every frame, the panel every `every` frames at the head's hour. */
  async play(frames: number, hours: number, every: number) {
    clock.playing = true; clock.anchor();
    await settle(30);
    const before = { requests: requests.length, commits };
    why.clear();
    for (let at = 0; at < frames; at++) {
      clock.step(hours);
      if (at % every === 0) push(clock.getHour());
      values(clock.getHour());
      await frame();
    }
    await settle();
    return { head: clock.getHour(), pushes: Math.ceil(frames / every), commits: commits - before.commits, why: Object.fromEntries(why),
      tiles: requests.slice(before.requests).filter(product => product === 'series-tiles').length,
      asked: requests.slice(before.requests).filter(product => !['follow', 'unfollow', 'follow-clock', 'series-tiles'].includes(product)) };
  },
  pause() { clock.playing = false; clock.anchor(); },
  /** The list beside the chart: each layer's name and painted value, then Other's, then the total. */
  list: () => [...(host?.querySelectorAll<HTMLElement>('.world-tension-list li') ?? [])].map(item => item.textContent?.trim() ?? ''),
  total: () => host?.querySelector('.world-tension-total b')?.textContent ?? '',
  lastShown: () => shown.at(-1) ?? null,
};
(window as unknown as { worldTension: typeof fixture }).worldTension = fixture;
