import { createElement, useContext, useLayoutEffect, useState, useSyncExternalStore } from 'react';
import type { Context } from 'react';
import * as times from './widgetTime';
import { createRoot } from 'react-dom/client';
import type { Root } from 'react-dom/client';
import '../widgets';
import { loadWidgetRenderer } from '../widgets/rendererLoaders';
import type { WidgetRuntime } from './definitions';
import { createWidget, resolveWindow } from './model';
import type { BoundTrack, WidgetSpec } from './model';
import { PlaybackController } from './playback';
import { MapViewsStore } from './geometry';
import { WidgetBody } from './widgets';
import { datedCountryToken } from './datedCountry';
import { subjectToken } from './chosen';
import NavyManagement from '../widgets/navy-size/NavyManagement';

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
console.error = (...args: unknown[]) => { errors.push(args.map(item => item instanceof Error ? `${item.message} ${item.stack?.split(String.fromCharCode(10)).slice(0, 3).join(' ')}` : String(item)).join(' ').slice(0, 500)); logged(...args); };

const FIRST = 100, LAST = 109;
type Reply = (hour: number, params: Record<string, unknown>) => unknown;
interface Ask { product: string; hour: number | null; params: Record<string, unknown> | null }
const state = {
  asks: [] as Ask[], options: [] as [string, unknown][], replies: {} as Record<string, Reply>, products: {} as Record<string, unknown>,
  groups: null as unknown, trade: undefined as unknown, raids: undefined as unknown, drill: null as unknown, values: {} as Record<string, unknown>, props: {} as Record<string, Record<string, unknown>>,
  listeners: new Set<() => void>(),
};
const answered = new Map<string, unknown>();
const changed = () => { for (const listener of [...state.listeners]) listener(); };
const subscribe = (listener: () => void) => { state.listeners.add(listener); return () => { state.listeners.delete(listener); }; };
const values = { get: () => undefined, missing: () => 'Not recorded' };
const live = { read: () => values, subscribe: () => () => {} };

type Registered = { heads: { set(token: object, active: boolean): void } };
const timeApi = times as unknown as { useRegion?: (widget: WidgetSpec, action?: unknown) => unknown; useReadsRegion?: (region: unknown, active: boolean) => void;
  WidgetTimeContext?: Context<Registered | null>; readsHostTime?: (host: Registered | null, widget: WidgetSpec, hover: number | null) => boolean };
function useRegistered(widget: WidgetSpec, options: { active?: boolean; time?: unknown; hoverHour?: number | null }) {
  const active = options.active !== false;
  if (timeApi.useReadsRegion && timeApi.useRegion) { timeApi.useReadsRegion(timeApi.useRegion(widget, options.time), active); return; }
  const host = useContext(timeApi.WidgetTimeContext!);
  const heads = host && timeApi.readsHostTime!(host, widget, options.hoverHour ?? null) ? host.heads : null;
  const [token] = useState(() => ({}));
  useLayoutEffect(() => {
    if (!heads || !active) return;
    heads.set(token, true);
    return () => heads.set(token, false);
  }, [heads, token, active]);
}

function head(runtime: WidgetRuntime, widget: WidgetSpec, product: string, params: Record<string, unknown> | null, options: { active?: boolean; time?: { kind: string; hour?: number | null }; hoverHour?: number | null }) {
  useSyncExternalStore(runtime.clock.subscribe, runtime.clock.getHour, runtime.clock.getHour);
  useSyncExternalStore(subscribe, () => state.replies, () => state.replies);
  useRegistered(widget, options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const time = options.time;
  const raw = !track ? null : time ? time.kind === 'at' ? time.hour ?? null
    : resolveWindow(time.kind === 'follow' ? { ...widget, time: { mode: 'follow' } } : widget, track, runtime.tracks, runtime.clock.getHour())?.lastHour ?? null
    : options.hoverHour ?? resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour())?.lastHour ?? null;
  const hour = raw === null ? null : Math.floor(raw);
  const recorded = !!track && hour !== null && hour >= track.branch.firstHour && hour <= track.branch.lastHour;
  if (params && options.active !== false) state.asks.push({ product, hour: recorded ? hour : null, params });
  const reply = product in state.replies ? state.replies[product] : undefined;
  const key = JSON.stringify([product, hour, params]);
  if (recorded && params && options.active !== false && reply && !answered.has(key)) answered.set(key, reply(hour!, params));
  const data = recorded && params && options.active !== false && reply ? answered.get(key) : undefined;
  return { data, error: undefined, retry() {}, root() {}, live, outside: !recorded, outsideDetail: undefined, hour: recorded ? hour : null,
    at: data === undefined ? null : hour, atNow: () => data === undefined ? null : hour, shownAt: () => data === undefined ? null : hour, current: () => data,
    read: recorded ? 'asked' : 'off' };
}

function useDrill<S, A>(reduce: (state: S, action: A) => S, initial: S): [S, (action: A) => void] {
  const held = useSyncExternalStore(subscribe, () => state.drill, () => state.drill);
  return [(held ?? initial) as S, (action: A) => { state.drill = reduce((state.drill ?? initial) as S, action); changed(); }];
}
function useValue<T>(name: string, initial: T): [T, (next: T | ((before: T) => T)) => void] {
  const held = useSyncExternalStore(subscribe, () => state.values[name], () => state.values[name]);
  return [(name in state.values ? held : initial) as T, next => {
    state.values[name] = typeof next === 'function' ? (next as (before: T) => T)(state.values[name] as T) : next; changed();
  }];
}

const stub = {
  head, useDrill, useValue,
  useRaidLog: (_widget: unknown, runtime: WidgetRuntime) => state.raids === undefined ? undefined
    : { track: runtime.tracks[0], data: state.raids, error: undefined, retry() {} },
  useTradeFlows: (_widget: unknown, runtime: WidgetRuntime) => state.trade === undefined ? undefined
    : { track: runtime.tracks[0], all: true, resolved: null, groups: stub.groups(), scope: null, data: state.trade, error: undefined, retry() {}, extent: [100, 109] },
  groups: () => ({ data: state.groups ?? undefined, complete: true, error: undefined, retry() {} }),
  product: (_client: unknown, request: { product: string; params: unknown } | null) => request && request.product in state.products
    ? { data: state.products[request.product], retry() {}, error: undefined } : undefined,
  capture: (name: string) => (props: Record<string, unknown>) => { state.props[name] = props; return createElement('p', { 'data-stub': name }, String(props.label ?? props.title ?? name)); },
};
Object.assign(globalThis, { __finding: stub });

const source = { id: 'findings', path: 'findings.battleplan', revision: '1', metadata: { path: 'findings.battleplan', name: 'Findings', format: 1, captureMode: 'full', campaignKey: 'campaign',
  branches: [{ id: '1', sessionId: 's', firstHour: FIRST, lastHour: LAST, main: true, fork: null, joins: [], sampleBoundary: '', gaps: [] }],
  countries: [{ id: '1', tag: 'GER', name: 'Germany', observedBranchId: '1', observedHour: FIRST }, { id: '2', tag: 'ENG', name: 'England', observedBranchId: '1', observedHour: FIRST }],
  playerCountryId: '1', contentVerified: true, partial: null, warnings: [], buildWarnings: [], revision: '1' } };
const track = { id: 'main', roleId: 'primary', label: 'Main', branchIndex: 0, anchorOffset: 0, source, branch: source.metadata.branches[0], axisOffset: 0,
  selectedTag: 'GER', selectedSlot: null } as unknown as BoundTrack;
const pending = new Promise<never>(() => {});
const reply = (request: { sourceId: string; revision: string; product: string }, data: unknown) => ({ sourceId: request.sourceId, revision: request.revision, product: request.product, data });
const client = {
  available: true,
  product: async (request: { sourceId: string; revision: string; product: string }) => {
    if (request.product === 'follow') return reply(request, { available: false, reason: 'not followed here' });
    if (request.product === 'follow-clock' || request.product === 'unfollow') return reply(request, null);
    if (request.product in state.products) return reply(request, state.products[request.product]);
    return pending;
  },
  setMapViews: async () => {}, onMapStates: async () => () => {}, releaseSource: async () => {}, openSource: async () => { throw new Error('none'); },
};
const clock = new PlaybackController();
clock.setBounds({ firstHour: 0, lastHour: 200 });
const runtime = { campaignTracks: [track], defaultRoleId: 'primary', tracks: [track], clock, maps: new MapViewsStore(client as never, clock), onMapBranch() {}, client,
  alignment: 'calendar', nativeHidden: false, campaigns: [], recordings: [], assignCampaign: async () => {}, requestSeek: () => false, addWidgetBelow() {} } as unknown as WidgetRuntime;

const turn = async () => {
  await new Promise<void>(resolve => setTimeout(resolve, 0));
  await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
};
const settle = async () => { for (let round = 0; round < 12; round++) await turn(); };
let root: Root | null = null;
const host = () => document.getElementById('finding');
const read = () => {
  const element = host();
  const drill = element?.querySelector('.drill-host[data-open] > .drill-view, .drill-popout:has(.drill-body)');
  return { text: element?.textContent ?? '', drillOpen: !!drill, drill: drill?.textContent ?? '', asks: state.asks.splice(0), options: state.options.splice(0),
    buttons: [...element?.querySelectorAll('button') ?? []].map(button => button.getAttribute('aria-label') ?? button.textContent ?? ''), errors: errors.splice(0) };
};

export interface Scene { kind: string; time: WidgetSpec['time']; options?: Record<string, unknown>; countries?: WidgetSpec['countries']; hour: number;
  replies?: Record<string, string>; products?: Record<string, unknown>; groups?: unknown; trade?: unknown; raids?: unknown }

const replyOf = (body: string): Reply => new Function('hour', 'params', `return (${body});`) as Reply;

Object.assign(window, { findings: {
  subjectToken: (handle: string) => subjectToken('campaign', handle),
  identityToken: (originOffset: number) => datedCountryToken({ campaign: 'campaign', branch: '1', tag: 'GER', originOffset }),
  async preload(kinds: string[]) { await Promise.all(kinds.map(kind => loadWidgetRenderer(kind))); },
  async mount(scene: Scene) {
    root?.unmount();
    document.body.replaceChildren();
    answered.clear();
    Object.assign(state, { asks: [], options: [], props: {}, replies: Object.fromEntries(Object.entries(scene.replies ?? {}).map(([name, body]) => [name, replyOf(body)])),
      products: scene.products ?? {}, groups: scene.groups ?? null, trade: scene.trade, raids: scene.raids, drill: null, values: {} });
    clock.seek(scene.hour);
    const made = createWidget(scene.kind, 'main', 'finding');
    const widget = { ...made, time: scene.time, countries: scene.countries ?? made.countries, options: { ...made.options, ...scene.options } } as WidgetSpec;
    const element = document.body.appendChild(document.createElement('div'));
    element.id = 'finding';
    element.style.cssText = 'width: 900px; height: 600px; position: relative';
    root = createRoot(element);
    root.render(<WidgetBody widget={widget} runtime={runtime} setOption={(key, value) => { state.options.push([key, value]); }} />);
    await settle();
    return read();
  },
  async navy(time: { hour: number; pin: number }) {
    root?.unmount();
    document.body.replaceChildren();
    Object.assign(state, { asks: [], replies: { 'naval-management': replyOf('({ sections: [], warnings: [] })') } });
    clock.seek(time.hour);
    const made = createWidget('navy-size', 'main', 'finding');
    const widget = { ...made, time: { mode: 'pin', windows: { main: { firstOffset: time.pin - FIRST, lastOffset: time.pin - FIRST } } } } as WidgetSpec;
    const element = document.body.appendChild(document.createElement('div'));
    element.id = 'finding';
    root = createRoot(element);
    root.render(<NavyManagement runtime={runtime} widget={widget} track={track} tag="GER" hour={time.pin} following active />);
    await settle();
    element.querySelector('summary')?.click();
    await settle();
    return read();
  },
  async inject(set: { drill?: unknown; values?: Record<string, unknown> }) {
    if ('drill' in set) state.drill = set.drill;
    Object.assign(state.values, set.values ?? {});
    changed(); await settle(); return read();
  },
  async seek(hour: number) { clock.seek(hour); await settle(); return read(); },
  async step(direction: -1 | 1) { clock.step(direction); await settle(); return read(); },
  async dispatch(action: unknown, reduce: 'drill') {
    if (reduce === 'drill') state.drill = action;
    changed(); await settle(); return read();
  },
  async press(label: string) {
    const button = [...host()?.querySelectorAll('button') ?? []].find(item => (item.getAttribute('aria-label') ?? item.textContent ?? '').includes(label));
    button?.click(); await settle(); return { pressed: !!button, ...read() };
  },
  props: (name: string) => {
    const props = state.props[name];
    return props ? JSON.parse(JSON.stringify(props, (_key, value) => typeof value === 'function' ? undefined : value)) : null;
  },
  call: (name: string, method: string, ...args: unknown[]) => {
    const value = (state.props[name]?.[method] as ((...values: unknown[]) => unknown) | undefined)?.(...args);
    return JSON.parse(JSON.stringify(value ?? null));
  },
} });
