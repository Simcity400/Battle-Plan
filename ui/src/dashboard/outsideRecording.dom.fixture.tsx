import { createRoot } from 'react-dom/client';
import WidgetState from '../components/WidgetState';
import type { EngineFrame, FrameEngine } from '../engine/frame';
import { PHASES } from '../engine/frame';
import { registerWidgets } from './definitions';
import type { WidgetProps, WidgetRuntime } from './definitions';
import { useHeadProduct } from './headProduct';
import { useGate } from './regionGate';
import type { WidgetSpec } from './model';
import { PlaybackController } from './playback';
import { WidgetBody } from './widgets';
import { definition as warGoals } from '../widgets/war-goals/definition';

const WIDGETS = 114;
let now = 1000;
performance.now = () => now;
const tasks = new Map<string, (frame: EngineFrame) => boolean | void>();
const engine = { every: (phase: string, run: (frame: EngineFrame) => boolean | void) => {
  tasks.set(phase, run);
  return { wake() {}, stop() { tasks.delete(phase); } };
} } as unknown as FrameEngine;
const clock = new PlaybackController(engine);
clock.setBounds({ firstHour: 0, lastHour: 500 });

const renders = { count: 0 };
function Probe({ widget, runtime, active }: WidgetProps) {
  renders.count++;
  const head = useHeadProduct(runtime, useGate(runtime, widget), 'probe-moment', {}, { active, validate: value => value });
  return head.data !== undefined ? <p>read</p> : <WidgetState kind="loading" label="Loading the probeâ€¦" />;
}
registerWidgets([{ ...warGoals, playsBackDated: true, render: Probe }]);

const waiting = new Promise<never>(() => {});
const client = {
  available: true,
  product: async (request: { sourceId: string; revision: string; product: string; params: Record<string, unknown> }) => {
    if (request.product === 'follow') return { sourceId: request.sourceId, revision: request.revision, product: request.product, data: { available: false, reason: 'not followed here' } };
    if (request.product === 'follow-clock' || request.product === 'unfollow') return { sourceId: request.sourceId, revision: request.revision, product: request.product, data: null };
    return waiting;
  },
  setMapViews: async () => {}, onMapStates: async () => () => {}, releaseSource: async () => {}, openSource: async () => { throw new Error('none'); },
};
const source = { id: 'outside', revision: '1', metadata: { countries: [], captureMode: 'full' } };
const track = (id: string, firstHour: number, lastHour: number) => ({ id, roleId: id, label: id, branchIndex: 0, anchorOffset: null, axisOffset: 0, selectedTag: 'GER', source,
  branch: { id, firstHour, lastHour, gaps: [] } });
const tracks = [track('early', 0, 100), track('late', 101, 500)];
const runtime = { tracks, campaignTracks: [], defaultRoleId: null, clock, client, addWidgetBelow() {}, campaigns: [], recordings: [], requestSeek() {} } as unknown as WidgetRuntime;
const widgets = Array.from({ length: WIDGETS }, (_, index) => ({ id: `probe-${index}`, kind: 'war-goals', configVersion: 1, title: 'Probe',
  trackIds: [index % 2 ? 'late' : 'early'], countries: { kind: 'tags', tags: ['GER'] }, time: { mode: 'follow' }, options: {} }) as unknown as WidgetSpec);

const host = document.body.appendChild(document.createElement('div'));
createRoot(host).render(<>{widgets.map(widget => <div key={widget.id} id={widget.id}><WidgetBody widget={widget} runtime={runtime} /></div>)}</>);

const settle = () => new Promise<void>(resolve => setTimeout(resolve, 0));
const stateOf = (id: string) => {
  const state = document.getElementById(id)?.querySelector<HTMLElement>('.widget-state');
  return state ? `${state.dataset.state}${state.textContent ? `: ${state.textContent}` : ''}` : null;
};
const shown = () => {
  const by = { early: new Set<string | null>(), late: new Set<string | null>() };
  widgets.forEach((widget, index) => by[index % 2 ? 'late' : 'early'].add(stateOf(widget.id)));
  return { hour: clock.getHour(), early: [...by.early], late: [...by.late], renders: renders.count };
};
const tick = (ms: number) => {
  now += ms;
  for (const phase of PHASES) tasks.get(phase)?.({ now, dt: ms, index: 0, phase } as EngineFrame);
};

async function frames(count: number, ms: number) {
  const seen = [];
  for (let frame = 0; frame < count; frame++) {
    const before = renders.count;
    tick(ms);
    await settle();
    seen.push({ ...shown(), renders: renders.count - before });
  }
  return seen;
}

Object.assign(window, { outsideFixture: {
  async start(hour: number) { clock.seek(hour); await settle(); await settle(); return shown(); },
  async play(speed: number, count: number, ms: number) {
    clock.setSpeed(speed);
    clock.setPlaying(true);
    await settle();
    const seen = await frames(count, ms);
    clock.setPlaying(false);
    await settle();
    return seen;
  },
} });
