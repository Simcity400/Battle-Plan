import { datedPanel } from '../../dashboard/follow/messages.fixture.ts';
import { Profiler } from 'react';
import type { ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import type { Root } from 'react-dom/client';
import SpecialProjectsWidget from './SpecialProjectsWidget';
import { MeasuredWidget } from '../../dashboard/widgetPerf.ts';
import { measureWith } from '../../shell/perf.ts';
import type { PerfRecorder } from '../../shell/perf.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import type { SpecialProjectDetails, SpecialProjectRun, SpecialProjectsReply } from '../../generated/dashboard';

const native = window.requestAnimationFrame.bind(window);
const frame = () => new Promise<void>(resolve => native(() => resolve()));
async function settle(frames = 4) { for (let at = 0; at < frames; at++) await frame(); }

const shown = new Map<string, (number | null)[]>();
const recordHour = (widget: string, hour: number | null) => { shown.set(widget, [...shown.get(widget) ?? [], hour]); };
measureWith(new Proxy({
  shown: recordHour,
}, {
  get: (target, name) => (target as Record<string | symbol, unknown>)[name] ?? (() => {}),
}) as unknown as PerfRecorder);

const HOURS = 120_000;
const START = 30_000;
export const LATE = 34_000;
const track = { id: 'main', axisOffset: 0, selectedTag: 'GER', source: { id: 'source', revision: '1', metadata: { countries: [] } },
  branch: { id: '1', firstHour: 0, lastHour: HOURS - 1, gaps: [] } };
const identities = [{ firstHour: 0, lastHour: HOURS - 1, slot: 1, countryId: 1 }];
const GROUPS = { branchId: '1', humans: [], factions: [], wars: [], warStarts: [], identityChanges: [], warnings: [], capitulations: [],
  countryIdentities: [{ tag: 'GER', identities }], unobserved: { humans: [], factions: [], wars: [] } };

const daily = (from: number, start: number, perDay: number): SpecialProjectRun[] => {
  const runs: SpecialProjectRun[] = [];
  for (let day = 0; from + day * 24 < HOURS; day++) {
    const progress = Math.min(100, start + Math.floor(day * perDay));
    runs.push({ firstHour: from + day * 24, lastHour: Math.min(HOURS - 1, from + day * 24 + 23), presentation: 0, progress, state: 'running',
      completion: null, cancellation: null, iterations: Math.floor(progress / 25) });
  }
  return runs;
};
const merge = (runs: SpecialProjectRun[]) => runs.reduce<SpecialProjectRun[]>((out, run) => {
  const last = out.at(-1);
  if (last && last.progress === run.progress && last.iterations === run.iterations) last.lastHour = run.lastHour; else out.push({ ...run });
  return out;
}, []);
export const reply: SpecialProjectsReply = { branchId: '1', tag: 'GER', identities, available: [{ firstHour: 0, lastHour: HOURS - 1 }],
  presentations: [{ name: 'Rocket engines', specialization: null, specializationName: null, icon: null, iconMissing: 'none', scientist: 0, scientistAsOf: null }],
  people: [{ assigned: false, id: null, name: null, portrait: null, missing: 'No scientist assigned.' }],
  iterationTexts: { 0: '0', 1: '1', 2: '2', 3: '3', 4: '4' }, unavailable: null, warnings: [],
  projects: [
    { id: 'fast', start: { hour: 0, exact: false }, runs: merge(daily(0, 10, 1 / 50)) },
    { id: 'slow', start: { hour: 0, exact: false }, runs: merge(daily(0, 0, 1 / 60)) },
    { id: 'late', start: { hour: LATE, exact: false }, runs: merge(daily(LATE, 0, 0)) },
  ] };

export const details = (read: number): SpecialProjectDetails => ({ hour: read, unavailable: [], warnings: [], projects: [],
  scientists: [{ character: 7, person: { assigned: true, id: 7, name: 'Wernher', portrait: null, missing: null }, active: 1,
    injuryDays: Math.max(0, 400 - Math.floor(read / 24) % 400), injuryHour: read, visible: true, visibilityHour: read, stateRefusals: {}, skills: [], traits: [] }],
  breakthroughs: [{ specialization: { token: 'sp_rockets', name: 'Rocketry' }, points: Math.floor(read / 24) * 100, readHour: read }],
  programs: [] });

const stamp = { route: 9, epoch: 1, generation: 1 };
const panels: { id: number; port?: MessagePort } = { id: -1 };
function follow(params: { id: number; query: { panel?: string } }) {
  if (params.query.panel !== 'special-project-details') return { available: false, reason: 'not followed here' };
  panels.id = params.id;
  return { available: true, ...stamp };
}
function push(hour: number) {
  const read = hour - (hour - 4) % 24;
  panels.port?.postMessage({ kind: 'panels', bytes: datedPanel(panels.id, hour, details(read)) });
}

let clock: Clock;
class Clock {
  position = START;
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

function answer(product: string): unknown {
  if (product === 'country-groups') return GROUPS;
  if (product === 'special-projects') return reply;
  if (product === 'special-project-details') return details(START - (START - 4) % 24);
  if (product === 'country-facts') return { branchId: '1', known: [[0, HOURS - 1]], names: [], reasons: [], flags: [], countries: [], warnings: [] };
  if (product === 'country-styles') return { branchId: '1', styles: [], warnings: [] };
  throw new Error(`${product} is not served here`);
}

const requests: string[] = [];
const renders: Record<string, number> = {};
let root: Root | null = null;
let host: HTMLDivElement | null = null;
const client = {
  available: true,
  product: async (request: { sourceId: string; revision: string; product: string; params: Record<string, unknown> }) => {
    requests.push(request.product);
    const data = request.product === 'follow' ? follow(request.params as never) : request.product === 'follow-clock' || request.product === 'unfollow' ? stamp : answer(request.product);
    return { sourceId: request.sourceId, revision: request.revision, product: request.product, data };
  },
  openFollow: async (ports: readonly MessagePort[]) => { panels.port = ports[0]; },
  setMapViews: async () => {}, onMapStates: async () => () => {}, releaseSource: async () => {}, openSource: async () => { throw new Error('none'); },
};

const counted = (id: string, children: ReactNode) => <MeasuredWidget value={id}>
  <Profiler id={id} onRender={() => { renders[id] = (renders[id] ?? 0) + 1; }}>{children}</Profiler>
</MeasuredWidget>;

export const fixture = {
  async mount() {
    root?.unmount(); host?.remove();
    clock = new Clock(); requests.length = 0; shown.clear(); panels.id = -1;
    for (const key of Object.keys(renders)) delete renders[key];
    host = document.body.appendChild(document.createElement('div'));
    host.style.cssText = 'zoom: 0.75; width: 640px; display: flex; flex-direction: column;';
    root = createRoot(host);
    const runtime = { tracks: [track], clock, client, addWidgetBelow() {}, campaigns: [], recordings: [], requestSeek() {} };
    const widget = (id: string, view: string) => ({ id, kind: 'special-projects', trackIds: ['main'], countries: { kind: 'tags', tags: ['GER'] },
      time: { mode: 'follow' }, options: { view } });
    const props = (spec: unknown) => ({ widget: spec, runtime, active: true, onDiagnostics() {} }) as unknown as WidgetProps;
    root.render(<>
      <div id="current" style={{ height: '640px', display: 'flex' }}>{counted('current', <SpecialProjectsWidget {...props(widget('current', 'live'))} />)}</div>
      <div id="history" style={{ height: '320px', display: 'flex' }}>{counted('history', <SpecialProjectsWidget {...props(widget('history', 'history'))} />)}</div>
    </>);
    await settle(30);
    push(clock.getHour());
    await settle(4);
  },
  async play(frames: number, hours: number) {
    clock.playing = true; clock.anchor();
    await settle(30);
    const before = { requests: requests.length, renders: { ...renders } };
    for (let at = 0; at < frames; at++) {
      clock.step(hours);
      push(clock.getHour());
      await frame();
    }
    await settle(30);
    const rendered = Object.fromEntries(Object.entries(renders).map(([id, count]) => [id, count - (before.renders[id] ?? 0)]));
    return { head: clock.getHour(), asked: requests.slice(before.requests), rendered };
  },
  pause() { clock.playing = false; clock.anchor(); },
  cards: () => [...document.querySelectorAll<HTMLElement>('#current [data-project]')].map(card => ({ id: card.dataset.project,
    progress: card.querySelector('[data-paint="progress"]')?.textContent ?? null })),
  painted: () => Object.fromEntries([...document.querySelectorAll<HTMLElement>('#current [data-paint^="s:"], #current [data-paint^="b:"]')]
    .map(element => [element.dataset.paint, element.textContent])),
  shown: () => Object.fromEntries([...shown].map(([id, hours]) => [id, hours.at(-1) ?? null])),
  progressAt: (id: string, hour: number) => {
    const run = reply.projects.find(project => project.id === id)!.runs.find(run => run.firstHour <= hour && hour <= run.lastHour);
    return run ? `${run.progress}%` : null;
  },
};
(window as unknown as { specialProjects: typeof fixture }).specialProjects = fixture;
