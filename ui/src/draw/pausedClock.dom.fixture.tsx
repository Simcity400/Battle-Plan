import { useCallback, useLayoutEffect, useRef } from 'react';
import { createRoot } from 'react-dom/client';
import '../styles.css';
import { PlaybackController } from '../dashboard/playback';
import { useDrawnPanel } from '../dashboard/DrawnPanel';
import { panelFeed } from '../dashboard/panelFeed';
import type { PanelFeed } from '../dashboard/panelFeed';
import { useHeadProduct, withHeld } from '../dashboard/headProduct';
import type { WidgetRuntime } from '../dashboard/definitions';
import type { WidgetSpec } from '../dashboard/model';
import { clockGeneration, onDraw } from '../dashboard/pulled';
import { FollowDecoder } from '../dashboard/follow/decode';
import { datedPanel } from '../dashboard/follow/messages.fixture';
import DatedArsenal from '../charts/race/DatedArsenal';
import { readTokens } from '../charts/tokens';
import { FrameEngine } from '../engine/frame';
import { arsenalNotice, arsenalStructure, validateNuclearArsenal } from '../widgets/nuclear-arsenal/nuclearArsenalData';
import type { NuclearArsenalReply } from '../generated/dashboard';
import type { Present } from './notifications';
import type { FromWorker } from './kit';

document.documentElement.dataset.motion = 'reduced';
const HOURS = 100_000;
const reply: NuclearArsenalReply = { types: [{ ordinal: 0, token: 'nuke', name: 'Nuclear bomb' }],
  countries: [{ tag: 'GER', countryId: 1, total: 3, counts: [{ ordinal: 0, value: 3, ready: 3 }] }], missing: null, warnings: [] };
const arsenalAt = (hour: number): NuclearArsenalReply => ({ ...reply,
  countries: [{ tag: 'GER', countryId: 1, total: hour, counts: [{ ordinal: 0, value: hour, ready: hour }] }] });
const date = { since: 0, through: HOURS - 1 };
const feed: PanelFeed = { view: 'nuclear-arsenal', offset: 0, structure: arsenalStructure(reply), reply, date };
const clock = new PlaybackController(new FrameEngine({ request() {} }));
clock.setBounds({ firstHour: 0, lastHour: HOURS - 1 });
clock.seek(23);

const ports: MessagePort[] = [];
const decoder = new FollowDecoder();
const native = { id: -1, hold: false, asked: null as { hour: number; seek: number } | null, pushed: '', asks: 0 };
function push() {
  const asked = native.asked;
  if (!asked || native.id < 0 || native.pushed === `${asked.seek}:${asked.hour}`) return;
  native.pushed = `${asked.seek}:${asked.hour}`;
  const item = decoder.item(datedPanel(native.id, asked.hour, arsenalAt(asked.hour), asked.seek).buffer);
  for (const port of ports) port.postMessage([item]);
}
const answer = (product: string, params: Record<string, unknown>): unknown => {
  if (product === 'follow') { native.id = (params as { id: number }).id; return { available: true, route: 9 }; }
  if (product === 'follow-clock') {
    const { playing, hour, seek } = params as { playing: boolean; hour: number; seek: number };
    if (!playing) { native.asked = { hour: Math.floor(hour), seek }; if (!native.hold) push(); }
    return null;
  }
  if (product === 'unfollow') return null;
  if (product === 'nuclear-arsenal') { native.asks++; return arsenalAt((params as { hour: number }).hour); }
  throw new Error(`${product} is not served here`);
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
const headed = { id: 'headed', kind: 'nuclear-arsenal', trackIds: ['main'], countries: { kind: 'tags', tags: ['GER'] }, time: { mode: 'follow' }, options: {} } as unknown as WidgetSpec;
const NO_PARAMS = {};

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
const panelId = () => Math.min(...presents.keys()), headedId = () => presents.size > 1 ? Math.max(...presents.keys()) : -1;
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

function Listed() {
  const head = useHeadProduct(runtime, headed, 'nuclear-arsenal', NO_PARAMS, { active: true, validate: validateNuclearArsenal, structure: arsenalStructure });
  useLayoutEffect(() => { listedRead = head.read; wake(); });
  if (!head.data) return <p>Loading the arsenal…</p>;
  return <p id="listed" ref={head.root}>{head.data.countries[0]?.total}</p>;
}

function Race() {
  return <div id="race" style={{ width: '480px', height: '160px', display: 'flex' }}>
    <DatedArsenal label="Nuclear arsenal" items={[]} slots={5} name={key => key} active clock={clock}
      model={{ offset: 0, reply, date, top: 10, tags: ['GER'], split: false, tokens: readTokens(), slots: 5, overlay: true,
        colours: new Map([['GER', '#808080']]), looks: new Map([['GER', { name: 'Germany', color: '#808080' }]]) }} />
  </div>;
}

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
function shown() {
  const ink1 = inkOf('panel'), ink2 = inkOf('headed');
  const race = document.querySelector<HTMLCanvasElement>('#race canvas');
  return { field: ink('panel'), headedField: ink('headed'), mounts, hour: clock.getHour(), strict, asks: native.asks,
    listed: document.querySelector('#listed')?.textContent ?? null,
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

async function heldSeek(hour: number, settled: () => boolean) {
  let pending: ReturnType<typeof shown>[] = [];
  const seen = await watch(() => { native.hold = true; clock.seek(hour); }, async () => {
    await until(settled);
    pending = [shown()];
    native.hold = false; push();
    await until(() => current() && painted() && headedPresent()?.readHour === hour && shown().listed === String(hour));
  });
  return { seen, pending: pending[0]! };
}

export const fixture = {
  async mount() {
    createRoot(document.body.appendChild(document.createElement('div'))).render(<><Panel /><Race /><Headed /><Listed /></>);
    await until(() => followedSurface && listedRead === 'followed' && shown().listed === '23' && current() && painted());
    return shown();
  },
  speed: (hoursPerSecond: number) => watch(() => clock.setSpeed(hoursPerSecond)),
  heldSeek: (hour: number) => heldSeek(hour, () => current() && readEmptied()),
  heldRewind: (hour: number) => heldSeek(hour, () => currentOf(panelPresent(), racePresent) && readEmptied()),
  step: () => watch(() => clock.step(1), () => until(() => current() && painted())),
  async playFor(milliseconds: number) {
    const start = performance.now();
    at(start, () => clock.setPlaying(true));
    await until(current);
    at(start + milliseconds, () => clock.setPlaying(false));
    await until(() => current() && painted());
    return shown();
  },
};
(window as unknown as { pausedClock: typeof fixture }).pausedClock = fixture;
