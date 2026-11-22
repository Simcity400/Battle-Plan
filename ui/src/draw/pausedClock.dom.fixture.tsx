import { useCallback, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { createRoot } from 'react-dom/client';
import '../styles.css';
import { PlaybackController } from '../dashboard/playback';
import { useDrawnPanel } from '../dashboard/DrawnPanel';
import { panelFeed } from '../dashboard/panelFeed';
import type { PanelFeed } from '../dashboard/panelFeed';
import { useHeadProduct, withHeld } from '../dashboard/headProduct';
import type { WidgetProps, WidgetRuntime } from '../dashboard/definitions';
import type { WidgetSpec } from '../dashboard/model';
import { clockGeneration, inspectFollow, onDraw } from '../dashboard/pulled';
import { followStore } from '../dashboard/follow/connection';
import { FollowDecoder } from '../dashboard/follow/decode';
import { datedPanel } from '../dashboard/follow/messages.fixture';
import DatedArsenal from '../charts/race/DatedArsenal';
import { dayLabel } from '../charts/draw';
import { readTokens } from '../charts/tokens';
import { tooltipStore } from '../charts/tooltip';
import { FrameEngine } from '../engine/frame';
import AcesWidget from '../widgets/aces/AcesWidget';
import DoctrineMasteryWidget from '../widgets/doctrine-mastery/DoctrineMasteryWidget';
import SpecialProjectsWidget from '../widgets/special-projects/SpecialProjectsWidget';
import { arsenalNotice, arsenalStructure, validateNuclearArsenal } from '../widgets/nuclear-arsenal/nuclearArsenalData';
import type { NuclearArsenalReply } from '../generated/dashboard';
import type { Present } from './notifications';
import type { FromWorker, ToWorker } from './kit';
import type { PageReading } from './followed';

document.documentElement.dataset.motion = 'reduced';
const HOURS = 100_000;
const reply: NuclearArsenalReply = { types: [{ ordinal: 0, token: 'nuke', name: 'Nuclear bomb' }],
  countries: [{ tag: 'GER', countryId: 1, total: 3, counts: [{ ordinal: 0, value: 3, ready: 3 }] }], missing: null, warnings: [] };
const arsenalAt = (hour: number): NuclearArsenalReply => ({ ...reply,
  countries: [{ tag: 'GER', countryId: 1, total: hour, counts: [{ ordinal: 0, value: hour, ready: hour }] }] });
const identities = [{ firstHour: 0, lastHour: HOURS - 1, slot: 1, countryId: 1 }];
const doctrine = { folders: [{ ordinal: 0, id: 'land', name: 'Land', icon: null, grand: { id: 'mobile', name: 'Mobile warfare', icon: null }, completed: [],
  tracks: [{ ordinal: 0, name: 'Infantry', sub: { id: 'sub_a', name: 'Sub A', icon: null }, unlocked: 1, rewards: [{ id: 'r1', name: 'First', threshold: '5000000' }] }] }], warnings: [] };
const projects = { branchId: '1', tag: 'GER', unavailable: null, iterationTexts: {}, identities, available: [{ firstHour: 0, lastHour: 23 }],
  projects: [], presentations: [], people: [], warnings: [] };
const groups = { branchId: '1', warnings: [], warStarts: [], capitulations: [], identityChanges: [], factions: [], wars: [], humans: [],
  unobserved: { humans: [], factions: [], wars: [] }, countryIdentities: [{ tag: 'GER', identities }] };
const replies: Record<string, (hour: number) => unknown> = { 'nuclear-arsenal': arsenalAt, 'air-aces': hour => ({ readHour: hour, missing: null, aces: [], warnings: [] }),
  'doctrine-mastery': () => doctrine, 'special-projects': () => projects,
  'special-project-details': hour => ({ hour, scientists: [], programs: [], projects: [], breakthroughs: [], unavailable: [], warnings: [] }) };
const date = { since: 0, through: HOURS - 1 };
const feed: PanelFeed = { view: 'nuclear-arsenal', offset: 0, structure: arsenalStructure(reply), reply, date };
const clock = new PlaybackController(new FrameEngine({ request() {} }));
clock.setBounds({ firstHour: 0, lastHour: HOURS - 1 });
clock.seek(23);

const gate = { armed: false, held: [] as { message: FromWorker; deliver: () => void }[] };
const views = new Map<number, string>();
class GatedWorker extends Worker {
  override postMessage(message: ToWorker, transfer?: Transferable[] | StructuredSerializeOptions) {
    if (message.type === 'surface') views.set(message.id, message.model.view);
    super.postMessage(message, transfer as Transferable[]);
  }
}
Object.defineProperty(GatedWorker.prototype, 'onmessage', { configurable: true, set(this: Worker, heard: (event: MessageEvent<FromWorker>) => void) {
  this.addEventListener('message', (event: MessageEvent<FromWorker>) => {
    if (!gate.armed || event.data.type !== 'structure' && event.data.type !== 'inspection') { heard(event); return; }
    gate.held.push({ message: event.data, deliver: () => heard(event) });
    wake();
  });
} });
Object.assign(window, { Worker: GatedWorker });

const ports: MessagePort[] = [];
const decoder = new FollowDecoder();
const followed = new Map<number, string>(), pushedAt = new Map<number, string>();
const native = { id: -1, hold: false, asked: null as { hour: number; seek: number } | null, asks: 0 };
function push() {
  const asked = native.asked;
  if (!asked) return;
  const at = `${asked.seek}:${asked.hour}`;
  for (const [id, product] of followed) {
    if (pushedAt.get(id) === at) continue;
    pushedAt.set(id, at);
    const item = decoder.item(datedPanel(id, asked.hour, replies[product]!(asked.hour), asked.seek).buffer);
    for (const port of ports) port.postMessage([item]);
  }
}
const answer = (product: string, params: Record<string, unknown>): unknown => {
  if (product === 'follow') {
    const { id, query } = params as { id: number; query: { panel?: unknown } };
    const panel = String(query.panel);
    if (!replies[panel]) return { available: false, reason: `${panel} is not followed here` };
    if (panel === 'nuclear-arsenal') native.id = id;
    followed.set(id, panel);
    if (!native.hold) push();
    return { available: true, route: 9 };
  }
  if (product === 'follow-clock') {
    const { playing, hour, seek } = params as { playing: boolean; hour: number; seek: number };
    if (!playing) { native.asked = { hour: Math.floor(hour), seek }; if (!native.hold) push(); }
    return null;
  }
  if (product === 'unfollow') { followed.delete((params as { id: number }).id); return null; }
  if (product === 'country-groups') return groups;
  const served = replies[product];
  if (!served) throw new Error(`${product} is not served here`);
  native.asks++;
  return served((params as { hour: number }).hour);
};
const client = {
  available: true,
  product: async (request: { sourceId: string; revision: string; product: string; params: Record<string, unknown> }) =>
    ({ sourceId: request.sourceId, revision: request.revision, product: request.product, data: answer(request.product, request.params) }),
  openFollow: async (given: readonly MessagePort[]) => {
    for (const port of given) { port.onmessage = ({ data }) => decoder.release(data); ports.push(port); }
  },
  setMapViews: async () => {}, onMapStates: async () => () => {}, releaseSource: async () => {}, openSource: async () => { throw new Error('none'); },
};
const track = { id: 'main', axisOffset: 0, selectedTag: 'GER', source: { id: 'paused', revision: '1', metadata: { countries: [] } },
  branch: { id: '1', firstHour: 0, lastHour: HOURS - 1, gaps: [] } };
const runtime = { tracks: [track], clock, client, addWidgetBelow() {}, campaigns: [], recordings: [], requestSeek() {} } as unknown as WidgetRuntime;
const spec = (id: string, kind: string, countries: unknown, options: Record<string, unknown> = {}) =>
  ({ id, kind, configVersion: 1, trackIds: ['main'], countries, time: { mode: 'follow' }, options }) as unknown as WidgetSpec;
const headed = spec('headed', 'nuclear-arsenal', { kind: 'tags', tags: ['GER'] });
const widgetProps = (widget: WidgetSpec) => ({ widget, runtime, active: true, headerSlot: null, onDiagnostics() {} }) as unknown as WidgetProps;
const aces = widgetProps(spec('aces', 'aces', { kind: 'all' }));
const doctrines = widgetProps(spec('doctrine', 'doctrine-mastery', { kind: 'tags', tags: ['GER'] }));
const specials = widgetProps(spec('projects', 'special-projects', { kind: 'tags', tags: ['GER'] }, { view: 'live' }));
const NO_PARAMS = {};
const DOCTRINE_TIPS = 'doctrine-mastery:doctrine';

const presents = new Map<number, Present>();
const drawn = new Map<number, number | null>();
let racePresent: Extract<FromWorker, { type: 'racePresent' }> | undefined;
const emptied = { seek: -1 };
let strict: number | null = null, followedSurface = false, listedRead = '';
const waiting = new Set<() => void>();
const wake = () => queueMicrotask(() => { for (const heard of [...waiting]) heard(); });
onDraw(message => {
  if (message.type === 'present') for (const surface of message.surfaces) {
    presents.set(surface.id, surface);
    if (surface.readHour !== undefined || !surface.waiting) drawn.set(surface.id, surface.readHour ?? null);
  }
  if (message.type === 'racePresent') racePresent = message;
  if (message.type === 'structure' && message.updates.some(update => update.id === native.id && update.panel === null))
    emptied.seek = clock.getSnapshot().seekSequence;
  wake();
});
tooltipStore.subscribe(wake);
const arsenals = () => [...presents.keys()].filter(id => views.get(id) === 'nuclear-arsenal');
const panelId = () => Math.min(...arsenals()), headedId = () => { const ids = arsenals(); return ids.length > 1 ? Math.max(...ids) : -1; };
const panelPresent = () => presents.get(panelId());
const headedPresent = () => presents.get(headedId());

function Panel() {
  const root = useRef<HTMLDivElement>(null);
  useDrawnPanel(root, clock, feed, true);
  return <div id="panel" ref={root} style={{ width: '480px', height: '40px' }}>
    <span data-paint="empty">{arsenalNotice(reply, clock.getHour())}</span>
  </div>;
}

function Headed() {
  const root = useRef<HTMLDivElement | null>(null);
  const read = useHeadProduct(runtime, headed, 'nuclear-arsenal', NO_PARAMS, { active: true, validate: validateNuclearArsenal, structure: arsenalStructure });
  const head = withHeld(read);
  useDrawnPanel(root, clock, panelFeed('nuclear-arsenal', read, arsenalStructure, headed, runtime), true);
  const attach = useCallback((element: HTMLDivElement | null) => { root.current = element; read.root(element); }, [read.root]);
  useLayoutEffect(() => { strict = read.data?.countries[0]?.total ?? null; followedSurface = read.read === 'followed'; wake(); });
  return <div id="headed" ref={attach} style={{ width: '480px', height: '40px' }}>
    {head.data && <span data-paint="empty">{arsenalNotice(head.data, clock.getHour())}</span>}
  </div>;
}

const totalOf = (reply: NuclearArsenalReply) => String(reply.countries[0]?.total);
function Listed() {
  const head = useHeadProduct(runtime, headed, 'nuclear-arsenal', NO_PARAMS, { active: true, validate: validateNuclearArsenal, structure: totalOf });
  useLayoutEffect(() => { listedRead = head.read; wake(); });
  if (!head.data) return <div className="listed-loading">Loading the arsenal…</div>;
  return <p id="listed" ref={head.root}>{head.data.countries[0]?.total}</p>;
}

const probe = { commits: 0, bump: () => {} };
function Probe() {
  const [tick, setTick] = useState(0);
  probe.bump = () => setTick(count => count + 1);
  useLayoutEffect(() => { probe.commits++; wake(); }, [tick]);
  return null;
}
const removal = { seen: false, after: Infinity };
function watchRemoval() {
  const listed = document.querySelector('#listed');
  removal.seen = false; removal.after = Infinity;
  if (!listed) return;
  const observer = new IntersectionObserver(entries => {
    if (listed.isConnected || entries.every(entry => entry.isIntersecting)) return;
    observer.disconnect(); removal.seen = true; removal.after = probe.commits; probe.bump();
  });
  observer.observe(listed);
}
const removalSeen = () => removal.seen && probe.commits > removal.after;

function Race() {
  return <div id="race" style={{ width: '480px', height: '160px', display: 'flex' }}>
    <DatedArsenal label="Nuclear arsenal" items={[]} slots={5} name={key => key} active clock={clock}
      model={{ offset: 0, reply, date, top: 10, tags: ['GER'], split: false, tokens: readTokens(), slots: 5, overlay: true,
        colours: new Map([['GER', '#808080']]), looks: new Map([['GER', { name: 'Germany', color: '#808080' }]]) }} />
  </div>;
}

const pop = document.createElement('div');
pop.id = 'pop'; pop.popover = 'manual'; pop.style.inset = 'auto 0 0 auto'; pop.style.margin = '0';
document.body.append(pop);
const box = { width: '260px', height: '140px', overflow: 'hidden' };

let mounts = 0;
new MutationObserver(records => {
  for (const record of records) for (const node of record.addedNodes) if (node instanceof HTMLCanvasElement && node.closest('#headed')) mounts++;
}).observe(document.body, { childList: true, subtree: true });

function opacity(element: Element | null) {
  let seen = 1;
  for (let at = element; at; at = at.parentElement) seen *= Number(getComputedStyle(at).opacity);
  return Math.round(seen * 1000) / 1000;
}
function surface(canvas: HTMLCanvasElement | null, region: Element | null, visible: boolean, hour: number | null | undefined) {
  return { visible, opacity: opacity(canvas), busy: region?.getAttribute('aria-busy') === 'true', hour: hour ?? null };
}
const ink = (panel: string) => document.querySelector<HTMLElement>(`#${panel} [data-paint="empty"]`)?.dataset.rowInk ?? null;
const inkOf = (panel: string) => document.querySelector<HTMLCanvasElement>(`#${panel} canvas.draw-panel-ink`);
const stateOf = (selector: string) => document.querySelector<HTMLElement>(selector)?.dataset.state ?? null;
function tip() {
  const open = tooltipStore.get();
  return open?.owner === DOCTRINE_TIPS ? open.content.rows?.find(row => row.label === 'As of')?.value ?? '' : null;
}
function shown() {
  const ink1 = inkOf('panel'), ink2 = inkOf('headed');
  const race = document.querySelector<HTMLCanvasElement>('#race canvas');
  return { field: ink('panel'), headedField: ink('headed'), mounts, hour: clock.getHour(), strict, asks: native.asks, listedRead,
    listed: document.querySelector('#listed')?.textContent ?? null,
    aces: stateOf('#aces .widget-state'), projects: stateOf('#projects .special-projects > .widget-state'),
    tip: tip(), described: !!document.querySelector('#doctrine span.sr-only[id]'),
    muted: Number(getComputedStyle(document.documentElement).getPropertyValue('--muted-row-opacity')),
    panel: surface(ink1, document.querySelector('#panel'), !!ink1 && !ink1.hidden, drawn.get(panelId())),
    race: surface(race, document.querySelector('#race .race-chart'), !!race && race.style.visibility !== 'hidden', racePresent?.hour),
    headed: surface(ink2, document.querySelector('#headed'), !!ink2 && !ink2.hidden, drawn.get(headedId())) };
}
const lit = (now: { visible: boolean; opacity: number }) => now.visible && now.opacity === 1;
const painted = () => { const now = shown(); return now.field === 'worker' && now.headedField === 'worker' && lit(now.panel) && lit(now.race) && lit(now.headed); };
const currentOf = (...given: (Present | Extract<FromWorker, { type: 'racePresent' }> | undefined)[]) => {
  const generation = clockGeneration(clock), seek = clock.getSnapshot().seekSequence;
  return given.every(present => present?.generation === generation && present.seek === seek);
};
const current = () => currentOf(panelPresent(), headedPresent(), racePresent);
const readEmptied = () => emptied.seek === clock.getSnapshot().seekSequence && strict === null;
const widgetsAt = (hour: number) => {
  const now = shown();
  return now.aces === 'empty' && now.projects === (hour <= 23 ? 'empty' : 'unavailable');
};

function until(ready: () => boolean) {
  return new Promise<void>(resolve => {
    const heard = () => { if (ready()) { waiting.delete(heard); resolve(); } };
    waiting.add(heard); heard();
  });
}

async function watch(change: () => void, settled: () => Promise<void> = () => until(current)) {
  change();
  const seen = [shown()];
  let watching = true;
  const sample = () => { if (!watching) return; seen.push(shown()); requestAnimationFrame(sample); };
  requestAnimationFrame(sample);
  await settled();
  watching = false;
  seen.push(shown());
  return seen;
}

function at(time: number, change: () => void) {
  performance.now = () => time;
  try { change(); } finally { Reflect.deleteProperty(performance, 'now'); }
}

const landed = (hour: number) => current() && painted() && headedPresent()?.readHour === hour && shown().listed === String(hour) && widgetsAt(hour);

async function heldSeek(hour: number, settled: () => boolean, done: () => boolean = () => true) {
  let pending: ReturnType<typeof shown>[] = [];
  const seen = await watch(() => { native.hold = true; watchRemoval(); clock.seek(hour); }, async () => {
    await until(() => settled() && removalSeen());
    pending = [shown()];
    native.hold = false; push();
    if (pending[0]!.listedRead === 'followed') await until(() => landed(hour) && strict === hour && done());
  });
  return { seen, pending: pending[0]! };
}

async function staleStep() {
  const hour = clock.getHour(), store = followStore(runtime.client, runtime.tracks[0]!.source, clock, track.axisOffset);
  const readings = (message: FromWorker): readonly (PageReading | undefined)[] =>
    message.type === 'structure' ? message.updates : message.type === 'inspection' ? [message.reading] : [];
  const said = (type: 'structure' | 'inspection', hours: number) => gate.held.some(({ message }) =>
    message.type === type && readings(message).some(reading => reading?.id === native.id && !!reading.panel && reading.hour === hours));
  gate.armed = true;
  clock.step(1);
  await until(() => said('structure', hour + 1));
  void inspectFollow(native.id);
  await until(() => said('inspection', hour + 1));
  native.hold = true;
  const stale = gate.held.splice(0);
  clock.step(1);
  const taken = stale.map(({ message, deliver }) => {
    deliver();
    const entry = store.get(native.id);
    return { type: message.type, hour: entry?.panel ? entry.hour : null };
  });
  const pending = shown();
  gate.armed = false;
  for (const { deliver } of gate.held.splice(0)) deliver();
  native.hold = false; push();
  if (taken.every(reply => reply.hour === null)) await until(() => landed(hour + 2) && strict === hour + 2);
  return { taken, pending, done: shown() };
}

export const fixture = {
  async mount() {
    pop.showPopover();
    createRoot(document.body.appendChild(document.createElement('div'))).render(<><Panel /><Race /><Headed />{createPortal(<Listed />, pop)}<Probe />
      <div style={{ display: 'flex', gap: '8px' }}><div id="aces" style={box}><AcesWidget {...aces} /></div>
        <div id="doctrine" style={box}><DoctrineMasteryWidget {...doctrines} /></div><div id="projects" style={box}><SpecialProjectsWidget {...specials} /></div></div></>);
    await until(() => followedSurface && listedRead === 'followed' && landed(23) && !!document.querySelector('#doctrine [data-track]'));
    document.querySelector<HTMLElement>('#doctrine [data-track]')!.focus();
    await until(() => shown().tip === dayLabel(23) && shown().described);
    return shown();
  },
  speed: (hoursPerSecond: number) => watch(() => clock.setSpeed(hoursPerSecond)),
  heldSeek: (hour: number) => heldSeek(hour, () => current() && readEmptied(), () => shown().tip === dayLabel(hour)),
  heldRewind: (hour: number) => heldSeek(hour, () => currentOf(panelPresent(), racePresent) && readEmptied()),
  step: () => watch(() => clock.step(1), () => until(() => landed(clock.getHour()))),
  staleStep,
  async playFor(milliseconds: number) {
    const start = Math.ceil(performance.now());
    at(start, () => clock.setPlaying(true));
    await until(current);
    at(start + milliseconds, () => clock.setPlaying(false));
    await until(() => landed(clock.getHour()));
    return shown();
  },
};
(window as unknown as { pausedClock: typeof fixture }).pausedClock = fixture;
