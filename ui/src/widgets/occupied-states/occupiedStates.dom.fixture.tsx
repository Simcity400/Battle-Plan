/** Occupied states following the head in a real browser: the panel's structure pushed once, its resistance and
 * compliance cells pushed a frame at a time as the service writes them at 15 hours a frame; what it painted, how many
 * commits it took, what it asked and the hours it reported shown. */
// Before react-dom: a production-shaped hook it reports each commit to, for why a commit happened.
import { attributeCommits } from '../../shell/commitCount.ts';
import { Profiler } from 'react';
import { createRoot } from 'react-dom/client';
import type { Root } from 'react-dom/client';
import OccupiedStatesWidget from './OccupiedStatesWidget';
import { CELLS } from '../../dashboard/follow/liveCells';
import { message } from '../../dashboard/follow/messages.fixture.ts';
import { MeasuredWidget } from '../../dashboard/widgetPerf.ts';
import { measureWith } from '../../shell/perf.ts';
import type { PerfRecorder } from '../../shell/perf.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import type { OccupiedState } from './occupiedStatesData';
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
const track = { id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: { id: '1', firstHour: 0, lastHour: HOURS - 1, gaps: [] } };
const STATES = ['10', '11'];
/** As a real recording holds them: forty occupied states, their garrisons read anew at every push. */
const MANY = Array.from({ length: 40 }, (_, at) => String(100 + at));

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

const state = (id: number): OccupiedState => ({ state: id, name: `State ${id}`, nameMissing: null, owner: 'POL', occupier: 'GER', occupied: 'POL',
  garrisonRequiredSign: 1, garrisonAlert: false, assignedKnown: true, garrisonMissing: null, law: 'harsh', lawMissing: null });

let clock = new Clock();
let receive: ((bytes: ArrayBuffer | Uint8Array) => void) | null = null;
let panel: number | null = null;
const follow = CELLS + 7;
/** The garrisons' cells (StateGarrisons' required manpower and amount) are a second followed group. */
const garrisons = CELLS + 8;
const GARRISON_KINDS = [{ id: 'requiredManpower', op: 'value', scale: 1 }, { id: 'garrison', op: 'value', scale: 1 }];
/** The garrison amount of the state at `at` at `hour`: it moves every hour, as the requirement follows resistance. */
const garrisonAmount = (at: number, hour: number) => 250_000 + at * 100_000 + (hour % 50) * 1_000;
/** The garrisons' strengths (GarrisonStrengths, which the recorder derives) are a third followed group. */
const strengths = CELLS + 9;
/** The content's red threshold, fixed5. */
export const RED = 30_000;
/** The strength of the garrison at `at` at `hour`: the first's crosses the red threshold every fifty hours, the
 * second's never does. */
export const strengthAt = (at: number, hour: number) => at === 0 ? 20_000 + (hour % 50) * 1_000 : 60_000 + (hour % 50) * 500;
const requests: string[] = [];
let commits = 0;
/** Why each commit of the widget rendered: the component that began the work and what changed in it, with a count. */
const why = new Map<string, number>();
attributeCommits((_zones, causes) => { for (const named of causes.values()) for (const [cause, count] of named) why.set(cause, (why.get(cause) ?? 0) + count); });
let root: Root | null = null;
let host: HTMLDivElement | null = null;

function reply(product: string, params: Record<string, unknown>): unknown {
  if (product === 'follow') {
    const query = params.query as { panel?: string };
    if (query.panel === 'occupied-states') panel = params.id as number;
    return { available: true };
  }
  if (product === 'unfollow' || product === 'follow-clock') return {};
  if (product === 'country-styles') return { branchId: '1', hour: params.hour, styles: [], warnings: [] };
  if (product === 'country-facts') return { branchId: '1', known: [[0, HOURS - 1]], names: [], reasons: [], flags: [], countries: [], warnings: [] };
  if (product === 'country-names') return { branchId: '1', names: [], warnings: [] };
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

/** The panel's one structure push: two occupied states, their four live columns followed under `follow`. */
function structure(hour: number) {
  const columns = ['resistance', 'compliance', 'resistanceSpeed', 'complianceSpeed'];
  const garrisonKeys = STATES.map(id => String(1n << 32n | BigInt(id)));
  const data = { states: [state(10), state(11)], complete: true, missing: null, laws: [{ token: 'harsh', name: 'Harsh', icon: null, missing: null }], strengthMissing: '',
    garrisonRedRaw: String(RED), warnings: [],
    live: { groups: [{ table: 70, columns: [1, 2, 3, 4], keys: STATES, follow }, { table: 67, columns: [1, 3], keys: garrisonKeys, follow: garrisons },
      { table: 514, columns: [0], keys: garrisonKeys, follow: strengths }],
      rows: STATES, kinds: [...columns.map(id => ({ id, op: 'value', scale: 1 })), ...GARRISON_KINDS, { id: 'garrisonStrength', op: 'value', scale: 1 }],
      cells: STATES.flatMap((_, row) => [...columns.flatMap((_, column) => [row, column, 1, 0, row, column]), row, 4, 1, 1, row, 0, row, 5, 1, 1, row, 1, row, 6, 1, 2, row, 0]) } };
  receive?.(new TextEncoder().encode(`PNL1${JSON.stringify({ panels: [{ id: panel, hour, data, refused: null }] })}`));
}

/** The panel as the service pushes it four times a second on a real recording: forty states at `hour`, every garrison
 * moved since the last push, the same four live columns. */
function structureMany(hour: number) {
  const columns = ['resistance', 'compliance', 'resistanceSpeed', 'complianceSpeed'];
  const data = { states: MANY.map(id => ({ ...state(Number(id)), garrisonAlert: (hour + Number(id)) % 97 === 0 })), complete: true, missing: null,
    laws: [{ token: 'harsh', name: 'Harsh', icon: null, missing: null }], strengthMissing: '', garrisonRedRaw: String(RED), warnings: [],
    live: { groups: [{ table: 70, columns: [1, 2, 3, 4], keys: MANY, follow }], rows: MANY, kinds: columns.map(id => ({ id, op: 'value', scale: 1 })),
      cells: MANY.flatMap((_, row) => columns.flatMap((_, column) => [row, column, 1, 0, row, column])) } };
  receive?.(new TextEncoder().encode(`PNL1${JSON.stringify({ panels: [{ id: panel, hour, data, refused: null }] })}`));
}

/** The forty states' cells at `hour`: their resistance trades places every hour, so the table's order by it moves. */
function cellsMany(hour: number) {
  receive?.(message(hour, [{ id: follow, answer: { keys: MANY.map(BigInt), columns: 4,
    values: MANY.flatMap((_, at) => [BigInt(2_000_000 + ((at * 37 + hour) % 40) * 100_000), BigInt(300_000 + (hour % 40) * 1_000), -7n, 3n]) } }]));
}

/** The cells at `hour`: each state's resistance, compliance and garrison amount move every hour. */
function cells(hour: number) {
  receive?.(message(hour, [{ id: follow, answer: { keys: STATES.map(BigInt), columns: 4,
    values: STATES.flatMap((_, at) => [BigInt(2_000_000 + at * 1_000_000 + (hour % 50) * 10_000), BigInt(300_000 + (hour % 40) * 1_000), -7n, 3n]) } },
  { id: garrisons, answer: { keys: STATES.map(id => 1n << 32n | BigInt(id)), columns: 2, values: STATES.flatMap((_, at) => [100n, BigInt(garrisonAmount(at, hour))]) } },
  { id: strengths, answer: { keys: STATES.map(id => 1n << 32n | BigInt(id)), columns: 1, values: STATES.map((_, at) => BigInt(strengthAt(at, hour))) } }]));
}

export const fixture = {
  async mount(many = false) {
    root?.unmount(); host?.remove();
    clock = new Clock(); requests.length = 0; shown.length = 0; commits = 0; panel = null;
    // As the grid does: the page's clock owns when structure is taken while it plays.
    setPageClock(clock as never);
    host = document.body.appendChild(document.createElement('div'));
    host.style.cssText = 'width: 1400px; height: 400px; display: flex; flex-direction: column;';
    root = createRoot(host);
    const runtime = { tracks: [track], clock, client, addWidgetBelow() {}, campaigns: [], recordings: [] };
    const widget = { id: 'occupied', kind: 'occupied-states', trackIds: ['main'], countries: { kind: 'all' }, time: { mode: 'follow' }, options: {} };
    root.render(<MeasuredWidget value="occupied"><Profiler id="occupied" onRender={() => { commits++; }}>
      <OccupiedStatesWidget {...({ widget, runtime, active: true, onDiagnostics() {}, setOption() {} } as unknown as WidgetProps)} />
    </Profiler></MeasuredWidget>);
    await settle(12);
    (many ? structureMany : structure)(clock.getHour());
    await settle(12);
    (many ? cellsMany : cells)(clock.getHour());
    await settle(12);
    return panel;
  },
  async play(frames: number, hours: number) {
    clock.playing = true; clock.anchor();
    await settle(30);
    const before = { requests: requests.length, commits };
    for (let at = 0; at < frames; at++) {
      clock.step(hours);
      cells(clock.getHour());
      await frame();
    }
    await settle();
    return { head: clock.getHour(), asked: requests.slice(before.requests).filter(product => !['follow', 'unfollow', 'follow-clock'].includes(product)), commits: commits - before.commits };
  },
  /** Plays as the real recording does: the forty states' cells each frame, and the panel pushed anew every `every` frames
   * (four times a second at 60 frames a second, every 15), each push at the head's hour with its garrisons moved. */
  async playPushed(frames: number, hours: number, every: number) {
    clock.playing = true; clock.anchor();
    await settle(30);
    const before = { requests: requests.length, commits };
    why.clear();
    const started = performance.now();
    for (let at = 0; at < frames; at++) {
      clock.step(hours);
      if (at % every === 0) structureMany(clock.getHour());
      cellsMany(clock.getHour());
      await frame();
    }
    await settle();
    return { head: clock.getHour(), elapsed: performance.now() - started, pushes: Math.ceil(frames / every),
      asked: requests.slice(before.requests).filter(product => !['follow', 'unfollow', 'follow-clock'].includes(product)), commits: commits - before.commits,
      why: Object.fromEntries(why) };
  },
  pause() { clock.playing = false; clock.anchor(); },
  /** Paused, a seek to `hour` where no state is occupied: the panel pushed there holding none. */
  async seekEmpty(hour: number) {
    const data = { states: [], complete: true, missing: null, laws: [], strengthMissing: '', garrisonRedRaw: String(RED), warnings: [], live: { groups: [], rows: [], kinds: [], cells: [] } };
    receive?.(new TextEncoder().encode(`PNL1${JSON.stringify({ panels: [{ id: panel, hour, data, refused: null }] })}`));
    await settle(12);
    return { shown: shown.at(-1) ?? null, empty: host?.textContent?.includes('No occupied states match') ?? false };
  },
  /** Each mounted row's painted cells, by row. */
  painted: () => [...(host?.querySelectorAll<HTMLElement>('tbody tr:not([aria-hidden="true"])') ?? [])]
    .map(row => [...row.querySelectorAll<HTMLElement>('.table-live')].map(cell => cell.textContent)),
  /** Each mounted row's garrison bar (its scale) and whether its garrison shows red, by row. */
  bars: () => [...(host?.querySelectorAll<HTMLElement>('tbody tr:not([aria-hidden="true"])') ?? [])].map(row => {
    const cell = row.querySelector<HTMLElement>('[data-table-live-column="garrison"]');
    return [cell?.closest('.table-cell')?.querySelector<HTMLElement>('.table-cellbar')?.style.transform ?? null, cell?.classList.contains('occupation-garrison-alert') ?? false];
  }),
  lastShown: () => shown.at(-1) ?? null,
};
(window as unknown as { occupiedStates: typeof fixture }).occupiedStates = fixture;
