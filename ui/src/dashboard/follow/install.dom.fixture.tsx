import { PlaybackController } from '../playback.ts';
import { Pipeline, type PipelineHost } from './pipeline.ts';
import { packet, resetStamps, splitForLanes } from './split.ts';
import type { DashboardClient, DashboardSource } from '../client';

const NATIVE = 3, HOURS = 6000, LANES = 2;
const valueAt = (sub: number, hour: number) => sub * 100_000 + Math.floor(hour / (11 + sub)) * (11 + sub);
const changesAt = (sub: number, hour: number) => hour % (11 + sub) === 0;

function answer(sub: number, hour: number): Uint8Array {
  const out = new Uint8Array(72), view = new DataView(out.buffer);
  view.setUint8(0, 1); view.setUint8(1, 1); view.setUint32(4, 1, true); view.setUint32(8, 1, true);
  view.setBigUint64(16, BigInt(sub), true); view.setBigInt64(32, BigInt(valueAt(sub, hour)), true);
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
let emitted = -1, seek = 0;
const subs = new Set<number>();
const stamp = resetStamps();
const send = (bytes: Uint8Array) => {
  const resets = stamp(bytes), split = splitForLanes(bytes, LANES);
  if (split.kind !== 'failure') split.lanes.forEach((part, lane) => { if (part) ports[lane]!.postMessage(packet('follow', part, resets), [part.buffer]); });
};
const native = (to: number) => {
  while (emitted < Math.min(HOURS - 1, to)) { emitted++; const changed = [...subs].filter(sub => changesAt(sub, emitted)); send(message(emitted, seek, false, changed)); }
};

const source = { id: 's', revision: 'r', path: 'p', metadata: {} } as unknown as DashboardSource;
const client = {
  available: true,
  openFollow: (opened: readonly MessagePort[]) => { ports = opened; return Promise.resolve(); },
  product: async (request: { product: string; params: Record<string, unknown> }) => {
    if (request.product === 'follow') {
      const id = request.params.id as number;
      subs.add(id);
      queueMicrotask(() => send(message(Math.max(0, emitted), seek, false, [id])));
      return { data: { available: true, route: NATIVE } };
    }
    if (request.product === 'follow-clock') {
      const told = request.params as { seek: number; hour: number };
      if (told.seek !== seek) { seek = told.seek; emitted = Math.floor(told.hour); send(message(emitted, seek, true, [...subs])); }
      return { data: { available: true } };
    }
    return { data: { available: true } };
  },
} as unknown as DashboardClient;

const clock = new PlaybackController();
clock.setBounds({ firstHour: 0, lastHour: HOURS - 1 });
const workers = Array.from({ length: LANES }, () => new Worker(new URL('../../draw/lane.worker.ts', import.meta.url), { type: 'module' }));
const host: PipelineHost = { lanes: workers, open: list => client.openFollow!(list), channel: () => new MessageChannel(), frameRate: () => 60, rate: () => 1 };
const tap: string[] = [];
workers.forEach((worker, lane) => {
  const post = worker.postMessage.bind(worker);
  worker.postMessage = ((message: { type: string; sub?: number; widget?: number }, transfer?: Transferable[]) => {
    if (message.type !== 'frame') tap.push(`>${lane} ${message.type} ${message.sub ?? message.widget ?? ''}`);
    post(message, transfer ?? []);
  }) as typeof worker.postMessage;
  worker.addEventListener('message', event => { const data = event.data as { type: string; entries?: { widget: number }[]; reason?: string }; tap.push(`<${lane} ${data.type} ${data.entries?.map(e => e.widget).join(',') ?? data.reason ?? ''}`); });
});
const pipeline = new Pipeline(clock, client, host);
clock.onPublish((_previous, next) => native(next.hour + (next.playing ? 60 : 0)));

interface Shown { widget: number; text: string; dim: boolean; mode: string; target: number }
const widgets: { id: number; sub: number; node: HTMLElement }[] = [];
const frames: { at: number; shown: Shown[]; faults: string[] }[] = [];

function mount(count: number) {
  for (let at = 0; at < count; at++) {
    const node = document.createElement('div');
    document.body.append(node);
    const id = pipeline.mount({ offset: 0, pin: null, bounds: [0, HOURS - 1] });
    const sub = pipeline.subscribe(id, {}, source, '1', 0, { product: 'series-state', params: { branchId: '1', widget: at % 3 } }, 'followed')!;
    const render = () => {
      const view = pipeline.view(id)!;
      const body = view.mode === 'picture' ? view.fields?.[`answer:${sub}`] as Uint8Array | undefined : undefined;
      const value = body ? new DataView(body.buffer, body.byteOffset).getBigInt64(32, true) : null;
      node.textContent = view.mode === 'picture' ? `${view.id}|${view.h}|${value}` : view.mode;
      node.toggleAttribute('data-dim', view.dim);
    };
    pipeline.onInstall(id, render);
    pipeline.onDate(id, render);
    widgets.push({ id, sub, node });
  }
}

function sample() {
  const faults: string[] = [], shown: Shown[] = [];
  for (const { id, sub, node } of widgets) {
    const view = pipeline.view(id)!, text = node.textContent ?? '';
    shown.push({ widget: id, text, dim: view.dim, mode: view.mode, target: view.target });
    if (view.mode !== 'picture') continue;
    const [entry, h, value] = text.split('|');
    if (Number(entry) !== view.id || Number(h) !== view.h) faults.push(`widget ${id}: DOM shows ${text}, installed ${view.id} at ${view.h}`);
    if (view.h! > view.target || view.through! > view.target) faults.push(`widget ${id}: ahead, h ${view.h} through ${view.through} at ${view.target}`);
    if (Number(value) !== valueAt(sub, view.h!)) faults.push(`widget ${id}: value ${value} is not the recording's ${valueAt(sub, view.h!)} at ${view.h}`);
    if (node.hasAttribute('data-dim') !== view.dim) faults.push(`widget ${id}: dim on the page ${node.hasAttribute('data-dim')}, installed ${view.dim}`);
  }
  frames.push({ at: performance.now(), shown, faults });
}

const watch = () => { sample(); requestAnimationFrame(watch); };
const settle = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms));
const until = async (done: () => boolean, ms = 3000) => { const end = performance.now() + ms; while (!done() && performance.now() < end) await settle(16); return done(); };

declare global { interface Window { installFixture: unknown } }
window.installFixture = {
  async start(count: number) {
    mount(count); clock.seek(100); requestAnimationFrame(watch);
    const ok = await until(() => widgets.every(w => pipeline.view(w.id)?.mode === 'picture'));
    return ok || JSON.stringify({ modes: widgets.map(w => `${w.id}/${w.sub}/${pipeline.lane(w.id)}:${pipeline.view(w.id)?.mode}`), rejected: pipeline.page.stats.rejected, installs: pipeline.page.stats.installs, emitted, seek, tap: tap.slice(0, 40) });
  },
  async play(speed: number, ms: number) {
    frames.length = 0; clock.setSpeed(speed); clock.setPlaying(true); await settle(ms);
    return { frames: frames.length, faults: frames.flatMap(f => f.faults).slice(0, 10), installed: frames.reduce((sum, f) => sum + f.shown.filter(s => s.mode === 'picture').length, 0) };
  },
  async pause() {
    clock.setPlaying(false); await settle(200);
    const last = frames.at(-1)!;
    return { faults: frames.flatMap(f => f.faults).slice(0, 10), shown: last.shown };
  },
  async rewind(to: number) {
    frames.length = 0; clock.seek(to); sample(); const first = frames[0]!;
    await until(() => widgets.every(w => pipeline.view(w.id)?.mode === 'picture' && pipeline.view(w.id)!.h! <= to));
    sample();
    return { first: first.shown, faults: frames.flatMap(f => f.faults).slice(0, 10), settled: frames.at(-1)!.shown,
      debug: JSON.stringify({ modes: widgets.map(w => `${w.id}:${pipeline.view(w.id)?.mode}:${pipeline.view(w.id)?.h}`), rejected: pipeline.page.stats.rejected, emitted, seek, tap: tap.slice(-30) }) };
  },
  async speedChange(speed: number) {
    clock.setPlaying(true); await settle(300); frames.length = 0; clock.setSpeed(speed); await settle(300); clock.setPlaying(false); await settle(100);
    return { blanks: frames.filter(f => f.shown.some(s => s.mode !== 'picture')).length, faults: frames.flatMap(f => f.faults).slice(0, 10) };
  },
  async pin(widget: number, hour: number) {
    const held = widgets[widget]!;
    pipeline.retime(held.id, { pin: hour });
    sample();
    const now = frames.at(-1)!.shown.find(s => s.widget === held.id)!;
    await until(() => pipeline.view(held.id)?.mode === 'picture');
    return { immediate: now, settled: pipeline.view(held.id) };
  },
};
