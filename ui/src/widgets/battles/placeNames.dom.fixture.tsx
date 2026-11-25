import { Profiler } from 'react';
import type { ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import type { Root } from 'react-dom/client';
import LandBattlesWidget from '../land-battles/LandBattlesWidget';
import { MeasuredWidget } from '../../dashboard/widgetPerf.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import '../../styles.css';

const native = window.requestAnimationFrame.bind(window);
const frame = () => new Promise<void>(resolve => native(() => resolve()));
async function settle(frames = 8) { for (let at = 0; at < frames; at++) await frame(); }

const HOURS = 100_000;
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

const WINDOWS = [{ firstHour: 0, lastHour: 27_999 }, { firstHour: 28_000, lastHour: 31_999 }, { firstHour: 32_000, lastHour: HOURS - 1 }];
const STARTS = [...Array.from({ length: 300 }, (_, at) => 28_000 + at * 6), ...Array.from({ length: 20 }, (_, at) => 30_100 + at * 40)];
const battle = (start: number, at: number) => ({ id: `B${at + 1}`, provinceId: at + 1, start: { hour: start, bound: 'exact' }, end: null, endUnknownHour: null,
  counterBase: 0, priorPeak: null, priorSidePeaks: null, losses: [], result: null,
  runs: [{ firstHour: start, lastHour: WINDOWS[1].lastHour, counts: [3, 2], rosterUnknown: false, countries: [['GER'], ['SOV']], countriesUnknown: [false, false], duration: 0 }] });
const landWindow = (index: number) => ({ window: index, interval: WINDOWS[index], observed: [[WINDOWS[index].firstHour, WINDOWS[index].lastHour]], labels: [], reasons: [],
  warnings: [], contentWarnings: [], rows: index === 1 ? STARTS.map(battle) : [] });

const asks: { land: boolean; places: { provinceId: number; hour: number }[]; landed: boolean; release: () => void }[] = [];
const named = (place: { provinceId: number; hour: number }) => ({ provinceId: place.provinceId, hour: place.hour, name: `Province ${place.provinceId}`, missing: null });

function reply(product: string, params: Record<string, unknown>): unknown {
  if (product === 'land-battles-manifest') return { windows: WINDOWS, available: true, unavailable: null, contentWarnings: [] };
  if (product === 'land-battles') return landWindow(params.window as number);
  if (product === 'land-battle-terrain') return { provinces: (params.provinces as number[]).map(provinceId => ({ provinceId, name: 'Plains', missing: null })), warnings: [] };
  if (product === 'country-styles') return { branchId: '1', hour: params.hour, styles: [], warnings: [] };
  if (product === 'country-facts') return { branchId: '1', known: [[0, HOURS - 1]], names: [], reasons: [], flags: [], countries: [], warnings: [] };
  if (product === 'follow') return { available: false, reason: 'not followed here' };
  if (product === 'unfollow' || product === 'follow-clock') return {};
  throw new Error(`${product} is not served here`);
}

let clock = new Clock();
const requests: string[] = [];
const renders: Record<string, number> = {};
let root: Root | null = null;
let host: HTMLDivElement | null = null;

const client = {
  available: true,
  product: async (request: { sourceId: string; revision: string; product: string; params: Record<string, unknown> }) => {
    requests.push(request.product);
    const answer = (data: unknown) => ({ sourceId: request.sourceId, revision: request.revision, product: request.product, data });
    if (request.product !== 'place-names') return answer(reply(request.product, request.params));
    const places = request.params.places as { provinceId: number; hour: number }[];
    return new Promise(resolve => {
      const ask = { land: request.params.land === true, places, landed: false, release: () => {} };
      ask.release = () => { ask.landed = true; resolve(answer({ places: places.map(named), states: [], warnings: [] })); };
      asks.push(ask);
    });
  },
  setMapViews: async () => {}, onMapStates: async () => () => {}, releaseSource: async () => {}, openSource: async () => { throw new Error('none'); },
};

function Counted({ id, children }: { id: string; children: ReactNode }) {
  return <MeasuredWidget value={id}>
    <Profiler id={id} onRender={() => { renders[id] = (renders[id] ?? 0) + 1; }}>{children}</Profiler>
  </MeasuredWidget>;
}

function shownRows(selector: string): HTMLElement[] {
  return [...host!.querySelectorAll<HTMLElement>(`${selector} .log-row`)]
    .filter(row => (row.dataset.reveal === undefined || row.dataset.reveal === 'in') && !row.classList.contains('is-leaving'))
    .map(row => ({ row, y: Number.parseFloat(row.style.transform.replace('translateY(', '')), box: row.getBoundingClientRect() }))
    .filter(({ box }) => { const view = row(selector).getBoundingClientRect(); return box.bottom > view.top && box.top < view.bottom; })
    .sort((a, b) => a.y - b.y).map(({ row }) => row);
}
const row = (selector: string) => host!.querySelector<HTMLElement>(`${selector} .log-scroll`)!;
const shownState = (element: HTMLElement) => element.querySelector<HTMLElement>(':scope > .log-state:not([hidden])') ?? element;

export const fixture = {
  async mount() {
    root?.unmount(); host?.remove();
    clock = new Clock(); requests.length = 0; asks.length = 0;
    for (const key of Object.keys(renders)) delete renders[key];
    host = document.body.appendChild(document.createElement('div'));
    host.style.cssText = 'width: 1200px; display: flex; flex-direction: column;';
    root = createRoot(host);
    const runtime = { tracks: [track], clock, client, addWidgetBelow() {}, campaigns: [], recordings: [] };
    const props = (widget: unknown) => ({ widget, runtime, active: true, onDiagnostics() {}, setOption() {} }) as unknown as WidgetProps;
    const land = { id: 'land', kind: 'land-battles', trackIds: ['main'], countries: { kind: 'all' }, time: { mode: 'follow' }, options: {} };
    root.render(<div data-case="land" style={{ height: 420, display: 'flex', flexDirection: 'column' }}><Counted id="land"><LandBattlesWidget {...props(land)} /></Counted></div>);
    await settle(40);
  },
  asks: () => asks.map(ask => ({ land: ask.land, provinces: ask.places.map(place => place.provinceId), landed: ask.landed })),
  release(index: number) { asks[index]!.release(); },
  frame,
  settle,
  landShown: () => shownRows('[data-case="land"]').map(element => {
    const cells = shownState(element).querySelector<HTMLElement>('[data-battle]');
    return { province: Number(cells?.dataset.battle?.slice(1)), place: cells?.querySelector('.lb-place b')?.textContent ?? '' };
  }),
  async landAndCount(indices: number[], hours = 0) {
    for (const index of indices) asks[index]!.release();
    await new Promise(resolve => setTimeout(resolve, 0));
    const before = { ...renders };
    const named = () => fixture.landShown().every(shown => shown.place === `Province ${shown.province}`);
    let frames = 0;
    while (!named() && frames < 30) { if (clock.playing) clock.step(hours); await frame(); frames++; }
    return { frames, commits: Object.fromEntries(Object.entries(renders).map(([id, count]) => [id, count - (before[id] ?? 0)])) };
  },
  renders: () => ({ ...renders }),
  async play(frames: number, hours: number) {
    clock.playing = true; clock.anchor();
    for (let at = 0; at < frames; at++) { clock.step(hours); await frame(); }
  },
  async step(hours: number) { clock.step(hours); await frame(); },
  head: () => clock.getHour(),
  pause() { clock.playing = false; clock.anchor(); },
  requests: () => [...requests],
};
(window as unknown as { placeNames: typeof fixture }).placeNames = fixture;
