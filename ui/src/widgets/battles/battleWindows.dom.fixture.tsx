/** Land battles following the head at 1,440 h/s in a real browser, over a recording cut in windows of 720 hours as the
 * service cuts a long campaign: every request the widget makes, with the head it was sent at and the hour it asks from. */
import { createRoot } from 'react-dom/client';
import type { Root } from 'react-dom/client';
import LandBattlesWidget from '../land-battles/LandBattlesWidget';
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
/** Thirty battles begin in each window, a day apart, each lasting 300 hours, in a province of its own: the rows in view
 * are the window's at the head, as a recording's are. */
const BATTLES = 30;
const battle = (window: number, at: number) => {
  const start = WINDOWS[window]!.firstHour + 10 + at * 24, last = Math.min(start + 300, WINDOWS[window]!.lastHour);
  return { id: `B${window}-${at}`, provinceId: window * BATTLES + at + 1, start: { hour: start, bound: 'exact' }, end: last < start + 300 ? null : { hour: last, bound: 'exact' },
    endUnknownHour: null, counterBase: 0, priorPeak: null, priorSidePeaks: null, losses: [], result: null,
    runs: [{ firstHour: start, lastHour: last, counts: [3, 2], rosterUnknown: false, countries: [['GER'], ['SOV']], countriesUnknown: [false, false], duration: last - start }] };
};
const landWindow = (index: number) => ({ window: index, interval: WINDOWS[index], observed: [[WINDOWS[index]!.firstHour, WINDOWS[index]!.lastHour]], labels: [], reasons: [],
  warnings: [], contentWarnings: [], rows: Array.from({ length: BATTLES }, (_, at) => battle(index, at)) });

function reply(product: string, params: Record<string, unknown>): unknown {
  if (product === 'land-battles-manifest') return { windows: WINDOWS, available: true, unavailable: null, contentWarnings: [] };
  if (product === 'land-battles') return landWindow(params.window as number);
  if (product === 'land-battle-terrain') return { provinces: (params.provinces as number[]).map(provinceId => ({ provinceId, name: 'Plains', missing: null })), warnings: [] };
  if (product === 'place-names') return { places: (params.places as { provinceId: number; hour: number }[]).map(place => ({ ...place, name: `Province ${place.provinceId}`, missing: null })),
    states: [], warnings: [] };
  if (product === 'country-styles') return { branchId: '1', hour: params.hour, styles: [], warnings: [] };
  if (product === 'country-facts') return { branchId: '1', known: [[0, HOURS - 1]], names: [], reasons: [], flags: [], countries: [], warnings: [] };
  if (product === 'follow') return { available: false, reason: 'not followed here' };
  if (product === 'unfollow' || product === 'follow-clock') return {};
  throw new Error(`${product} is not served here`);
}

/** The hour a request asks from, as the perf log names it: its own `asksFrom`, else the earliest hour its params list. */
function askedFrom(request: { asksFrom?: number; params: Record<string, unknown> }): number | null {
  if (typeof request.asksFrom === 'number') return request.asksFrom;
  for (const name of ['from', 'firstHour', 'hour', 'at']) if (typeof request.params[name] === 'number') return request.params[name] as number;
  const listed = (request.params.places as { hour: number }[] | undefined)?.map(place => place.hour) ?? [];
  return listed.length ? Math.min(...listed) : null;
}

let clock = new Clock();
const requests: { product: string; window: number | null; from: number | null; head: number; playing: boolean }[] = [];
let root: Root | null = null;
let host: HTMLDivElement | null = null;

const client = {
  available: true,
  product: async (request: { sourceId: string; revision: string; product: string; params: Record<string, unknown>; asksFrom?: number }) => {
    requests.push({ product: request.product, window: typeof request.params.window === 'number' ? request.params.window : null, from: askedFrom(request),
      head: clock.getHour(), playing: clock.playing });
    return { sourceId: request.sourceId, revision: request.revision, product: request.product, data: reply(request.product, request.params) };
  },
  setMapViews: async () => {}, onMapStates: async () => () => {}, releaseSource: async () => {}, openSource: async () => { throw new Error('none'); },
};

function Scoped({ children }: { children: React.ReactNode }) {
  return <MeasuredWidget value="land">{children}</MeasuredWidget>;
}

export const fixture = {
  async mount() {
    root?.unmount(); host?.remove();
    clock = new Clock(); requests.length = 0;
    host = document.body.appendChild(document.createElement('div'));
    host.style.cssText = 'width: 1200px; display: flex; flex-direction: column;';
    root = createRoot(host);
    const runtime = { tracks: [track], clock, client, addWidgetBelow() {}, campaigns: [], recordings: [] };
    const widget = { id: 'land', kind: 'land-battles', trackIds: ['main'], countries: { kind: 'all' }, time: { mode: 'follow' }, options: {} };
    root.render(<div style={{ height: 420, display: 'flex', flexDirection: 'column' }}><Scoped>
      <LandBattlesWidget {...{ widget, runtime, active: true, onDiagnostics() {}, setOption() {} } as unknown as WidgetProps} /></Scoped></div>);
    await settle(40);
  },
  /** Plays `frames` frames of `hours` each, settling a moment after anchoring as the app does. */
  async play(frames: number, hours: number) {
    clock.playing = true; clock.anchor();
    for (let at = 0; at < frames; at++) { clock.step(hours); await frame(); }
    await settle();
  },
  pause() { clock.playing = false; clock.anchor(); },
  head: () => clock.getHour(),
  /** Every request made while playing, with the head it was sent at and the hour it asks from. */
  played: () => requests.filter(request => request.playing),
  rows: () => host!.querySelectorAll('.log-row').length,
};
(window as unknown as { battleWindows: typeof fixture }).battleWindows = fixture;
