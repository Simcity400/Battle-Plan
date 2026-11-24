import { datedPanel } from '../../dashboard/follow/messages.fixture.ts';
import { packet, splitForLanes } from '../../dashboard/follow/split.ts';
import { Profiler } from 'react';
import type { ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import type { Root } from 'react-dom/client';
import SpecialProjectsWidget from './SpecialProjectsWidget';
import { MeasuredWidget } from '../../dashboard/widgetPerf.ts';
import { measureWith } from '../../shell/perf.ts';
import type { PerfRecorder } from '../../shell/perf.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import type { ProjectStatus, SpecialProjectDetails, SpecialProjectsReply } from '../../generated/dashboard';
import { tooltipStore } from '../../charts/tooltip.ts';

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

const daily = (from: number, start: number, perDay: number): ProjectStatus[] => {
  const runs: ProjectStatus[] = [];
  for (let day = 0; from + day * 24 < HOURS; day++) {
    const progress = Math.min(100, start + Math.floor(day * perDay));
    runs.push({ firstHour: from + day * 24, lastHour: Math.min(HOURS - 1, from + day * 24 + 23), readHour: from + day * 24, presentation: 0, progress, state: 'running',
      iterations: Math.floor(progress / 25), scientistRead: null, lastScientistRead: null });
  }
  return runs;
};
const merge = (runs: ProjectStatus[]) => runs.reduce<ProjectStatus[]>((out, run) => {
  const last = out.at(-1);
  if (last && last.progress === run.progress && last.iterations === run.iterations) last.lastHour = run.lastHour; else out.push({ ...run });
  return out;
}, []);
export const reply: SpecialProjectsReply = { branchId: '1', tag: 'GER', shown: { through: null, running: [], completed: [], undated: 0 }, identities, available: [{ firstHour: 0, lastHour: HOURS - 1 }], partial: [], unresolved: [],
  presentations: [{ name: 'Rocket engines', specialization: null, specializationName: null, icon: null, iconMissing: 'none', scientist: 0, lastScientist: null }], reads: [],
  people: [{ assigned: false, id: null, name: null, portrait: null, missing: 'No scientist assigned.' }],
  iterationTexts: { 0: '0', 1: '1', 2: '2', 3: '3', 4: '4' }, unavailable: null, warnings: [],
  projects: [
    { id: 'fast', owner: 9, local: 1, firstRead: 0, firstState: 'running', statuses: merge(daily(0, 10, 1 / 50)), entries: [], attempts: [] },
    { id: 'slow', owner: 9, local: 2, firstRead: 0, firstState: 'running', statuses: merge(daily(0, 0, 1 / 60)), entries: [], attempts: [] },
    { id: 'late', owner: 9, local: 3, firstRead: LATE, firstState: 'running', statuses: merge(daily(LATE, 0, 0)), entries: [], attempts: [] },
  ] };

export const LISTED = 33_000;
const whole: SpecialProjectsReply = { ...reply, presentations: reply.presentations.map(info => ({ ...info, name: 'Whole-branch name' })) };
export const replyAt = (hour: number): SpecialProjectsReply => hour < LISTED ? reply : { ...reply, projects: reply.projects.map(project => project.id !== 'fast' ? project
  : { ...project, entries: [{ ordinal: 0, date: LISTED - 10, completion: true, readHour: LISTED, counted: true, presentation: 0 }] }) };

export const details = (read: number): SpecialProjectDetails => ({ hour: read, unavailable: [], warnings: [], projects: [],
  scientists: [{ character: 7, person: { assigned: true, id: 7, name: 'Wernher', portrait: null, missing: null }, active: 1,
    injuryDays: Math.max(0, 400 - Math.floor(read / 24) % 400), injuryHour: read, visible: true, visibilityHour: read, stateRefusals: {}, skills: [], traits: [] }],
  breakthroughs: [{ specialization: { token: 'sp_rockets', name: 'Rocketry' }, points: Math.floor(read / 24) * 100, readHour: read }],
  programs: [] });

const stamp = { route: 9, epoch: 1, generation: 1 };
const panels: { followed: Map<number, string>; ports: readonly MessagePort[]; refuse: string | null } = { followed: new Map(), ports: [], refuse: null };
function follow(params: { id: number; query: { panel?: string } }) {
  const panel = params.query.panel;
  if (panel !== 'special-project-details' && panel !== 'special-projects' && panel !== 'special-projects-history') return { available: false, reason: 'not followed here' };
  panels.followed.set(params.id, panel);
  return { available: true, ...stamp };
}
function refusedPanel(id: number, hour: number, reason: string) {
  const body = new TextEncoder().encode(reason);
  const out = new Uint8Array((40 + body.length + 7) & ~7), words = new Uint32Array(out.buffer);
  words.set([0x324c4e50, 6, 1, 0, 9, 0, id, hour, 0, body.length]);
  out.set(body, 40);
  return out;
}
function push(hour: number) {
  const read = hour - (hour - 4) % 24;
  for (const [id, panel] of panels.followed) {
    const bytes = panel === 'special-project-details' ? datedPanel(id, hour, details(read), 0)
      : panels.refuse !== null ? refusedPanel(id, hour, panels.refuse) : datedPanel(id, hour, replyAt(hour), 0);
    const split = splitForLanes(bytes, panels.ports.length);
    if (split.kind !== 'failure') split.lanes.forEach((part, lane) => { if (part) panels.ports[lane]!.postMessage(packet('panels', part, undefined), [part.buffer]); });
  }
}

let clock: Clock;
type Snapshot = { position: number; hour: number; fraction: number; playing: boolean; hoursPerSecond: number; sequence: number; seekSequence: number;
  composedAt: number; bounds: { firstHour: number; lastHour: number } };
type Published = (previous: Snapshot, next: Snapshot) => void;
class Clock {
  readonly clockId = 'fixture';
  private snapshot: Snapshot = { position: START, hour: START, fraction: 0, playing: false, hoursPerSecond: 1440, sequence: 0, seekSequence: 0, composedAt: 0,
    bounds: { firstHour: 0, lastHour: HOURS - 1 } };
  private readonly frames = new Set<() => void>();
  private readonly anchors = new Set<() => void>();
  private readonly firsts = new Set<Published>();
  private readonly publishes = new Set<Published>();
  private readonly anchored = new Set<Published>();
  get position() { return this.snapshot.position; }
  get playing() { return this.snapshot.playing; }
  getSnapshot = () => this.snapshot;
  getHour = () => this.snapshot.hour;
  getMotion = () => this.snapshot.playing ? this.snapshot.hoursPerSecond : undefined;
  positionAt = () => this.snapshot.position;
  composedAt = () => this.snapshot.composedAt;
  subscribe = (listener: () => void) => { this.anchors.add(listener); return () => { this.anchors.delete(listener); }; };
  subscribeFrame = (listener: () => void) => { this.frames.add(listener); return () => { this.frames.delete(listener); }; };
  subscribeHour = () => () => {};
  onPublish = (listener: Published, first = false) => { const into = first ? this.firsts : this.publishes; into.add(listener); return () => { into.delete(listener); }; };
  onAnchor = (listener: Published) => { this.anchored.add(listener); return () => { this.anchored.delete(listener); }; };
  onDiscard = () => () => {};
  private publish(next: Partial<Snapshot>, anchor: boolean) {
    const previous = this.snapshot, position = next.position ?? previous.position, hour = Math.floor(position);
    this.snapshot = { ...previous, ...next, position, hour, fraction: position - hour, composedAt: performance.now(), sequence: previous.sequence + (hour !== previous.hour ? 1 : 0) };
    for (const listener of [...this.firsts]) listener(previous, this.snapshot);
    for (const listener of [...this.publishes]) listener(previous, this.snapshot);
    if (anchor) for (const listener of [...this.anchored]) listener(previous, this.snapshot);
    for (const listener of [...this.frames]) listener();
    if (anchor) for (const listener of [...this.anchors]) listener();
  }
  setPlaying(playing: boolean) { this.publish({ playing }, true); }
  step(hours: number) { this.publish({ position: this.snapshot.position + hours }, false); }
}

function answer(product: string): unknown {
  if (product === 'country-groups') return GROUPS;
  if (product === 'special-projects') return whole;
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
    const data = request.product === 'follow' ? follow(request.params as never) : request.product === 'follow-clock' || request.product === 'unfollow' ? { available: true, ...stamp } : answer(request.product);
    return { sourceId: request.sourceId, revision: request.revision, product: request.product, data };
  },
  openFollow: async (ports: readonly MessagePort[]) => { panels.ports = ports; },
  setMapViews: async () => {}, onMapStates: async () => () => {}, releaseSource: async () => {}, openSource: async () => { throw new Error('none'); },
};

const counted = (id: string, children: ReactNode) => <MeasuredWidget value={id}>
  <Profiler id={id} onRender={() => { renders[id] = (renders[id] ?? 0) + 1; }}>{children}</Profiler>
</MeasuredWidget>;

export const fixture = {
  async mount(refuse: string | null = null) {
    root?.unmount(); host?.remove();
    clock = new Clock(); requests.length = 0; shown.clear(); panels.followed.clear(); panels.refuse = refuse;
    for (const key of Object.keys(renders)) delete renders[key];
    host = document.body.appendChild(document.createElement('div'));
    host.style.cssText = 'zoom: 0.75; width: 640px; display: flex; flex-direction: column;';
    root = createRoot(host);
    const runtime = { tracks: [track], clock, client, addWidgetBelow() {}, campaigns: [], recordings: [], requestSeek() {} };
    const widget = (id: string, view: string) => ({ id, kind: 'special-projects', trackIds: ['main'], countries: { kind: 'tags', tags: ['GER'] },
      time: { mode: 'follow' }, options: { view } });
    const props = (spec: unknown) => ({ widget: spec, runtime, active: true, onDiagnostics() {} }) as unknown as WidgetProps;
    root.render(<>
      <div id="current" style={{ height: '320px', display: 'flex' }}>{counted('current', <SpecialProjectsWidget {...props(widget('current', 'live'))} />)}</div>
      <div id="history" style={{ height: '280px', display: 'flex' }}>{counted('history', <SpecialProjectsWidget {...props(widget('history', 'history'))} />)}</div>
    </>);
    await settle(30);
    push(clock.getHour());
    await settle(30);
    push(clock.getHour());
    await settle(4);
  },
  async play(frames: number, hours: number) {
    clock.setPlaying(true);
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
  pause() { clock.setPlaying(false); },
  asked: () => [...requests],
  text: (id: string) => document.getElementById(id)?.textContent ?? '',
  statusStarts: (from: number, to: number) => reply.projects.reduce((count, project) => count + project.statuses.filter(run => from < run.firstHour && run.firstHour <= to).length, 0),
  historyKey(key: string) {
    const canvas = document.querySelector<HTMLCanvasElement>('#history canvas[aria-label="Special project history"]');
    if (!canvas) return null;
    canvas.focus();
    canvas.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, ...key === 'F10' ? { shiftKey: true } : {} }));
    return true;
  },
  tooltip: () => {
    const held = tooltipStore.get();
    return held ? { title: held.content.title ?? null, rows: (held.content.rows ?? []).map(row => [row.label, String(row.value)]) } : null;
  },
  async menuFor(card: string) {
    const element = document.querySelector<HTMLElement>(`#current [data-project="${card}"]`);
    if (!element) return null;
    const box = element.getBoundingClientRect();
    element.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: box.left + 4, clientY: box.top + 4 }));
    await settle(4);
    return menuItems();
  },
  menu: async () => { await settle(4); return menuItems(); },
  settled: () => settle(8),

  cards: () => [...document.querySelectorAll<HTMLElement>('#current [data-project]')].map(card => ({ id: card.dataset.project,
    progress: card.querySelector('[data-paint="progress"]')?.textContent ?? null })),
  painted: () => Object.fromEntries([...document.querySelectorAll<HTMLElement>('#current [data-paint^="s:"], #current [data-paint^="b:"]')]
    .map(element => [element.dataset.paint, element.textContent])),
  shown: () => Object.fromEntries([...shown].map(([id, hours]) => [id, hours.at(-1) ?? null])),
  progressAt: (id: string, hour: number) => {
    const run = reply.projects.find(project => project.id === id)!.statuses.find(run => run.firstHour <= hour && hour <= run.lastHour);
    return run ? `${run.progress}%` : null;
  },
};
function menuItems() {
  const items = [...document.querySelectorAll<HTMLElement>(':popover-open [role="menuitem"]')].map(item => item.textContent ?? '');
  document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
  return items;
}
(window as unknown as { specialProjects: typeof fixture }).specialProjects = fixture;
