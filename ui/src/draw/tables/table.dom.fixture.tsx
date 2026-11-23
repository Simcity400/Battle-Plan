import { createRoot } from 'react-dom/client';
import { flushSync } from 'react-dom';
import { PlaybackController } from '../../dashboard/playback.ts';
import { pipelineFor, type PipelineHost } from '../../dashboard/follow/pipeline.ts';
import { FollowedWidgetKey } from '../../dashboard/follow/store.ts';
import { packet, resetStamps, splitForLanes } from '../../dashboard/follow/split.ts';
import { answerBody } from '../../dashboard/follow/messages.fixture.ts';
import FamilyTable, { type FamilyColumn } from './FamilyTable.tsx';
import type { TableRowModel, TableValues } from './surface.ts';
import type { TableSettings } from './logic.ts';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import type { Live } from '../../charts/live';
import '../../styles.css';

const NATIVE = 3;
type Row = { tag: string; value?: bigint; reason?: string };

function message(hour: number, seek: number, answers: { id: number; hour: number; body: Uint8Array }[], reset = false) {
  const out = new Uint8Array(24 + answers.reduce((sum, a) => sum + 16 + a.body.length, 0)), view = new DataView(out.buffer);
  view.setUint32(0, 0x334c4f46, true); view.setUint32(4, NATIVE << 1 | (reset ? 1 : 0), true); view.setInt32(8, hour, true);
  view.setUint32(12, answers.length, true); view.setUint32(16, seek, true);
  let at = 24;
  for (const a of answers) { view.setUint32(at, a.id, true); view.setInt32(at + 4, a.hour, true); view.setUint32(at + 8, a.body.length, true); out.set(a.body, at + 16); at += 16 + a.body.length; }
  return out;
}
const table = (rows: Row[]) => answerBody({ keys: rows.map((_, at) => BigInt(at + 1)), columns: 1, values: rows.map(row => row.value ?? 0n),
  states: rows.map(row => row.reason === undefined ? 0 : 2), reasons: rows.map(row => row.reason ?? null), names: rows.map(row => row.tag) });

let ports: readonly MessagePort[] = [];
let seek = 0;
const stamp = resetStamps();
const send = (bytes: Uint8Array) => {
  const resets = stamp(bytes), split = splitForLanes(bytes, ports.length);
  if (split.kind !== 'failure') split.lanes.forEach((part, lane) => { if (part) ports[lane]!.postMessage(packet('follow', part, resets), [part.buffer]); });
};
const source = { id: 's', revision: 'r', path: 'p', metadata: {} } as unknown as DashboardSource;
const client = {
  available: true,
  openFollow: (opened: readonly MessagePort[]) => { ports = opened; return Promise.resolve(); },
  product: async (request: { product: string; params: Record<string, unknown> }) => {
    if (request.product === 'follow') return { data: { available: true, route: NATIVE } };
    return { data: { available: true } };
  },
} as unknown as DashboardClient;

const clock = new PlaybackController();
clock.setBounds({ firstHour: 0, lastHour: 1000 });
const workers = [0, 1].map(() => new Worker(new URL('../lane.worker.ts', import.meta.url), { type: 'module' }));
const host: PipelineHost = { lanes: workers, open: list => client.openFollow!(list), channel: () => new MessageChannel(), frameRate: () => 60, rate: () => 1 };
const pipeline = pipelineFor(clock, client, () => host);

const COLUMNS: FamilyColumn[] = [{ key: 'v', label: 'Value', priority: 0, numeric: true, as: 'plain', offered: ['plain'] }];
const SETTINGS: TableSettings = { density: 'comfortable', overflow: 'scroll', order: ['v'], columns: { v: { show: true, as: 'plain', width: 120 } }, sort: { column: 'v', direction: 'descending' } };
const ROWS: TableRowModel[] = ['GER', 'ITA'].map(tag => ({ key: tag, cells: { v: { valueKey: `${tag}\u0000s`, format: 'integer' } } }));
const still = <T,>(value: T): Live<T> => ({ read: () => value, subscribe: () => () => {} });
const NO_VALUES: Live<TableValues> = still({ get: () => undefined, missing: () => 'Not read yet' });

interface Mounted { key: string; pipe: number; sub: number; node: HTMLElement }
const mounted: Mounted[] = [];

async function mount(key: string) {
  const pipe = pipeline.acquire(`${key}|0`, { offset: 0, pin: null });
  const sub = pipeline.subscribe(pipe, {}, source, '1', 0, { product: 'series-state', params: { branchId: '1', table: key } }, 'followed')!;
  const node = document.createElement('div');
  node.style.cssText = 'width:320px;height:160px;display:flex;flex-direction:column';
  document.body.append(node);
  flushSync(() => createRoot(node).render(<FollowedWidgetKey.Provider value={key}>
    <FamilyTable label={key} clock={clock} offset={0} columns={COLUMNS} rows={ROWS} settings={SETTINGS} live={NO_VALUES} sources={still({ s: sub })}
      shownAt={0} onHover={() => {}} headerSlot={null} tooltipKeys={row => [`${row}\0s`]} />
  </FollowedWidgetKey.Provider>));
  const held = { key, pipe, sub, node };
  mounted.push(held);
  return held;
}

const stepTo = (hour: number) => { while (clock.getHour() < hour) clock.step(1); };
const settle = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms));
async function until(done: () => boolean, ms = 5000) { const end = performance.now() + ms; while (!done() && performance.now() < end) await settle(16); return done(); }

async function pixels(node: HTMLElement): Promise<string> {
  const canvas = node.querySelector<HTMLCanvasElement>('canvas.tf-canvas')!;
  const bitmap = await createImageBitmap(canvas), out = new OffscreenCanvas(bitmap.width, bitmap.height), ctx = out.getContext('2d')!;
  ctx.drawImage(bitmap, 0, 0);
  const data = ctx.getImageData(0, 0, bitmap.width, bitmap.height).data;
  let hash = 2166136261;
  for (let at = 0; at < data.length; at++) hash = Math.imul(hash ^ data[at]!, 16777619);
  return `${bitmap.width}x${bitmap.height}:${(hash >>> 0).toString(16)}`;
}

const tipOf = (held: Mounted) => {
  const tip = pipeline.view(held.pipe)?.tooltip as { hour: number; values: Record<string, unknown> } | null;
  return tip ? { hour: tip.hour, values: Object.values(tip.values).map(String) } : null;
};
const state = (held: Mounted) => {
  const view = pipeline.view(held.pipe)!;
  return { mode: view.mode, h: view.h, through: view.through, dim: view.dim,
    mirror: [...held.node.querySelectorAll('.table-accessibility tbody tr')].map(tr => [...tr.querySelectorAll('td')].map(td => td.textContent).join('|')),
    held: !!held.node.querySelector('[data-held]') };
};

declare global { interface Window { familyTableFixture: unknown } }
window.familyTableFixture = {
  async start() {
    const a = await mount('a');
    clock.seek(10);
    seek = clock.getSnapshot().seekSequence;
    await until(() => pipeline.status(a.sub).status === 'followed');
    send(message(10, seek, [{ id: a.sub, hour: 10, body: table([{ tag: 'GER', value: 10n }, { tag: 'ITA', value: 5n }]) }], true));
    const ok = await until(() => state(a).mode === 'picture' && !state(a).dim && state(a).mirror.length === 2);
    pipeline.inspect(a.pipe, 'row:GER');
    await until(() => pipeline.view(a.pipe)?.tooltip !== null);
    return { ok, state: state(a), pixels: await pixels(a.node), tooltip: tipOf(a) };
  },
  async change() {
    const a = mounted[0]!;
    send(message(11, seek, [{ id: a.sub, hour: 11, body: answerBody({ keys: [1n], columns: 1, values: [20n], changed: true }) }]));
    stepTo(11);
    await until(() => state(a).h === 11 && !state(a).dim);
    await until(() => (pipeline.view(a.pipe)?.tooltip as { values?: unknown } | null) !== null);
    const tooltip = tipOf(a);
    const hovered = pipeline.view(a.pipe)!.id;
    pipeline.inspect(a.pipe, null);
    await until(() => pipeline.view(a.pipe)!.id !== hovered);
    const b = await mount('b');
    await until(() => pipeline.status(b.sub).status === 'followed');
    send(message(11, seek, [{ id: b.sub, hour: 11, body: table([{ tag: 'GER', value: 20n }, { tag: 'ITA', value: 5n }]) }]));
    await until(() => state(b).mode === 'picture' && !state(b).dim && state(b).mirror.length === 2);
    await settle(100);
    return { a: state(a), b: state(b), changed: await pixels(a.node), fresh: await pixels(b.node), tooltip };
  },
  async limited() {
    const a = mounted[0]!;
    send(withLimit(message(14, seek, []), a.sub, 12));
    stepTo(14);
    await until(() => state(a).mode === 'picture');
    await settle(100);
    return state(a);
  },
  async missing() {
    const a = mounted[0]!;
    send(message(20, seek, [{ id: a.sub, hour: 20, body: table([{ tag: 'GER', value: 20n }, { tag: 'ITA', reason: 'ITA was not recorded at this hour' }]) }]));
    stepTo(20);
    await until(() => state(a).h === 20 && !state(a).dim);
    await settle(100);
    return state(a);
  },
};

function withLimit(bytes: Uint8Array, id: number, through: number) {
  const out = new Uint8Array(bytes.length + 16), view = new DataView(out.buffer);
  out.set(bytes);
  view.setUint32(12, view.getUint32(12, true) + 1, true);
  view.setUint32(bytes.length, id, true); view.setInt32(bytes.length + 4, through, true);
  return out;
}
