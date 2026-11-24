import { createRoot } from 'react-dom/client';
import type { Root } from 'react-dom/client';
import '../styles.css';
import '../widgets';
import { loadWidgetRenderer } from '../widgets/rendererLoaders';
import { widgetDefinitions } from './definitions';
import type { WidgetRuntime } from './definitions';
import { createWidget } from './model';
import type { BoundTrack, WidgetSpec, WidgetTime } from './model';
import { PlaybackController } from './playback';
import type { EngineFrame, FrameEngine } from '../engine/frame';
import { PHASES } from '../engine/frame';
import { MapViewsStore } from './geometry';
import { WidgetBody } from './widgets';
import { MeasuredWidget } from './widgetPerf';
import { measureWith } from '../shell/perf';
import type { PerfRecorder } from '../shell/perf';
import type { CountryGroupReply } from '../generated/dashboard';

class OnScreen {
  private readonly heard: IntersectionObserverCallback;
  constructor(heard: IntersectionObserverCallback) { this.heard = heard; }
  observe(target: Element) { queueMicrotask(() => this.heard([{ target, isIntersecting: true, intersectionRatio: 1 } as unknown as IntersectionObserverEntry], this as never)); }
  unobserve() {}
  disconnect() {}
  takeRecords() { return []; }
}
Object.assign(window, { IntersectionObserver: OnScreen });

const FIRST = 1000, LAST = 1009, OFFSET = 900, GAP = [1005, 1006] as const;
const GAP_REASON = 'Branch branch block 1005 – 1006 is unreadable: sweep';
const shown = new Map<string, number | null>();
const crashes = new Map<string, string>();
measureWith(new Proxy({
  shown: (widget: string, hour: number | null) => { shown.set(widget, hour); },
  widgetError: (widget: string | null, message: string) => { if (widget) crashes.set(widget, message); },
  reader: () => () => undefined,
}, { get: (target, name) => (target as Record<string | symbol, unknown>)[name] ?? (() => undefined) }) as unknown as PerfRecorder);

const groups: CountryGroupReply = {
  branchId: 'branch', warnings: [], warStarts: [], capitulations: [], identityChanges: [], factions: [], wars: [],
  humans: [{ tag: 'GER', firstHour: FIRST, lastHour: LAST }, { tag: 'ENG', firstHour: FIRST, lastHour: LAST }],
  unobserved: { humans: [], factions: [], wars: [] },
  countryIdentities: ['GER', 'ENG'].map((tag, index) => ({ tag, identities: [{ slot: index + 1, countryId: index + 101, firstHour: FIRST, lastHour: LAST }] })),
} as unknown as CountryGroupReply;

type Replies = 'pending' | 'failing';
const pending = new Promise<never>(() => {});
const requests: string[] = [];
const asked: { product: string; hour: number }[] = [];
const reply = (request: { sourceId: string; revision: string; product: string }, data: unknown) => ({ sourceId: request.sourceId, revision: request.revision, product: request.product, data });
const POINT = (key: string) => key === 'hour' || key === 'reads' || key.endsWith('Hour') && key !== 'firstHour' && key !== 'lastHour';
const hoursIn = (value: unknown, key = ''): number[] => Array.isArray(value) ? value.flatMap(item => hoursIn(item, key))
  : value && typeof value === 'object' ? Object.entries(value).flatMap(([name, item]) => hoursIn(item, name))
  : typeof value === 'number' && POINT(key) ? [value] : [];
const makeClient = (replies: Replies) => ({
  available: true,
  product: async (request: { sourceId: string; revision: string; product: string; params?: unknown }) => {
    requests.push(`${request.product} ${JSON.stringify(request.params)}`.slice(0, 300));
    if (request.product !== 'follow' && request.product !== 'follow-clock') for (const hour of new Set(hoursIn(request.params))) asked.push({ product: request.product, hour });
    if (request.product === 'follow') return reply(request, { available: false, reason: 'not followed in the sweep' });
    if (request.product === 'follow-clock' || request.product === 'unfollow') return reply(request, null);
    if (request.product === 'country-groups') return reply(request, groups);
    if (request.product === 'country-identities') return reply(request, { branchId: 'branch', countries: groups.countryIdentities, unknown: [], warnings: [] });
    if (replies === 'pending') return pending;
    throw new Error(`${request.product} is not recorded in the sweep`);
  },
  setMapViews: async () => {}, onMapStates: async () => () => {}, releaseSource: async () => {}, openSource: async () => { throw new Error('none'); },
});
const source = { id: 'sweep', path: 'sweep.battleplan', revision: '1', metadata: { path: 'sweep.battleplan', name: 'Sweep', format: 1, captureMode: 'full', campaignKey: 'campaign',
  branches: [{ id: 'branch', sessionId: 's', firstHour: FIRST, lastHour: LAST, main: true, fork: null, joins: [], sampleBoundary: '', gaps: [{ firstHour: GAP[0], lastHour: GAP[1], reason: GAP_REASON }] }],
  countries: [{ id: '1', tag: 'GER', name: 'Germany', observedBranchId: 'branch', observedHour: FIRST }, { id: '2', tag: 'ENG', name: 'England', observedBranchId: 'branch', observedHour: FIRST }],
  playerCountryId: '1', contentVerified: true, partial: null, warnings: [], buildWarnings: [], revision: '1' } };
const track = { id: 'main', roleId: 'primary', label: 'Main', branchIndex: 0, anchorOffset: 0, source, branch: source.metadata.branches[0], axisOffset: OFFSET,
  selectedTag: 'GER', selectedSlot: 1 } as unknown as BoundTrack;
let now = 1000;
performance.now = () => now;
const tasks = new Map<string, (frame: EngineFrame) => boolean | void>();
const engine = { every: (phase: string, run: (frame: EngineFrame) => boolean | void) => {
  tasks.set(phase, run);
  return { wake() {}, stop() { tasks.delete(phase); } };
} } as unknown as FrameEngine;
const tick = (ms: number) => {
  now += ms;
  for (const phase of PHASES) tasks.get(phase)?.({ now, dt: ms, index: 0, phase } as EngineFrame);
};
const clock = new PlaybackController(engine);
clock.setBounds({ firstHour: FIRST - OFFSET - 5, lastHour: LAST - OFFSET + 5 });
const makeRuntime = (replies: Replies, client = makeClient(replies)) => ({ campaignTracks: [track], defaultRoleId: 'primary', tracks: [track], clock,
  maps: new MapViewsStore(client as never, clock), onMapBranch() {}, client, alignment: 'anchors', nativeHidden: false, campaigns: [], recordings: [], assignCampaign: async () => {}, requestSeek: () => false,
  addWidgetBelow() {} }) as unknown as WidgetRuntime;

export type Mode = 'follow' | 'pin inside' | 'pin gap' | 'pin outside' | 'range';
export interface Case { id: string; kind: string; mode: Mode; show: string | null; scope: 'moment' | 'history' }
const TIMES: Record<Mode, WidgetTime> = {
  follow: { mode: 'follow' },
  'pin inside': { mode: 'pin', windows: { main: { firstOffset: 3, lastOffset: 3 } } },
  'pin gap': { mode: 'pin', windows: { main: { firstOffset: 5, lastOffset: 5 } } },
  'pin outside': { mode: 'pin', windows: { main: { firstOffset: 15, lastOffset: 15 } } },
  range: { mode: 'range', firstOffset: 1, lastOffset: 8 },
};
const timeMode = (mode: Mode) => TIMES[mode].mode;

function cases(): Case[] {
  const all: Case[] = [];
  for (const definition of widgetDefinitions()) {
    const field = definition.options?.find(option => option.key === 'show');
    const shows = [null, ...(field?.kind === 'choice' ? field.choices : []).map(choice => choice.value).filter(value => value !== 'main')];
    for (const mode of Object.keys(TIMES) as Mode[]) {
      if (!definition.timeModes.includes(timeMode(mode))) continue;
      for (const show of shows) all.push({ id: `${definition.kind}|${mode}|${show ?? 'main'}`, kind: definition.kind, mode, show, scope: definition.scope });
    }
  }
  return all;
}

let root: Root | null = null;
let mounted: Case[] = [];
const spec = (item: Case): WidgetSpec => {
  const made = createWidget(item.kind, 'main', item.id);
  return { ...made, time: TIMES[item.mode], options: { ...made.options, ...(item.show ? { show: item.show } : {}) } };
};
const turn = async () => {
  await new Promise<void>(resolve => setTimeout(resolve, 0));
  await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
};
const settle = async () => {
  let last = '', same = 0;
  for (let round = 0; round < 400 && same < 12; round++) {
    await turn();
    const now = document.body.innerHTML.length + ':' + requests.length;
    same = now === last ? same + 1 : 0;
    last = now;
  }
};

function regionStates(element: Element | null) {
  return element ? [...element.querySelectorAll<HTMLElement>('.widget-state')].map(state => `${state.dataset.state}: ${state.textContent ?? ''}`) : [];
}

function read(item: Case) {
  const host = document.getElementById(item.id);
  const drill = host?.querySelector('.drill-host[data-open] > .drill-view, .drill-saved, .drill-popout:has(.drill-body)');
  const all = regionStates(host ?? null), inDrill = regionStates(drill ?? null);
  const main = all.filter(state => !inDrill.includes(state));
  const drawn = !!host?.querySelector('canvas, svg, table, [role="grid"], [role="table"], [role="status"]') || !!host?.textContent?.trim();
  return { main, drawn, drill: inDrill, open: !!drill, shown: shown.has(item.id) ? shown.get(item.id)! : undefined, crash: crashes.get(item.id) ?? null,
    text: (host?.textContent ?? '').slice(0, 400) };
}

Object.assign(window, { timeSweep: {
  cases: () => cases(),
  requests: () => requests.splice(0),
  asked: () => asked.splice(0),
  async preload() { await Promise.all(widgetDefinitions().map(definition => loadWidgetRenderer(definition.kind).catch(() => undefined))); return widgetDefinitions().length; },
  async mount(ids: string[], replies: Replies, axis: number) {
    const runtime = makeRuntime(replies);
    root?.unmount();
    document.body.replaceChildren();
    shown.clear(); crashes.clear();
    clock.seek(axis);
    const byId = new Map(cases().map(item => [item.id, item]));
    mounted = ids.map(id => byId.get(id)!).filter(Boolean);
    const host = document.body.appendChild(document.createElement('div'));
    root = createRoot(host);
    root.render(<>{mounted.map(item => <section key={item.id} id={item.id} style={{ width: 640, height: 420, overflow: 'hidden', position: 'relative' }}>
      <MeasuredWidget.Provider value={item.id}><WidgetBody widget={spec(item)} runtime={runtime} /></MeasuredWidget.Provider>
    </section>)}</>);
    await settle();
    return Object.fromEntries(mounted.map(item => [item.id, read(item)]));
  },
  async seek(axis: number) {
    shown.clear();
    clock.seek(axis);
    await settle();
    return { hour: clock.getHour(), states: Object.fromEntries(mounted.map(item => [item.id, read(item)])) };
  },
  async step(direction: -1 | 1) {
    shown.clear();
    clock.step(direction);
    await settle();
    return { hour: clock.getHour(), states: Object.fromEntries(mounted.map(item => [item.id, read(item)])) };
  },
  async play(speed: number, count: number, ms: number) {
    shown.clear();
    clock.setSpeed(speed);
    clock.setPlaying(true);
    for (let frame = 0; frame < count; frame++) { tick(ms); await settle(); }
    const playing = { hour: clock.getHour(), advancing: clock.getSnapshot().playing, states: Object.fromEntries(mounted.map(item => [item.id, read(item)])) };
    clock.setPlaying(false);
    await settle();
    return { playing, paused: { hour: clock.getHour(), states: Object.fromEntries(mounted.map(item => [item.id, read(item)])) } };
  },
} });
