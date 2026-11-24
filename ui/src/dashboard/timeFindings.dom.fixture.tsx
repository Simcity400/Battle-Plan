import { createElement } from 'react';
import { createRoot } from 'react-dom/client';
import type { Root } from 'react-dom/client';
import '../widgets';
import { loadWidgetRenderer } from '../widgets/rendererLoaders';
import type { WidgetRuntime } from './definitions';
import { createWidget } from './model';
import type { BoundTrack, WidgetSpec } from './model';
import { PlaybackController } from './playback';
import type { EngineFrame, FrameEngine } from '../engine/frame';
import { PHASES } from '../engine/frame';
import { TimeSection } from './settings/TimeSection';
import type { WidgetTime } from './model';
import { MapViewsStore } from './geometry';
import { WidgetBody } from './widgets';
import { WidgetFrameContext, widgetWording } from './widgetFrame';
import { widgetDefinition } from './definitions';
import { datedToken } from '../test-support/gated.ts';
import { subjectToken } from './chosen';
import { dayLabel } from '../charts/draw';
import { contentKey } from './options';
import { forgetProducts } from './products';
import { datedPanel } from './follow/messages.fixture.ts';
import { packet, splitForLanes } from './follow/split.ts';
import { pipelineFor } from './follow/pipeline.ts';
import { pageClock } from './widgetClock.ts';

class OnScreen {
  private readonly heard: IntersectionObserverCallback;
  constructor(heard: IntersectionObserverCallback) { this.heard = heard; }
  observe(target: Element) { queueMicrotask(() => this.heard([{ target, isIntersecting: true, intersectionRatio: 1 } as unknown as IntersectionObserverEntry], this as never)); }
  unobserve() {}
  disconnect() {}
  takeRecords() { return []; }
}
Object.assign(window, { IntersectionObserver: OnScreen });
const errors: string[] = [];
const logged = console.error.bind(console);
console.error = (...args: unknown[]) => { errors.push(args.map(item => item instanceof Error ? item.message : String(item)).join(' ').slice(0, 500)); logged(...args); };

const FIRST = 100, LAST = 109;
type Reply = (hour: number | null, params: Record<string, unknown>) => unknown;
interface Ask { product: string; hour: number | null; hours: number[]; params: Record<string, unknown> | null }
const DATED = (key: string) => key === 'hour' || key === 'hours' || key === 'reads' || key === 'from' || key === 'to' || key.endsWith('Hour');
const hoursIn = (value: unknown, key = ''): number[] => Array.isArray(value) ? value.flatMap(item => hoursIn(item, key))
  : value !== null && typeof value === 'object' ? Object.entries(value).flatMap(([name, item]) => hoursIn(item, name))
  : typeof value === 'number' && DATED(key) ? [value] : [];
const follows = new Map<number, string>();
const ROUTE = 9;
let ports: readonly MessagePort[] = [];
const panelFollows = new Map<number, { panel: string; params: Record<string, unknown> }>();
const pushed = new Map<number, number>();
const landed = () => {
  if (!pushed.size || clock.getSnapshot().playing) return true;
  const pipeline = pipelineFor(pageClock(clock), client as never, () => { throw new Error('missing pipeline'); });
  const views = [...pipeline.page.widgets.keys()].flatMap(widget => { const view = pipeline.view(widget); return view?.fields ? [view] : []; });
  const perFollow = (key: string) => key.startsWith('model:') || key.startsWith('refused:');
  const holds = (fields: Readonly<Record<string, unknown>>, id: number) => `model:${id}` in fields || `refused:${id}` in fields || !Object.keys(fields).some(perFollow);
  return [...pushed].every(([id, hour]) => views.some(view => holds(view.fields!, id) && (view.through ?? -Infinity) >= Math.min(hour, view.target)));
};
const refusedPanel = (id: number, hour: number, reason: string, seek: number) => {
  const body = new TextEncoder().encode(reason), out = new Uint8Array((40 + body.length + 7) & ~7), words = new Uint32Array(out.buffer);
  words.set([0x324c4e50, 6, 1, 0, ROUTE, seek, id, hour, 0, body.length]);
  out.set(body, 40);
  return out;
};
const pushPanels = (hour: number, only?: number) => {
  const seek = clock.getSnapshot().seekSequence;
  for (const [id, held] of panelFollows) {
    const reply = only === undefined || id === only ? state.replies[held.panel] : undefined;
    if (!reply || state.failures.has(held.panel)) continue;
    let bytes: Uint8Array;
    try { bytes = datedPanel(id, hour, reply(hour, held.params), seek, ROUTE); }
    catch (error) { bytes = refusedPanel(id, hour, error instanceof Error ? error.message : String(error), seek); }
    pushed.set(id, hour);
    const split = splitForLanes(bytes, ports.length);
    if (split.kind !== 'failure') split.lanes.forEach((part, lane) => { if (part) ports[lane]!.postMessage(packet('panels', part, undefined), [part.buffer]); });
  }
};
const followLog: string[] = [];
const state = {
  asks: [] as Ask[], options: [] as [string, unknown][], pins: [] as unknown[], replies: {} as Record<string, Reply>, failures: new Set<string>(),
  groups: null as unknown, raids: undefined as unknown, props: {} as Record<string, Record<string, unknown>>, accept: true,
  shown: [] as (number | null)[],
};
const reportShown = (hour: number | null) => { state.shown.push(hour); };
const capture = (name: string) => (props: Record<string, unknown>) => { state.props[name] = props; return createElement('p', { 'data-stub': name }, String(props.label ?? props.title ?? name)); };
Object.assign(globalThis, { __finding: {
  capture,
  shownSink: () => (_widget: string, hour: number | null) => reportShown(hour),
  importing: (held: Promise<unknown>) => { importing++; void held.finally(() => { importing--; }); },
  useRaidLog: (_widget: unknown, runtime: WidgetRuntime) => state.raids === undefined ? undefined
    : state.raids === 'failed' ? { track: runtime.tracks[0], data: undefined, error: new Error('raids failed'), retry() {} }
    : { track: runtime.tracks[0], data: state.raids, error: undefined, retry() {} },
} });

const pending = new Promise<never>(() => {});
const answer = (request: { sourceId: string; revision: string; product: string }, data: unknown) => ({ sourceId: request.sourceId, revision: request.revision, product: request.product, data });
let inFlight = 0, importing = 0, mutations = 0;
const delivered = <T,>(work: () => T | Promise<never>): Promise<T> => {
  let reply: T | Promise<never>;
  try { reply = work(); } catch (error) { reply = Promise.reject(error); }
  if (reply === pending) return pending;
  inFlight++;
  return Promise.resolve(reply).finally(() => { inFlight--; });
};
const client = {
  available: true,
  openFollow: async (opened: readonly MessagePort[]) => { ports = opened; },
  product: (request: { sourceId: string; revision: string; product: string; params?: Record<string, unknown> }) => delivered(() => {
    if (request.product === 'follow-clock') return answer(request, { available: true });
    const params = request.params ?? null;
    if (request.product === 'unfollow') { follows.delete(params?.id as number); panelFollows.delete(params?.id as number); pushed.delete(params?.id as number); followLog.push(`unfollow ${params?.id}`); return answer(request, null); }
    if (request.product === 'follow') {
      state.asks.push({ product: 'follow', hour: null, hours: hoursIn(params?.query), params });
      followLog.push(`follow ${params?.id}`);
      if (!state.accept) return answer(request, { available: false, reason: 'not followed here' });
      const id = params?.id as number, query = (params?.query ?? {}) as Record<string, unknown>;
      follows.set(id, JSON.stringify(query));
      if (typeof query.panel === 'string') {
        const { panel, ...asked } = query;
        panelFollows.set(id, { panel, params: asked });
        queueMicrotask(() => pushPanels(clock.getHour(), id));
      }
      return answer(request, { available: true, route: ROUTE });
    }
    const hour = typeof params?.hour === 'number' ? params.hour : null;
    state.asks.push({ product: request.product, hour, hours: hoursIn(params), params });
    if (state.failures.has(request.product)) throw new Error(`${request.product} failed`);
    if (request.product === 'country-groups') return state.groups ? answer(request, state.groups) : pending;
    const reply = state.replies[request.product];
    return reply ? answer(request, reply(hour, params ?? {})) : pending;
  }),
  setMapViews: async () => {}, onMapStates: async () => () => {}, releaseSource: async () => {}, openSource: async () => { throw new Error('none'); },
};
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
clock.setBounds({ firstHour: 0, lastHour: 200 });
clock.onPublish((previous, next) => { if (next.hour !== previous.hour || next.seekSequence !== previous.seekSequence) pushPanels(next.hour); });
const sourceOf = (gaps: { firstHour: number; lastHour: number; reason: string }[], last = LAST) => ({ id: 'findings', path: 'findings.battleplan', revision: '1',
  metadata: { path: 'findings.battleplan', name: 'Findings', format: 1, captureMode: 'full', campaignKey: 'campaign',
    branches: [{ id: '1', sessionId: 's', firstHour: FIRST, lastHour: last, main: true, fork: null, joins: [], sampleBoundary: '', gaps }],
    countries: [{ id: '1', tag: 'GER', name: 'Germany', observedBranchId: '1', observedHour: FIRST }, { id: '2', tag: 'ENG', name: 'England', observedBranchId: '1', observedHour: FIRST }],
    playerCountryId: '1', contentVerified: true, partial: null, warnings: [], buildWarnings: [], revision: '1' } });
const runtimeOf = (gaps: { firstHour: number; lastHour: number; reason: string }[], last = LAST) => {
  const source = sourceOf(gaps, last);
  const track = { id: 'main', roleId: 'primary', label: 'Main', branchIndex: 0, anchorOffset: 0, source, branch: source.metadata.branches[0], axisOffset: 0,
    selectedTag: 'GER', selectedSlot: null } as unknown as BoundTrack;
  return { campaignTracks: [track], defaultRoleId: 'primary', tracks: [track], clock, maps: new MapViewsStore(client as never, clock), onMapBranch() {}, client,
    alignment: 'calendar', nativeHidden: false, campaigns: [], recordings: [], assignCampaign: async () => {}, requestSeek: () => false,
    addWidgetBelow(_id: string, spec: unknown) { state.pins.push(JSON.parse(JSON.stringify(spec))); } } as unknown as WidgetRuntime;
};

const turn = async () => {
  await new Promise<void>(resolve => setTimeout(resolve, 0));
  await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
};
new MutationObserver(records => { mutations += records.length; }).observe(document.documentElement, { subtree: true, childList: true, attributes: true, characterData: true });
const quietState = () => `${mutations}:${state.asks.length}:${state.options.length}:${state.pins.length}:${state.shown.length}:${followLog.length}:${errors.length}`;
const settle = async () => {
  for (let quiet = 0; quiet < 2;) {
    const before = quietState();
    await turn();
    quiet = inFlight === 0 && importing === 0 && landed() && quietState() === before ? quiet + 1 : 0;
  }
};
let root: Root | null = null;
const host = () => document.getElementById('finding');
const read = () => {
  const element = host();
  const drill = element?.querySelector('.drill-host[data-open] > .drill-view, .drill-saved, .drill-popout:has(.drill-body)');
  return { text: element?.textContent ?? '', drillOpen: !!drill, drill: drill?.textContent ?? '', day: drill?.querySelector('.drill-day')?.textContent ?? null,
    asks: state.asks.splice(0), options: state.options.splice(0), pins: state.pins.splice(0),
    follows: [...follows.values()], followLog: followLog.splice(0), shown: state.shown.splice(0),
    buttons: [...element?.querySelectorAll('button') ?? []].map(button => button.getAttribute('aria-label') ?? button.textContent ?? ''), errors: errors.splice(0) };
};

export interface Scene { kind: string; time: WidgetSpec['time']; options?: Record<string, unknown>; countries?: WidgetSpec['countries']; hour: number;
  replies?: Record<string, string>; failures?: string[]; groups?: unknown; raids?: unknown; gaps?: { firstHour: number; lastHour: number; reason: string }[];
  accept?: boolean; last?: number; requestsWhilePlaying?: boolean }

const dated = new Map<string, boolean | undefined>();
const observeRequestsWhilePlaying = (kind: string, observe: boolean | undefined) => {
  for (const [held, was] of dated) Object.assign(widgetDefinition(held)!, { playsBackDated: was });
  dated.clear();
  const definition = widgetDefinition(kind);
  if (!observe || !definition) return;
  dated.set(kind, definition.playsBackDated);
  Object.assign(definition, { playsBackDated: true });
};

const replyOf = (body: string): Reply => new Function('hour', 'params', `return (${body});`) as Reply;

Object.assign(window, { findings: {
  subjectToken: (handle: string) => subjectToken('campaign', handle),
  identityToken: (originOffset: number) => datedToken({ campaign: 'campaign', branch: '1', tag: 'GER', originOffset }),
  day: (hour: number) => dayLabel(hour),
  content: (token: string) => contentKey(token),
  async mount(scene: Scene) {
    await loadWidgetRenderer(scene.kind);
    root?.unmount();
    await settle();
    follows.clear(); panelFollows.clear(); pushed.clear(); followLog.length = 0;
    forgetProducts(client as never, 'findings');
    document.body.replaceChildren();
    Object.assign(state, { asks: [], options: [], pins: [], shown: [], props: {}, replies: Object.fromEntries(Object.entries(scene.replies ?? {}).map(([name, body]) => [name, replyOf(body)])),
      failures: new Set(scene.failures ?? []), groups: scene.groups ?? null, raids: scene.raids, accept: scene.accept ?? true });
    clock.setPlaying(false);
    clock.setBounds({ firstHour: 0, lastHour: Math.max(200, (scene.last ?? LAST) + 100) });
    clock.seek(scene.hour);
    observeRequestsWhilePlaying(scene.kind, scene.requestsWhilePlaying);
    const made = createWidget(scene.kind, 'main', 'finding');
    const widget = { ...made, time: scene.time, countries: scene.countries ?? made.countries, options: { ...made.options, ...scene.options } } as WidgetSpec;
    const element = document.body.appendChild(document.createElement('div'));
    element.id = 'finding';
    element.style.cssText = 'width: 900px; height: 600px; position: relative';
    const runtime = runtimeOf(scene.gaps ?? [], scene.last);
    const wording = widgetWording(undefined, widget.title);
    const frame = {
      title: widget.title, subject: wording.subject, things: wording.things,
      timeMode: widget.time.mode, pinned: widget.time.mode === 'pin' || widget.time.mode === 'range',
      bounds: { firstHour: FIRST, lastHour: scene.last ?? LAST }, hasSubject: true, captionSlot: null,
      openCountries: () => {}, openTime: () => {}, reportState: () => {}, announce: () => {}, clearAnnouncement: () => {},
      setDynamicViewSwitch: () => {}, skeleton: 'generic' as const, setSkeleton: () => {},
    };
    root = createRoot(element);
    root.render(<WidgetFrameContext.Provider value={frame}><WidgetBody widget={widget} runtime={runtime} setOption={(key, value) => { state.options.push([key, value]); }} /></WidgetFrameContext.Provider>);
    await settle();
    return read();
  },
  async seek(hour: number) { clock.seek(hour); await settle(); return read(); },
  async play(speed: number, frames: number, ms: number) {
    clock.setSpeed(speed);
    clock.setPlaying(true);
    for (let frame = 0; frame < frames; frame++) { tick(ms); await settle(); }
    return { hour: clock.getHour(), playing: clock.getSnapshot().playing, ...read() };
  },
  async pause() { clock.setPlaying(false); await settle(); return read(); },
  async settings(scene: { kind: string; time: WidgetTime; hour: number; gaps?: { firstHour: number; lastHour: number; reason: string }[] }) {
    root?.unmount();
    await settle();
    document.body.replaceChildren();
    Object.assign(state, { asks: [], options: [], pins: [] });
    clock.setPlaying(false);
    clock.seek(scene.hour);
    const runtime = runtimeOf(scene.gaps ?? []);
    const widget = { ...createWidget(scene.kind, 'main', 'finding'), time: scene.time } as WidgetSpec;
    const element = document.body.appendChild(document.createElement('div'));
    element.id = 'finding';
    root = createRoot(element);
    root.render(<TimeSection widget={widget} effective={widget} reference={runtime.tracks[0]} runtime={runtime} modes={['follow', 'campaign', 'pin', 'range']}
      onChange={time => { state.options.push(['time', time]); }} />);
    await settle();
    return read();
  },
  async press(label: string) {
    const button = [...host()?.querySelectorAll('button') ?? []].find(item => (item.getAttribute('aria-label') ?? item.textContent ?? '').includes(label));
    button?.click(); await settle(); return { pressed: !!button, ...read() };
  },
  async click(selector: string) {
    const target = host()?.querySelector<HTMLElement>(selector);
    target?.click(); await settle(); return { clicked: !!target, ...read() };
  },
  has: (selector: string) => !!host()?.querySelector(selector),
  async expand(selector: string) {
    const details = host()?.querySelector<HTMLDetailsElement>(selector);
    if (details) { details.open = true; details.dispatchEvent(new Event('toggle')); }
    await settle(); return { expanded: !!details, ...read() };
  },
  async call(name: string, method: string, ...args: unknown[]) {
    const fn = state.props[name]?.[method] as ((...values: unknown[]) => unknown) | undefined;
    if (!fn) return { called: false, ...read() };
    fn(...args); await settle(); return { called: true, ...read() };
  },
  prop: (name: string, key: string) => { const value = state.props[name]?.[key]; return value === undefined || typeof value === 'object' || typeof value === 'function' ? null : value; },
  stateLabel: (name: string) => (state.props[name]?.state as { props?: { label?: unknown } } | undefined)?.props?.label ?? null,
  props: (name: string) => {
    const props = state.props[name];
    return props ? JSON.parse(JSON.stringify(props, (_key, value) => typeof value === 'function' ? undefined : value)) : null;
  },
} });
