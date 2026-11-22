import { createElement, useId, useLayoutEffect } from 'react';
import { createRoot } from 'react-dom/client';
import type { Root } from 'react-dom/client';
import '../widgets';
import { loadWidgetRenderer } from '../widgets/rendererLoaders';
import type { WidgetRuntime } from './definitions';
import { createWidget } from './model';
import type { BoundTrack, WidgetSpec } from './model';
import { PlaybackController } from './playback';
import { MapViewsStore } from './geometry';
import { WidgetBody } from './widgets';
import { datedToken } from '../test-support/gated.ts';
import { subjectToken } from './chosen';
import { dayLabel } from '../charts/draw';

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
const hoursIn = (params: Record<string, unknown> | null): number[] => !params ? [] : Object.entries(params).flatMap(([key, value]) =>
  key === 'reads' && Array.isArray(value) ? value.flatMap(read => typeof read === 'number' ? [read] : typeof read?.hour === 'number' ? [read.hour] : [])
    : (key === 'hour' || key.endsWith('Hour') || key === 'from' || key === 'to') && typeof value === 'number' ? [value] : []);
const follows = new Map<string, string | null>();
const state = {
  asks: [] as Ask[], options: [] as [string, unknown][], pins: [] as unknown[], replies: {} as Record<string, Reply>, failures: new Set<string>(),
  groups: null as unknown, trade: undefined as unknown, raids: undefined as unknown, props: {} as Record<string, Record<string, unknown>>,
};
const capture = (name: string) => (props: Record<string, unknown>) => { state.props[name] = props; return createElement('p', { 'data-stub': name }, String(props.label ?? props.title ?? name)); };
Object.assign(globalThis, { __finding: {
  capture,
  useFollow: (_store: unknown, query: unknown) => {
    const id = useId();
    useLayoutEffect(() => { follows.set(id, query ? JSON.stringify(query) : null); });
    useLayoutEffect(() => () => { follows.delete(id); }, [id]);
    return undefined;
  },
  useRaidLog: (_widget: unknown, runtime: WidgetRuntime) => state.raids === undefined ? undefined
    : state.raids === 'failed' ? { track: runtime.tracks[0], data: undefined, error: new Error('raids failed'), retry() {} }
    : { track: runtime.tracks[0], data: state.raids, error: undefined, retry() {} },
  useTradeFlows: (_widget: unknown, runtime: WidgetRuntime) => state.trade === undefined ? undefined
    : { track: runtime.tracks[0], all: true, resolved: null, groups: { data: state.groups ?? undefined, complete: true, error: undefined, retry() {} }, scope: null,
      data: state.trade, error: undefined, retry() {}, extent: [100, 109] },
} });

const pending = new Promise<never>(() => {});
const answer = (request: { sourceId: string; revision: string; product: string }, data: unknown) => ({ sourceId: request.sourceId, revision: request.revision, product: request.product, data });
const client = {
  available: true,
  product: async (request: { sourceId: string; revision: string; product: string; params?: Record<string, unknown> }) => {
    if (request.product === 'follow') return answer(request, { available: false, reason: 'not followed here' });
    if (request.product === 'follow-clock' || request.product === 'unfollow') return answer(request, null);
    const params = request.params ?? null;
    const hour = typeof params?.hour === 'number' ? params.hour : null;
    state.asks.push({ product: request.product, hour, hours: hoursIn(params), params });
    if (state.failures.has(request.product)) throw new Error(`${request.product} failed`);
    if (request.product === 'country-groups') return state.groups ? answer(request, state.groups) : pending;
    const reply = state.replies[request.product];
    return reply ? answer(request, reply(hour, params ?? {})) : pending;
  },
  setMapViews: async () => {}, onMapStates: async () => () => {}, releaseSource: async () => {}, openSource: async () => { throw new Error('none'); },
};
const clock = new PlaybackController();
clock.setBounds({ firstHour: 0, lastHour: 200 });
const sourceOf = (gaps: { firstHour: number; lastHour: number; reason: string }[]) => ({ id: 'findings', path: 'findings.battleplan', revision: '1',
  metadata: { path: 'findings.battleplan', name: 'Findings', format: 1, captureMode: 'full', campaignKey: 'campaign',
    branches: [{ id: '1', sessionId: 's', firstHour: FIRST, lastHour: LAST, main: true, fork: null, joins: [], sampleBoundary: '', gaps }],
    countries: [{ id: '1', tag: 'GER', name: 'Germany', observedBranchId: '1', observedHour: FIRST }, { id: '2', tag: 'ENG', name: 'England', observedBranchId: '1', observedHour: FIRST }],
    playerCountryId: '1', contentVerified: true, partial: null, warnings: [], buildWarnings: [], revision: '1' } });
const runtimeOf = (gaps: { firstHour: number; lastHour: number; reason: string }[]) => {
  const source = sourceOf(gaps);
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
const settle = async () => { for (let round = 0; round < 16; round++) await turn(); };
let root: Root | null = null;
const host = () => document.getElementById('finding');
const read = () => {
  const element = host();
  const drill = element?.querySelector('.drill-host[data-open] > .drill-view, .drill-popout:has(.drill-body)');
  return { text: element?.textContent ?? '', drillOpen: !!drill, drill: drill?.textContent ?? '', day: drill?.querySelector('.drill-day')?.textContent ?? null,
    asks: state.asks.splice(0), options: state.options.splice(0), pins: state.pins.splice(0),
    follows: [...follows.values()].filter((query): query is string => query !== null),
    buttons: [...element?.querySelectorAll('button') ?? []].map(button => button.getAttribute('aria-label') ?? button.textContent ?? ''), errors: errors.splice(0) };
};

export interface Scene { kind: string; time: WidgetSpec['time']; options?: Record<string, unknown>; countries?: WidgetSpec['countries']; hour: number;
  replies?: Record<string, string>; failures?: string[]; groups?: unknown; trade?: unknown; raids?: unknown; gaps?: { firstHour: number; lastHour: number; reason: string }[] }

const replyOf = (body: string): Reply => new Function('hour', 'params', `return (${body});`) as Reply;

Object.assign(window, { findings: {
  subjectToken: (handle: string) => subjectToken('campaign', handle),
  identityToken: (originOffset: number) => datedToken({ campaign: 'campaign', branch: '1', tag: 'GER', originOffset }),
  day: (hour: number) => dayLabel(hour),
  async preload(kinds: string[]) { await Promise.all(kinds.map(kind => loadWidgetRenderer(kind))); },
  async mount(scene: Scene) {
    root?.unmount();
    document.body.replaceChildren();
    Object.assign(state, { asks: [], options: [], pins: [], props: {}, replies: Object.fromEntries(Object.entries(scene.replies ?? {}).map(([name, body]) => [name, replyOf(body)])),
      failures: new Set(scene.failures ?? []), groups: scene.groups ?? null, trade: scene.trade, raids: scene.raids });
    clock.setPlaying(false);
    clock.seek(scene.hour);
    const made = createWidget(scene.kind, 'main', 'finding');
    const widget = { ...made, time: scene.time, countries: scene.countries ?? made.countries, options: { ...made.options, ...scene.options } } as WidgetSpec;
    const element = document.body.appendChild(document.createElement('div'));
    element.id = 'finding';
    element.style.cssText = 'width: 900px; height: 600px; position: relative';
    root = createRoot(element);
    root.render(<WidgetBody widget={widget} runtime={runtimeOf(scene.gaps ?? [])} setOption={(key, value) => { state.options.push([key, value]); }} />);
    await settle();
    return read();
  },
  async seek(hour: number) { clock.seek(hour); await settle(); return read(); },
  async play(speed: number, ms: number) {
    clock.setSpeed(speed); clock.setPlaying(true);
    await new Promise<void>(resolve => setTimeout(resolve, ms));
    const playing = read();
    clock.setPlaying(false); await settle();
    return { playing, paused: read() };
  },
  async press(label: string) {
    const button = [...host()?.querySelectorAll('button') ?? []].find(item => (item.getAttribute('aria-label') ?? item.textContent ?? '').includes(label));
    button?.click(); await settle(); return { pressed: !!button, ...read() };
  },
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
