import { Profiler } from 'react';
import { createRoot } from 'react-dom/client';
import '../styles.css';
import '../widgets';
import { isPlaybackDatedView } from '../widgets/manifest';
import { loadWidgetRenderer } from '../widgets/rendererLoaders';
import type { EngineFrame, FrameEngine } from '../engine/frame';
import { PHASES } from '../engine/frame';
import { widgetDefinitions } from './definitions';
import type { WidgetRuntime } from './definitions';
import { createWidget } from './model';
import type { BoundTrack, WidgetSpec } from './model';
import { PlaybackController } from './playback';
import { MapViewsStore } from './geometry';
import { WidgetBody } from './widgets';

class OnScreen {
  private readonly heard: IntersectionObserverCallback;
  constructor(heard: IntersectionObserverCallback) { this.heard = heard; }
  observe(target: Element) { queueMicrotask(() => this.heard([{ target, isIntersecting: true, intersectionRatio: 1 } as unknown as IntersectionObserverEntry], this as never)); }
  unobserve() {}
  disconnect() {}
  takeRecords() { return []; }
}
Object.assign(window, { IntersectionObserver: OnScreen });

let now = 1000;
performance.now = () => now;
const tasks = new Map<string, (frame: EngineFrame) => boolean | void>();
const engine = { every: (phase: string, run: (frame: EngineFrame) => boolean | void) => {
  tasks.set(phase, run);
  return { wake() {}, stop() { tasks.delete(phase); } };
} } as unknown as FrameEngine;
const clock = new PlaybackController(engine);
clock.setBounds({ firstHour: 0, lastHour: 500 });

const waiting = new Promise<never>(() => {});
const reply = (request: { sourceId: string; revision: string; product: string }, data: unknown) => ({ sourceId: request.sourceId, revision: request.revision, product: request.product, data });
const client = {
  available: true,
  product: async (request: { sourceId: string; revision: string; product: string; params: Record<string, unknown> }) => {
    if (request.product === 'follow') return reply(request, { available: false, reason: 'not followed here' });
    if (request.product === 'follow-clock' || request.product === 'unfollow') return reply(request, null);
    return waiting;
  },
  setMapViews: async () => {}, onMapStates: async () => () => {}, releaseSource: async () => {}, openSource: async () => { throw new Error('none'); },
};
const branch = (id: string, firstHour: number, lastHour: number) => ({ id, sessionId: id, firstHour, lastHour, main: id === 'early', fork: null, joins: [], sampleBoundary: '', gaps: [] });
const branches = [branch('early', 0, 100), branch('late', 101, 500)];
const source = { id: 'outside', path: 'outside.battleplan', revision: '1', metadata: { path: 'outside.battleplan', name: 'Outside', format: 1, captureMode: 'full',
  campaignKey: 'campaign', branches, countries: [{ id: '1', tag: 'GER', name: 'Germany', observedBranchId: 'early', observedHour: 0 }], playerCountryId: '1',
  contentVerified: true, partial: null, warnings: [], buildWarnings: [], revision: '1' } };
const tracks = branches.map((held, index) => ({ id: held.id, roleId: held.id, label: held.id, branchIndex: index, anchorOffset: null, axisOffset: 0, selectedTag: 'GER',
  selectedSlot: 1, source, branch: held })) as unknown as BoundTrack[];
const runtime = { tracks, campaignTracks: [], defaultRoleId: null, clock, client, maps: new MapViewsStore(client as never, clock), onMapBranch() {}, alignment: 'anchors',
  nativeHidden: false, addWidgetBelow() {}, campaigns: [], recordings: [], assignCampaign: async () => {}, requestSeek: () => false } as unknown as WidgetRuntime;

export interface Mounted { id: string; kind: string; track: 'early' | 'late'; follows: boolean; dated: boolean }
let mounted: Mounted[] = [];
const renders = new Map<string, number>();
const counted = (id: string) => () => { renders.set(id, (renders.get(id) ?? 0) + 1); };

const settle = () => new Promise<void>(resolve => setTimeout(resolve, 0));
const renderTotal = () => [...renders.values()].reduce((sum, count) => sum + count, 0);
const quiet = async () => {
  for (let same = 0, round = 0; round < 40 && same < 3; round++) {
    const before = renderTotal();
    await settle();
    same = renderTotal() === before ? same + 1 : 0;
  }
};
const stateOf = (id: string) => {
  const element = document.getElementById(id);
  const state = element?.querySelector<HTMLElement>('.widget-state, .dash-empty');
  return state ? `${state.dataset.state ?? 'empty'}: ${state.textContent ?? ''}` : (element?.textContent ?? '').slice(0, 120);
};
const shown = (before: ReadonlyMap<string, number>) => ({ hour: clock.getHour(),
  widgets: Object.fromEntries(mounted.map(item => [item.id, { state: stateOf(item.id), renders: (renders.get(item.id) ?? 0) - (before.get(item.id) ?? 0) }])) });
const tick = (ms: number) => {
  now += ms;
  for (const phase of PHASES) tasks.get(phase)?.({ now, dt: ms, index: 0, phase } as EngineFrame);
};

async function frames(count: number, ms: number) {
  const seen = [];
  for (let frame = 0; frame < count; frame++) {
    const before = new Map(renders);
    tick(ms);
    await quiet();
    seen.push(shown(before));
  }
  return seen;
}

Object.assign(window, { outsideFixture: {
  async start(hour: number) {
    const definitions = widgetDefinitions();
    await Promise.all(definitions.map(definition => loadWidgetRenderer(definition.kind)));
    clock.seek(hour);
    const built = definitions.map((definition, index) => {
      const track: 'early' | 'late' = index % 2 ? 'late' : 'early';
      const id = `widget-${definition.kind}`;
      const follows = definition.timeModes.includes('follow');
      const made = createWidget(definition.kind, track, id);
      const spec = { ...made, campaign: 'tracks', trackIds: [track], time: follows ? { mode: 'follow' } : made.time } as WidgetSpec;
      const item: Mounted = { id, kind: definition.kind, track, follows, dated: follows && isPlaybackDatedView(definition.kind, spec.options, definition) };
      return { item, spec };
    });
    mounted = built.map(entry => entry.item);
    const specs = built.map(entry => entry.spec);
    createRoot(document.body.appendChild(document.createElement('div'))).render(<>{specs.map(widget =>
      <section key={widget.id} id={widget.id} style={{ width: 480, height: 320, overflow: 'hidden', position: 'relative' }}>
        <Profiler id={widget.id} onRender={counted(widget.id)}><WidgetBody widget={widget} runtime={runtime} /></Profiler>
      </section>)}</>);
    for (let turn = 0; turn < 20; turn++) await settle();
    await quiet();
    return { mounted, ...shown(new Map(renders)) };
  },
  async play(speed: number, count: number, ms: number) {
    clock.setSpeed(speed);
    clock.setPlaying(true);
    await quiet();
    const seen = await frames(count, ms);
    clock.setPlaying(false);
    await quiet();
    return seen;
  },
} });
