import { useRef } from 'react';
import { createRoot } from 'react-dom/client';
import { flushSync } from 'react-dom';
import { PlaybackController } from '../playback.ts';
import { packet, resetStamps, splitForLanes } from './split.ts';
import { followStore } from './store.ts';
import { laneCount } from './useFollowed.ts';
import { useFollow } from './useFollow.ts';
import { useRowTooltips } from '../../charts/useRowTooltips.ts';
import { tooltipStore } from '../../charts/tooltip.ts';
import { followQuery } from '../../test-support/gated.ts';
import type { DashboardClient, DashboardSource } from '../client';

const NATIVE = 3, HOURS = 2000, LANES = laneCount();
const valueAt = (hour: number) => 1000 + hour;

function answer(sub: number, hour: number): Uint8Array {
  const out = new Uint8Array(72), view = new DataView(out.buffer);
  view.setUint8(0, 1); view.setUint8(1, 1); view.setUint32(4, 1, true); view.setUint32(8, 1, true);
  view.setBigUint64(16, BigInt(sub), true); view.setBigInt64(32, BigInt(valueAt(hour)), true);
  view.setUint32(48, 0xffff_ffff, true); view.setUint32(56, 0xffff_ffff, true);
  return out;
}

function message(hour: number, seek: number, reset: boolean, subs: readonly number[]): Uint8Array {
  const answers = subs.map(sub => ({ sub, body: answer(sub, hour) }));
  const out = new Uint8Array(24 + answers.reduce((sum, a) => sum + 16 + a.body.length, 0)), view = new DataView(out.buffer);
  view.setUint32(0, 0x334c4f46, true); view.setUint32(4, NATIVE << 1 | (reset ? 1 : 0), true); view.setInt32(8, hour, true);
  view.setUint32(12, answers.length, true); view.setUint32(16, seek, true);
  let at = 24;
  for (const a of answers) { view.setUint32(at, a.sub, true); view.setInt32(at + 4, hour, true); view.setUint32(at + 8, a.body.length, true); out.set(a.body, at + 16); at += 16 + a.body.length; }
  return out;
}

let ports: readonly MessagePort[] = [];
let emitted = -1, seek = 0, through = Infinity, resetting = false;
const subs = new Set<number>();
const stamp = resetStamps();
const send = (bytes: Uint8Array) => {
  const resets = stamp(bytes), split = splitForLanes(bytes, LANES);
  if (split.kind !== 'failure') split.lanes.forEach((part, lane) => { if (part) ports[lane]!.postMessage(packet('follow', part, resets), [part.buffer]); });
};
const native = (to: number) => {
  if (!ports.length || !subs.size) return;
  while (emitted < Math.min(HOURS - 1, to, through)) { emitted++; send(message(emitted, seek, resetting, [...subs])); resetting = false; }
};

const source = { id: 's', revision: 'r', path: 'p', metadata: { branches: [{ id: '1', firstHour: 0, lastHour: HOURS - 1 }] } } as unknown as DashboardSource;
const client = {
  available: true,
  openFollow: (opened: readonly MessagePort[]) => { ports = opened; return Promise.resolve(); },
  product: async (request: { product: string; params: Record<string, unknown> }) => {
    if (request.product === 'follow') {
      const id = request.params.id as number;
      subs.add(id);
      queueMicrotask(() => { if (emitted >= 0) send(message(emitted, seek, false, [id])); });
      return { data: { available: true, route: NATIVE } };
    }
    if (request.product === 'follow-clock') {
      const told = request.params as { seek: number; hour: number };
      if (told.seek !== seek) { seek = told.seek; resetting = true; emitted = Math.floor(told.hour) - 1; native(Math.floor(told.hour)); }
      return { data: { available: true } };
    }
    return { data: { available: true } };
  },
} as unknown as DashboardClient;

const clock = new PlaybackController();
clock.setBounds({ firstHour: 0, lastHour: HOURS - 1 });
clock.onPublish((_previous, next) => native(next.hour));
const store = followStore(client, { source, axisOffset: 0 }, clock);
const query = followQuery({ product: 'series-state', params: { branchId: '1', widget: 0 } });
let menus = 0;

function Rows() {
  const target = useRef<HTMLDivElement>(null);
  const followed = useFollow(store, query, target, null);
  const tips = useRowTooltips<string>({ owner: 'rows', active: true, content: key => ({ title: `row ${key}`, rows: [] }),
    revision: String(followed.entry?.version ?? 0), onMenu: () => { menus++; } });
  const reading = followed.reading;
  return <div ref={target} id="followed" data-reading={reading?.state ?? 'none'} data-dim={reading?.state === 'covered' && reading.dim ? 'dim' : 'clear'}>
    <div id="row" {...tips.rowProps('a')}>a</div>
    {tips.hovered === 'a' && <span id={tips.descriptionId('a')} className="described">row a</span>}
  </div>;
}

const host = document.createElement('div');
host.style.cssText = 'width: 300px; height: 200px';
document.body.append(host);
const root = createRoot(host);

const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
const until = async (done: () => boolean, frames = 600) => { for (let at = 0; at < frames && !done(); at++) await frame(); return done(); };
const node = () => document.getElementById('followed')!;
const row = () => document.getElementById('row')!;

interface Shown { reading: string; dim: boolean; held: string | null; busy: string | null; tip: string | null; described: boolean; menus: number }
const shown = (): Shown => ({
  reading: node().dataset.reading ?? '', dim: node().dataset.dim === 'dim', held: node().getAttribute('data-held'), busy: node().getAttribute('aria-busy'),
  tip: tooltipStore.get()?.owner ?? null, described: !!node().querySelector('.described'), menus,
});

const hover = () => { const box = row().getBoundingClientRect(); row().dispatchEvent(new PointerEvent('pointermove', { bubbles: true, clientX: box.left + 2, clientY: box.top + 2 })); };
const leave = () => row().dispatchEvent(new PointerEvent('pointerout', { bubbles: true, relatedTarget: document.body }));
const menu = () => {
  row().dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true }));
  row().dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, key: 'ContextMenu' }));
};
const interact = async () => { hover(); await frame(); const hovered = shown(); menu(); await frame(); return { hovered, after: shown() }; };

declare global { interface Window { heldFixture: unknown } }
window.heldFixture = {
  async start() {
    flushSync(() => root.render(<Rows />));
    clock.seek(100);
    const ok = await until(() => shown().reading === 'covered' && !shown().dim);
    return { ok, shown: shown() };
  },
  async current() { const state = await interact(); leave(); await frame(); return state; },
  async step() {
    through = clock.getHour();
    clock.step(1);
    const ok = await until(() => shown().dim);
    hover(); await frame();
    const hovered = shown();
    menu(); await frame();
    row().focus(); await frame();
    const focused = shown();
    return { ok, hovered, focused, after: shown() };
  },
  async release() {
    row().blur(); leave(); await frame();
    through = Infinity;
    native(clock.getHour());
    const ok = await until(() => !shown().dim && shown().held === null);
    const state = await interact();
    return { ok, ...state };
  },
  async heldRead(to: number) {
    leave(); await frame();
    through = clock.getHour();
    clock.seek(to);
    await frame();
    const reading = shown();
    through = Infinity;
    native(to);
    await until(() => shown().reading === 'covered');
    return { reading, settled: shown() };
  },
};
