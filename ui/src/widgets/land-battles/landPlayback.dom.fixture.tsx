import { attributeCommits } from '../../shell/commitCount.ts';
import { Profiler } from 'react';
import { createRoot } from 'react-dom/client';
import type { Root } from 'react-dom/client';
import LandBattlesWidget from './LandBattlesWidget';
import { MeasuredWidget } from '../../dashboard/widgetPerf.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import '../../styles.css';

const native = window.requestAnimationFrame.bind(window);
const frame = () => new Promise<void>(resolve => native(() => resolve()));
async function settle(frames = 8) { for (let at = 0; at < frames; at++) await frame(); }

const WINDOW_HOURS = 720;
const HOURS = 140 * WINDOW_HOURS;
const HEAD = 30_000;
const branch = { id: '1', firstHour: 0, lastHour: HOURS - 1, gaps: [] };
const track = { id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [], branches: [branch] } }, branch };

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

const WINDOWS = Array.from({ length: HOURS / WINDOW_HOURS }, (_, index) => ({ firstHour: index * WINDOW_HOURS, lastHour: (index + 1) * WINDOW_HOURS - 1 }));
const TAGS = Array.from({ length: 40 }, (_, at) => `C${String(at).padStart(2, '0')}`);
const noise = (seed: number) => { const x = Math.sin(seed * 12.9898) * 43758.5453; return x - Math.floor(x); };
const PER_WINDOW = 200;
function battle(window: number, at: number) {
  const seed = window * PER_WINDOW + at;
  const start = WINDOWS[window]!.firstHour + Math.floor(at * WINDOW_HOURS / PER_WINDOW);
  const length = 200 + Math.floor(noise(seed) * 500);
  const ends = start + length;
  const last = Math.min(ends, WINDOWS[window]!.lastHour + 1440);
  const sides = (run: number) => [0, 1].map(side => {
    const count = 1 + Math.floor(noise(seed * 7 + side * 3 + Math.floor(run / 4)) * (side ? 3 : 5));
    return Array.from({ length: count }, (_, index) => TAGS[(seed * 3 + side * 11 + index * 5) % TAGS.length]!);
  });
  const runs = [];
  for (let first = start, run = 0; first <= last; first += 48, run++) {
    runs.push({ firstHour: first, lastHour: Math.min(first + 47, last), counts: [3 + run % 5, 2 + run % 3], rosterUnknown: false, countries: sides(run),
      countriesUnknown: [false, false], duration: first - start });
  }
  const losses = [];
  for (let hour = start + 24; hour <= last; hour += 24) {
    const day = (hour - start) / 24;
    losses.push({ hour, terminal: hour + 24 > last && last === ends, sides: [{ manpower: day * 310, equipment: String(day * 100_000) }, { manpower: day * 270, equipment: String(day * 90_000) }] });
  }
  const done = last === ends;
  return { id: `B${seed}`, provinceId: seed + 1, start: { hour: start, bound: 'exact' }, end: done ? { hour: ends, bound: 'exact' } : null, endUnknownHour: null, counterBase: 0,
    priorPeak: null, priorSidePeaks: null, losses,
    result: done ? { hour: ends, duration: length, winner: { side: seed % 2 }, countries: sides(runs.length - 1), countriesUnknown: [false, false] } : null, runs };
}
const landWindow = (index: number) => ({ window: index, interval: WINDOWS[index], observed: [[WINDOWS[index]!.firstHour, WINDOWS[index]!.lastHour]], labels: [], reasons: [],
  warnings: [], contentWarnings: [], rows: Array.from({ length: PER_WINDOW }, (_, at) => battle(index, at)) });

function reply(product: string, params: Record<string, unknown>): unknown {
  if (product === 'land-battles-manifest') return { windows: WINDOWS, available: true, unavailable: null, contentWarnings: [] };
  if (product === 'land-battles') return landWindow(params.window as number);
  if (product === 'land-battle-terrain') return { provinces: (params.provinces as number[]).map(provinceId => ({ provinceId, name: 'Plains', missing: null })), warnings: [] };
  if (product === 'place-names') return { places: (params.places as { provinceId: number; hour: number }[]).map(place => ({ ...place, name: `Province ${place.provinceId}`, missing: null })),
    states: [], warnings: [] };
  if (product === 'country-styles') return { branchId: '1', hour: params.hour, styles: [], warnings: [] };
  if (product === 'country-facts') return { branchId: '1', known: [[0, HOURS - 1]], names: [], reasons: [], flags: [], countries: [], warnings: [] };
  if (product === 'combat-log') return { available: false, reason: 'not here' };
  if (product === 'follow') return { available: false, reason: 'not followed here' };
  if (product === 'unfollow' || product === 'follow-clock') return {};
  throw new Error(`${product} is not served here`);
}

let clock = new Clock();
let root: Root | null = null;
let host: HTMLDivElement | null = null;
const errors: string[] = [];

const client = {
  available: true,
  product: async (request: { sourceId: string; revision: string; product: string; params: Record<string, unknown> }) => {
    try {
      return { sourceId: request.sourceId, revision: request.revision, product: request.product, data: reply(request.product, request.params) };
    } catch (error) { errors.push(String(error)); throw error; }
  },
  setMapViews: async () => {}, onMapStates: async () => () => {}, releaseSource: async () => {}, openSource: async () => { throw new Error('none'); },
};

let commits: { ms: number; kept: number; redrawn: number }[] = [];
let observer: MutationObserver | null = null;
let drawnRows = new Set<Element>();
let bindings: string[] = [];
let redrawnSince = 0;
function takeRecords() {
  for (const record of observer?.takeRecords() ?? []) for (const node of record.addedNodes) {
    if (!(node instanceof Element)) continue;
    const row = node.closest<HTMLElement>('.log-row');
    if (!row || row === node) continue;
    if (row.dataset.pool != null && node.parentElement === row) {
      if (node.classList.contains('lb-cells')) bindings.push(row.dataset.logKey ?? '');
      continue;
    }
    if (!drawnRows.has(row) || (node.classList.contains('log-state') && node.parentElement === row)) continue;
    redrawnSince += 1 + node.querySelectorAll('*').length;
  }
}
function takeCommit(ms: number) {
  if (!host) return;
  takeRecords();
  const redrawn = redrawnSince;
  redrawnSince = 0;
  const rows = new Set(host.querySelectorAll('.log-row'));
  commits.push({ ms, kept: [...rows].filter(row => drawnRows.has(row)).length, redrawn });
  drawnRows = rows;
}
const why = new Map<string, number>();
attributeCommits((_zones, causes) => {
  for (const named of causes.values()) for (const [cause, count] of named) why.set(cause, (why.get(cause) ?? 0) + count);
});

function Scoped({ children }: { children: React.ReactNode }) {
  return <MeasuredWidget value="land">
    <Profiler id="land" onRender={(_id, _phase, actual) => takeCommit(actual)}>{children}</Profiler>
  </MeasuredWidget>;
}

function timedFrame(step: () => void): Promise<number> {
  return new Promise(resolve => native(() => {
    const begun = performance.now();
    step();
    const channel = new MessageChannel();
    channel.port1.onmessage = () => resolve(performance.now() - begun);
    channel.port2.postMessage(0);
  }));
}

export const fixture = {
  async mount(options: Record<string, unknown> = {}) {
    root?.unmount(); host?.remove();
    clock = new Clock(); errors.length = 0;
    host = document.body.appendChild(document.createElement('div'));
    host.style.cssText = 'width: 1200px; display: flex; flex-direction: column;';
    observer?.disconnect();
    observer = new MutationObserver(() => {});
    observer.observe(host, { childList: true, subtree: true });
    root = createRoot(host);
    const runtime = { tracks: [track], clock, client, addWidgetBelow() {}, campaigns: [], recordings: [] };
    const widget = { id: 'land', kind: 'land-battles', trackIds: ['main'], countries: { kind: 'all' }, time: { mode: 'follow' }, options };
    root.render(<div style={{ height: 520, display: 'flex', flexDirection: 'column' }}><Scoped>
      <LandBattlesWidget {...{ widget, runtime, active: true, onDiagnostics() {}, setOption() {} } as unknown as WidgetProps} /></Scoped></div>);
    await settle(60);
  },
  async play(frames: number, hours: number) {
    clock.playing = true; clock.anchor();
    await settle(4);
    commits = []; why.clear(); observer?.takeRecords(); bindings = []; redrawnSince = 0; drawnRows = new Set(host!.querySelectorAll('.log-row'));
    const before = [...host!.querySelectorAll('.log-row')];
    const mains: number[] = [];
    for (let at = 0; at < frames; at++) mains.push(await timedFrame(() => clock.step(hours)));
    takeRecords();
    const after = [...host!.querySelectorAll('.log-row')];
    const played = { mains, commits, why: Object.fromEntries(why), rows: after.length, elements: host!.querySelectorAll('*').length, errors: [...errors],
      pooled: after.length > 0 && after.every(row => row instanceof HTMLElement && row.dataset.pool != null),
      sameBoxes: before.length === after.length && before.every((row, index) => row === after[index]),
      bindings: bindings.length, boundStates: new Set(bindings).size };
    clock.playing = false; clock.anchor();
    await settle();
    return played;
  },
  head: () => clock.getHour(),
  rows: () => host!.querySelectorAll('.log-row').length,
};
(window as unknown as { landPlayback: typeof fixture }).landPlayback = fixture;
