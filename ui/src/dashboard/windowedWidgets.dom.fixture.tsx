import { Component, Profiler } from 'react';
import type { ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import type { Root } from 'react-dom/client';
import EquipmentProducedWidget from '../widgets/equipment-produced/EquipmentProducedWidget';
import NavalSupremacyWidget from '../widgets/naval-supremacy/NavalSupremacyWidget';
import ProductionSpendingWidget from '../widgets/production-spending/ProductionSpendingWidget';
import AirKillsLossesWidget from '../widgets/air-kills-losses/AirKillsLossesWidget';
import { MeasuredWidget } from './widgetPerf.ts';
import { WINDOW_HOURS } from './heldWindows.ts';
import type { WidgetProps } from './definitions';
import '../styles.css';

const native = window.requestAnimationFrame.bind(window);
const frame = () => new Promise<void>(resolve => native(() => resolve()));
async function settle(frames = 8) { for (let at = 0; at < frames; at++) await frame(); }

const HOURS = 40 * WINDOW_HOURS;
const HEAD = 30_000;
const TAGS = ['GER', 'ITA'];
const branch = { id: '1', firstHour: 0, lastHour: HOURS - 1, gaps: [] };
const track = { id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [], branches: [branch], campaignKey: 'campaign' } }, branch };

class Clock {
  position = HEAD;
  playing = false;
  hoursPerSecond = 1440;
  private readonly frames = new Set<() => void>();
  private readonly anchors = new Set<() => void>();
  getSnapshot = () => ({ position: this.position, hour: Math.floor(this.position), fraction: this.position % 1, playing: this.playing,
    hoursPerSecond: this.hoursPerSecond, sequence: 0, seekSequence: 0, bounds: { firstHour: 0, lastHour: HOURS - 1 } });
  getHour = () => Math.floor(this.position);
  getMotion = () => this.playing ? this.hoursPerSecond : undefined;
  positionAt = () => this.position;
  subscribe = (listener: () => void) => { this.anchors.add(listener); return () => { this.anchors.delete(listener); }; };
  subscribeFrame = (listener: () => void) => { this.frames.add(listener); return () => { this.frames.delete(listener); }; };
  subscribeHour = () => () => {};
  step(hours: number) { this.position += hours; for (const listener of [...this.frames]) listener(); }
  anchor() { for (const listener of [...this.anchors]) listener(); }
}

const identities = (tag: string) => [{ firstHour: 0, lastHour: HOURS - 1, slot: TAGS.indexOf(tag) + 1, countryId: TAGS.indexOf(tag) + 1 }];
const hoursOf = (index: number) => [index * WINDOW_HOURS, Math.min(HOURS, (index + 1) * WINDOW_HOURS) - 1] as const;
const days = (index: number) => { const [first, last] = hoursOf(index); return Array.from({ length: Math.ceil((last - first + 1) / 24) }, (_, day) => first + day * 24); };

function produced(params: Record<string, unknown>) {
  const index = params.window as number, [firstHour, lastHour] = hoursOf(index);
  const lines = (params.tags as string[]).map((tag, at) => ({ tag, identities: identities(tag), series: { firstHours: days(index), lastHours: days(index).map(day => day + 23),
    values: days(index).map(day => Math.floor(day / 24) * (3 - at)), unknown: [], breaks: [] } }));
  return { branchId: '1', count: params.count, measure: params.measure, decimals: 3, types: params.types, rankHour: null, window: index, firstHour, lastHour,
    lines, reads: days(index).map(day => day + 23), observations: [0], warnings: [] };
}

function supremacy(params: Record<string, unknown>) {
  const index = params.window as number, [firstHour, lastHour] = hoursOf(index), count = days(index).length;
  return { window: index, firstHour, lastHour, firstDayHour: firstHour, dayHours: 24, days: count, unknown: [], scale: 100_000, warnings: [],
    regions: [{ id: '5', name: 'North Sea', runs: [{ firstDay: 0, lastDay: count - 1, control: 1, friendly: '300000', enemy: '0', share: '100000', own: '300000' }] }] };
}

function spending(params: Record<string, unknown>) {
  const index = params.window as number, [firstHour, lastHour] = hoursOf(index);
  const runs = days(index).map(day => ({ firstHour: day, lastHour: day + 23, totalRaw: String(Math.floor(day / 24) * 1000), reason: null }));
  const reads = days(index).map(day => day + 23);
  return { branchId: '1', tag: params.tag, decimals: 5, reads, observations: [0], warnings: [], equipmentReads: reads, rocketReads: reads,
    equipmentObservations: [0], rocketObservations: [0], rocketKey: null, rocketName: null, rocketTotal: [{ firstHour, lastHour, totalRaw: '0', reason: null }],
    production: [{ firstHour, lastHour, present: true, emptyCensusHour: null }],
    leaves: [{ variant: '1', typeKey: 'infantry_equipment', name: 'Infantry equipment', domain: 'land', rocket: false, purpose: 'new', unproven: [], names: [], runs }],
    total: runs, totalWithoutRefits: runs, equipmentTotal: runs, equipmentTotalWithoutRefits: runs, repairs: null, window: index, firstHour, lastHour };
}

function air(params: Record<string, unknown>) {
  const index = params.window as number, [firstHour, lastHour] = hoursOf(index);
  const identity = (tag: string) => `${TAGS.indexOf(tag) + 1}:${TAGS.indexOf(tag) + 1}`;
  const sides = [['GER', false, 300_000], ['GER', true, 100_000], ['ITA', false, 100_000], ['ITA', true, 200_000]] as const;
  const before = Array.from({ length: index }, (_, at) => days(at).length).reduce((sum, count) => sum + count, 0);
  return { family: params.family, window: index, firstHour, lastHour, countedFrom: 0, available: true, observed: [{ firstHour, lastHour }], incomplete: [],
    countries: TAGS.map(tag => ({ tag, identity: identity(tag), present: [{ firstHour, lastHour }], membership: null })),
    openings: before ? sides.map(([tag, dealt, step]) => ({ identity: identity(tag), dealt, cause: 1, opponentKnown: true, total: String(before * step) })) : [],
    parts: sides.map(([tag, dealt, step]) => ({ tag, identity: identity(tag), dealt, cause: 1, opponentKnown: true, buildingToken: null, buildingId: null, state: null,
      points: days(index).map((day, at) => [day, day, String((at + 1) * step)]) })),
    regions: [], buildings: [], notes: [], warnings: [] };
}

function reply(product: string, params: Record<string, unknown>): unknown {
  switch (product) {
    case 'equipment-produced': return produced(params);
    case 'equipment-produced-types': return { branchId: '1', types: [{ key: 'infantry_equipment', name: 'Infantry equipment' }], mostDelivered: 'infantry_equipment', warnings: [] };
    case 'naval-supremacy': return supremacy(params);
    case 'production-spending': return spending(params);
    case 'military-air-ledger': return air(params);
    case 'country-styles': return { branchId: '1', hour: params.hour, styles: [], warnings: [] };
    case 'country-facts': return { branchId: '1', known: [[0, HOURS - 1]], names: [], reasons: [], flags: [], countries: [], warnings: [] };
    case 'follow': return { available: false, reason: 'not followed here' };
    case 'unfollow': case 'follow-clock': return {};
    default: throw new Error(`${product} is not served here`);
  }
}

let clock = new Clock();
const requests: { product: string; window: number | null; head: number; playing: boolean }[] = [];
const renders: Record<string, number> = {};
const errors: Record<string, string> = {};
let root: Root | null = null;
let host: HTMLDivElement | null = null;

const client = {
  available: true,
  product: async (request: { sourceId: string; revision: string; product: string; params: Record<string, unknown> }) => {
    requests.push({ product: request.product, window: typeof request.params.window === 'number' ? request.params.window : null, head: clock.getHour(), playing: clock.playing });
    return { sourceId: request.sourceId, revision: request.revision, product: request.product, data: reply(request.product, request.params) };
  },
  setMapViews: async () => {}, onMapStates: async () => () => {}, releaseSource: async () => {}, openSource: async () => { throw new Error('none'); },
};

class Boundary extends Component<{ id: string; children: ReactNode }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  componentDidCatch(error: Error) { errors[this.props.id] = `${error.message} ${error.stack ?? ''}`.slice(0, 600); }
  render() { return this.state.failed ? null : this.props.children; }
}

function Counted({ id, children }: { id: string; children: ReactNode }) {
  return <MeasuredWidget value={id}><Boundary id={id}>
    <Profiler id={id} onRender={() => { renders[id] = (renders[id] ?? 0) + 1; }}>{children}</Profiler>
  </Boundary></MeasuredWidget>;
}

export const PRODUCTS = { produced: 'equipment-produced', supremacy: 'naval-supremacy', spending: 'production-spending', air: 'military-air-ledger' } as const;

export const fixture = {
  async mount() {
    root?.unmount(); host?.remove();
    clock = new Clock(); requests.length = 0;
    for (const key of Object.keys(renders)) delete renders[key];
    for (const key of Object.keys(errors)) delete errors[key];
    host = document.body.appendChild(document.createElement('div'));
    host.style.cssText = 'width: 1200px; display: flex; flex-direction: column;';
    root = createRoot(host);
    const runtime = { tracks: [track], clock, client, addWidgetBelow() {}, campaigns: [], recordings: [] };
    const widget = (id: string, kind: string, countries: unknown, options: Record<string, unknown> = {}) =>
      ({ widget: { id, kind, trackIds: ['main'], countries, time: { mode: 'follow' }, options }, runtime, active: true, onDiagnostics() {}, setOption() {} }) as unknown as WidgetProps;
    const both = { kind: 'tags', tags: TAGS }, one = { kind: 'tags', tags: ['GER'] };
    const box = (id: string, children: ReactNode) => <div data-widget={id} style={{ height: 240, display: 'flex', flexDirection: 'column' }}><Counted id={id}>{children}</Counted></div>;
    root.render(<>
      {box('produced', <EquipmentProducedWidget {...widget('produced', 'equipment-produced', both, { count: 'ic' })} />)}
      {box('supremacy', <NavalSupremacyWidget {...widget('supremacy', 'naval-supremacy', one)} />)}
      {box('spending', <ProductionSpendingWidget {...widget('spending', 'production-spending', one)} />)}
      {box('air', <AirKillsLossesWidget {...widget('air', 'air-kills-losses', both)} />)}
    </>);
    await settle(60);
  },
  async play(frames: number, hours: number) {
    clock.playing = true; clock.anchor();
    await settle(30);
    const before = { ...renders };
    const from = clock.getHour();
    for (let at = 0; at < frames; at++) { clock.step(hours); await frame(); }
    await settle();
    return { from, head: clock.getHour(), rendered: Object.fromEntries(Object.keys(PRODUCTS).map(id => [id, (renders[id] ?? 0) - (before[id] ?? 0)])) };
  },
  pause() { clock.playing = false; clock.anchor(); },
  opened: () => requests.filter(request => !request.playing),
  played: () => requests.filter(request => request.playing),
  all: () => requests.map(request => `${request.product}${request.window === null ? '' : `#${request.window}`}@${request.head}`),
  errors: () => ({ ...errors }),
  states: () => Object.fromEntries([...host!.querySelectorAll<HTMLElement>('.widget-state')].map(state => [state.closest<HTMLElement>('[data-widget]')?.dataset.widget ?? '',
    `${state.textContent ?? ''} ${state.dataset.detail ?? ''}`])),
};
(window as unknown as { windowedWidgets: typeof fixture }).windowedWidgets = fixture;
