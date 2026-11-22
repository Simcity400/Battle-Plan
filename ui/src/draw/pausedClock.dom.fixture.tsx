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
import NuclearArsenalWidget from '../widgets/nuclear-arsenal/NuclearArsenalWidget';
import { arsenalNotice, arsenalStructure, validateNuclearArsenal } from '../widgets/nuclear-arsenal/nuclearArsenalData';
import type { NuclearArsenalReply } from '../generated/dashboard';
import type { Present } from './notifications';
import type { FromWorker, ToWorker } from './kit';
import type { PageReading } from './followed';

document.documentElement.dataset.motion = 'reduced';
const HOURS = 100_000;
const SHORT_END = 100;
const reply: NuclearArsenalReply = { types: [{ ordinal: 0, token: 'nuke', name: 'Nuclear bomb' }],
  countries: [{ tag: 'GER', countryId: 1, total: 3, counts: [{ ordinal: 0, value: 3, ready: 3 }] }], missing: null, warnings: [] };
const arsenalAt = (hour: number): NuclearArsenalReply => ({ ...reply,
  countries: [{ tag: 'GER', countryId: 1, total: hour, counts: [{ ordinal: 0, value: hour, ready: hour }] }], warnings: [`read at ${hour}`] });
const doctrine = { folders: [{ ordinal: 0, id: 'land', name: 'Land', icon: null, grand: { id: 'mobile', name: 'Mobile warfare', icon: null }, completed: [],
  tracks: [{ ordinal: 0, name: 'Infantry', sub: { id: 'sub_a', name: 'Sub A', icon: null }, unlocked: 1, rewards: [{ id: 'r1', name: 'First', threshold: '5000000' }] }] }], warnings: [] };
const replies: Record<string, (hour: number, query: Record<string, unknown>) => unknown> = { 'nuclear-arsenal': arsenalAt,
  'air-aces': hour => ({ readHour: hour, missing: null, aces: [], warnings: [] }),
  'doctrine-mastery': (_hour, query) => query.country === 'ENG' ? { folders: [], warnings: [] } : doctrine };
const date = { since: 0, through: HOURS - 1 };
const feed: PanelFeed = { view: 'nuclear-arsenal', offset: 0, structure: arsenalStructure(reply), reply, date };
const clock = new PlaybackController(new FrameEngine({ request() {} }));
clock.setBounds({ firstHour: 0, lastHour: HOURS - 1 });
clock.seek(23);

const gate = { holds: null as ((message: FromWorker) => boolean) | null, held: [] as { message: FromWorker; deliver: () => void }[] };
const offscreens = new WeakMap<OffscreenCanvas, HTMLCanvasElement>(), surfaces = new WeakMap<HTMLCanvasElement, number>(), owners = new Map<number, number>();
const transferred = HTMLCanvasElement.prototype.transferControlToOffscreen;
HTMLCanvasElement.prototype.transferControlToOffscreen = function (this: HTMLCanvasElement) {
  const offscreen = transferred.call(this);
  offscreens.set(offscreen, this);
  return offscreen;
};
const surfaceOf = (canvas: HTMLCanvasElement | null) => canvas && surfaces.get(canvas);
class GatedWorker extends Worker {
  override postMessage(message: ToWorker, transfer?: Transferable[] | StructuredSerializeOptions) {
    const canvas = (message.type === 'surface' || message.type === 'race') && message.canvas && offscreens.get(message.canvas);
    if (canvas) surfaces.set(canvas, message.id);
    if (message.type === 'surface' || message.type === 'race') owners.set(message.id, message.owner);
    super.postMessage(message, transfer as Transferable[]);
  }
}
Object.defineProperty(GatedWorker.prototype, 'onmessage', { configurable: true, set(this: Worker, heard: (event: MessageEvent<FromWorker>) => void) {
  this.addEventListener('message', (event: MessageEvent<FromWorker>) => {
    const deliver = () => { delivered(event.data); heard(event); wake(); };
    if (!gate.holds?.(event.data)) { deliver(); return; }
    gate.held.push({ message: event.data, deliver });
    wake();
  });
} });
Object.assign(window, { Worker: GatedWorker });

const ports: MessagePort[] = [];
const decoder = new FollowDecoder();
const followed = new Map<number, Record<string, unknown>>(), pushedAt = new Map<number, string>();
const native = { id: -1, hold: false, asked: null as { hour: number; seek: number } | null, asks: 0 };
function push() {
  const asked = native.asked;
  if (!asked) return;
  const at = `${asked.seek}:${asked.hour}`;
  for (const [id, query] of followed) {
    if (pushedAt.get(id) === at || query.branchId === shortTrack.branch.id && asked.hour > SHORT_END) continue;
    pushedAt.set(id, at);
    const item = decoder.item(datedPanel(id, asked.hour, replies[String(query.panel)]!(asked.hour, query), asked.seek).buffer);
    for (const port of ports) port.postMessage([item]);
  }
}
const answer = (product: string, params: Record<string, unknown>): unknown => {
  if (product === 'follow') {
    const { id, query } = params as { id: number; query: Record<string, unknown> };
    const panel = String(query.panel);
    if (!replies[panel]) return { available: false, reason: `${panel} is not followed here` };
    if (panel === 'nuclear-arsenal') native.id = id;
    followed.set(id, query);
    if (!native.hold) push();
    return { available: true, route: 9 };
  }
  if (product === 'follow-clock') {
    const { playing, hour, seek } = params as { playing: boolean; hour: number; seek: number };
    if (!playing) { native.asked = { hour: Math.floor(hour), seek }; if (!native.hold) push(); }
    return null;
  }
  if (product === 'unfollow') { followed.delete((params as { id: number }).id); return null; }
  const served = replies[product];
  if (!served) throw new Error(`${product} is not served here`);
  native.asks++;
  return served((params as { hour: number }).hour, params);
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
const shortTrack = { ...track, id: 'short', branch: { id: '2', firstHour: 0, lastHour: SHORT_END, gaps: [] } };
const runtime = { tracks: [track, shortTrack], clock, client, addWidgetBelow() {}, campaigns: [], recordings: [], requestSeek() {} } as unknown as WidgetRuntime;
const spec = (id: string, kind: string, countries: unknown, options: Record<string, unknown> = {}, trackId = 'main') =>
  ({ id, kind, configVersion: 1, trackIds: [trackId], countries, time: { mode: 'follow' }, options }) as unknown as WidgetSpec;
const headed = spec('headed', 'nuclear-arsenal', { kind: 'tags', tags: ['GER'] });
const widgetProps = (widget: WidgetSpec) => ({ widget, runtime, active: true, headerSlot: null, onDiagnostics() {} }) as unknown as WidgetProps;
const aces = widgetProps(spec('aces', 'aces', { kind: 'all' }));
const shortAces = widgetProps(spec('short-aces', 'aces', { kind: 'all' }, {}, shortTrack.id));
const doctrines = widgetProps(spec('doctrine', 'doctrine-mastery', { kind: 'tags', tags: ['GER'] }));
const noDoctrines = widgetProps(spec('no-doctrines', 'doctrine-mastery', { kind: 'tags', tags: ['ENG'] }));
const arsenalWarnings = { told: [] as string[] };
const arsenal = { ...widgetProps(spec('arsenal', 'nuclear-arsenal', { kind: 'tags', tags: ['GER'] }, { view: 'race' })),
  onDiagnostics: (warnings: string[]) => { arsenalWarnings.told = warnings; wake(); } };
const NO_PARAMS = {};
const DOCTRINE_TIPS = 'doctrine-mastery:doctrine';

const presents = new Map<number, Present>();
const drawn = new Map<number, number | null>();
const racePresents = new Map<number, Extract<FromWorker, { type: 'racePresent' }>>();
const emptied = { seek: -1 };
let strict: number | null = null, followedSurface = false, listedRead = '';
const waiting = new Set<() => void>();
const wake = () => queueMicrotask(() => { for (const heard of [...waiting]) heard(); });
function delivered(message: FromWorker) {
  if (message.type === 'present') for (const surface of message.surfaces) {
    presents.set(surface.id, surface);
    const accepted = surface.owner === owners.get(surface.id) && surface.seek === clock.getSnapshot().seekSequence && surface.generation === clockGeneration(clock);
    if (accepted && (surface.readHour !== undefined || !surface.waiting)) drawn.set(surface.id, surface.readHour ?? null);
  }
  if (message.type === 'racePresent') racePresents.set(message.id, message);
}
onDraw(message => {
  if (message.type === 'structure' && message.updates.some(update => update.id === native.id && update.panel === null))
    emptied.seek = clock.getSnapshot().seekSequence;
  wake();
});
tooltipStore.subscribe(wake);
const ink = (panel: string) => document.querySelector<HTMLElement>(`#${panel} [data-paint="empty"]`)?.dataset.rowInk ?? null;
const inkOf = (panel: string) => document.querySelector<HTMLCanvasElement>(`#${panel} canvas.draw-panel-ink`);
const raceCanvas = () => document.querySelector<HTMLCanvasElement>('#race canvas');
const panelPresent = () => presents.get(surfaceOf(inkOf('panel')) ?? -1);
const headedPresent = () => presents.get(surfaceOf(inkOf('headed')) ?? -1);
const racePresent = () => racePresents.get(surfaceOf(raceCanvas()) ?? -1);

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

const raceReply = { set: (_: NuclearArsenalReply) => {} };
const withoutGermany: NuclearArsenalReply = { ...reply, countries: [{ tag: 'ENG', countryId: 2, total: 1, counts: [{ ordinal: 0, value: 1, ready: 1 }] }] };
function Race() {
  const [ranked, setRanked] = useState(reply);
  raceReply.set = setRanked;
  return <div id="race" style={{ width: '480px', height: '160px', display: 'flex' }}>
    <DatedArsenal label="Nuclear arsenal" items={[]} slots={5} name={key => key} active clock={clock} tooltip={key => ({ title: `${key} at ${clock.getHour()}`, rows: [] })}
      model={{ offset: 0, reply: ranked, date, top: 10, tags: ['GER'], split: false, tokens: readTokens(), slots: 5, overlay: true,
        colours: new Map([['GER', '#808080']]), looks: new Map([['GER', { name: 'Germany', color: '#808080' }]]) }} />
  </div>;
}

const pop = document.createElement('div');
pop.id = 'pop'; pop.popover = 'manual'; pop.style.inset = 'auto 0 0 auto'; pop.style.margin = '0';
document.body.append(pop);
const box = { width: '240px', height: '140px', overflow: 'hidden' };

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
const stateOf = (selector: string) => document.querySelector<HTMLElement>(selector)?.dataset.state ?? null;
function tip() {
  const open = tooltipStore.get();
  return open?.owner === DOCTRINE_TIPS ? open.content.rows?.find(row => row.label === 'As of')?.value ?? '' : null;
}
function shown() {
  const ink1 = inkOf('panel'), ink2 = inkOf('headed'), race = raceCanvas();
  return { field: ink('panel'), headedField: ink('headed'), mounts, hour: clock.getHour(), strict, asks: native.asks, listedRead,
    listed: document.querySelector('#listed')?.textContent ?? null, warned: arsenalWarnings.told.filter(warning => warning.startsWith('read at ')),
    aces: stateOf('#aces .widget-state'), shortAces: stateOf('#short-aces .widget-state'), noDoctrines: stateOf('#no-doctrines .widget-state'), tip: tip(), described: !!document.querySelector('#doctrine span.sr-only[id]'),
    raceDescribed: !!raceCanvas()?.getAttribute('aria-describedby'), raceRanking: document.querySelector('#race p[id$="-ranking"]')?.textContent ?? '',
    raceLive: document.querySelector('#race p[aria-live]')?.textContent ?? '', raceFocused: !!raceCanvas() && document.activeElement === raceCanvas(),
    muted: Number(getComputedStyle(document.documentElement).getPropertyValue('--muted-row-opacity')),
    panel: surface(ink1, document.querySelector('#panel'), !!ink1 && !ink1.hidden, drawn.get(surfaceOf(ink1) ?? -1)),
    race: surface(race, document.querySelector('#race .race-chart'), !!race && race.style.visibility !== 'hidden', racePresent()?.hour),
    headed: surface(ink2, document.querySelector('#headed'), !!ink2 && !ink2.hidden, drawn.get(surfaceOf(ink2) ?? -1)) };
}
const lit = (now: { visible: boolean; opacity: number }) => now.visible && now.opacity === 1;
const painted = () => { const now = shown(); return now.field === 'worker' && now.headedField === 'worker' && lit(now.panel) && lit(now.race) && lit(now.headed); };
const currentOf = (...given: (Present | Extract<FromWorker, { type: 'racePresent' }> | undefined)[]) => {
  const generation = clockGeneration(clock), seek = clock.getSnapshot().seekSequence;
  return given.every(present => present?.generation === generation && present.seek === seek);
};
const current = () => currentOf(panelPresent(), headedPresent(), racePresent());
const readEmptied = () => emptied.seek === clock.getSnapshot().seekSequence && strict === null;

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

const landed = (hour: number) => {
  const now = shown();
  return current() && painted() && headedPresent()?.readHour === hour && now.listed === String(hour) && now.aces === 'empty' && now.noDoctrines === 'empty';
};

const openMenus = () => [...document.querySelectorAll<HTMLElement>('.context-menu')].filter(menu => menu.matches(':popover-open'));
function menuOpens() {
  const row = document.querySelector<HTMLElement>('#doctrine [data-track]');
  if (!row) return null;
  const opened: string[] = [];
  const tried = (how: string, event: Event) => {
    row.dispatchEvent(event);
    if (openMenus().length) opened.push(how);
    for (const menu of openMenus()) menu.hidePopover();
  };
  tried('pointer', new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 10, clientY: 10 }));
  tried('keyboard', new KeyboardEvent('keydown', { key: 'F10', shiftKey: true, bubbles: true, cancelable: true }));
  row.focus();
  return opened;
}

async function heldSeek(hour: number, settled: () => boolean, done: () => boolean = () => true) {
  let pending: (ReturnType<typeof shown> & { menus: string[] | null })[] = [];
  const seen = await watch(() => { native.hold = true; watchRemoval(); clock.seek(hour); }, async () => {
    await until(() => settled() && removalSeen());
    pending = [{ ...shown(), menus: menuOpens() }];
    native.hold = false; push();
    if (pending[0]!.listedRead === 'followed') await until(() => landed(hour) && strict === hour && done());
  });
  return { seen, pending: pending[0]! };
}

async function staleStep() {
  const hour = clock.getHour(), store = followStore(runtime.client, runtime.tracks[0]!.source, clock, track.axisOffset);
  const readings = (message: FromWorker): readonly (PageReading | undefined)[] =>
    message.type === 'structure' ? message.updates : message.type === 'inspection' ? [message.reading] : [];
  const carries = (message: FromWorker, type: 'structure' | 'inspection', hours: number) =>
    message.type === type && readings(message).some(reading => reading?.id === native.id && !!reading.panel && reading.hour === hours);
  const said = (type: 'structure' | 'inspection', hours: number) => gate.held.some(({ message }) => carries(message, type, hours));
  gate.holds = message => carries(message, 'structure', hour + 1) || carries(message, 'inspection', hour + 1);
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
  gate.holds = null;
  for (const { deliver } of gate.held.splice(0)) deliver();
  native.hold = false; push();
  if (taken.every(reply => reply.hour === null)) await until(() => landed(hour + 2) && strict === hour + 2);
  return { taken, pending, done: shown() };
}

async function raceHold(hour: number, gone: boolean) {
  const race = raceCanvas()!, from = clock.getHour();
  race.focus();
  race.dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true, cancelable: true }));
  await until(() => shown().raceLive.includes(`at ${from}`));
  const before = shown(), id = surfaceOf(race);
  gate.holds = message => message.type === 'racePresent' && message.id === id;
  const heldNow = () => gate.held.some(({ message }) => message.type === 'racePresent' && message.owner === owners.get(id ?? -1) &&
    message.seek === clock.getSnapshot().seekSequence && message.hour === hour);
  let pending = before;
  const seen = await watch(() => { clock.seek(hour); if (gone) raceReply.set(withoutGermany); }, async () => {
    await until(() => heldNow() && shown().race.busy);
    pending = shown();
    gate.holds = null;
    for (const { deliver } of gate.held.splice(0)) deliver();
    await until(() => { const now = shown(); return !now.race.busy && now.race.hour === hour && now.raceRanking !== ''; });
  });
  return { before, seen, pending, done: seen.at(-1)! };
}

export const fixture = {
  async mount() {
    pop.showPopover();
    createRoot(document.body.appendChild(document.createElement('div'))).render(<><Panel /><Race /><Headed />{createPortal(<Listed />, pop)}<Probe />
      <div style={{ display: 'flex', gap: '8px' }}><div id="aces" style={box}><AcesWidget {...aces} /></div>
        <div id="short-aces" style={box}><AcesWidget {...shortAces} /></div>
        <div id="doctrine" style={box}><DoctrineMasteryWidget {...doctrines} /></div>
        <div id="no-doctrines" style={box}><DoctrineMasteryWidget {...noDoctrines} /></div></div>
      <div id="arsenal-widget" style={{ width: '480px', height: '160px', overflow: 'hidden' }}><NuclearArsenalWidget {...arsenal} /></div></>);
    await until(() => followedSurface && listedRead === 'followed' && landed(23) && !!document.querySelector('#doctrine [data-track]'));
    raceCanvas()!.focus();
    raceCanvas()!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true, cancelable: true }));
    await until(() => !!shown().raceRanking && !!shown().raceLive);
    const focused = shown();
    document.querySelector<HTMLElement>('#doctrine [data-track]')!.focus();
    const menus = menuOpens();
    await until(() => shown().tip === dayLabel(23) && shown().described);
    return { ...shown(), menus, focused };
  },
  speed: (hoursPerSecond: number) => watch(() => clock.setSpeed(hoursPerSecond)),
  heldSeek: (hour: number) => heldSeek(hour, () => current() && readEmptied(), () => shown().tip === dayLabel(hour) && shown().warned.join() === `read at ${hour}`),
  heldRewind: (hour: number) => heldSeek(hour, () => currentOf(panelPresent(), racePresent()) && readEmptied()),
  step: () => watch(() => clock.step(1), () => until(() => landed(clock.getHour()))),
  async pastShortEnd() {
    clock.seek(SHORT_END);
    await until(() => landed(SHORT_END) && shown().shortAces === 'empty');
    const before = shown();
    clock.step(1);
    await until(() => landed(SHORT_END + 1));
    return { before, after: shown() };
  },
  staleStep,
  raceHold,
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
