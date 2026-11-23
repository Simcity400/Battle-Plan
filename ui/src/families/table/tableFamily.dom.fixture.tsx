import { Profiler } from 'react';
import { createRoot } from 'react-dom/client';
import { flushSync } from 'react-dom';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { pipelineFor, type PipelineHost } from '../../dashboard/follow/pipeline.ts';
import { FollowedWidgetKey } from '../../dashboard/follow/store.ts';
import { answerBody } from '../../dashboard/follow/messages.fixture.ts';
import { packet, resetStamps, splitForLanes } from '../../dashboard/follow/split.ts';
import type { WidgetSpec } from '../../dashboard/model';
import { PlaybackController } from '../../dashboard/playback.ts';
import { tableSettingsOf } from './settings.ts';
import { SCOREBOARD_FIXTURE_COLUMNS, scoreboardColumnsSpec, scoreboardProjection } from './tableFamilyDomShared.ts';
import FamilyTableHost from './FamilyTableHost.tsx';
import '../../styles.css';

const NATIVE = 7;

function followMessage(hour: number, seek: number, answers: { id: number; hour: number; body: Uint8Array }[], reset = false) {
  const out = new Uint8Array(24 + answers.reduce((sum, a) => sum + 16 + a.body.length, 0));
  const view = new DataView(out.buffer);
  view.setUint32(0, 0x334c4f46, true);
  view.setUint32(4, (NATIVE << 1) | (reset ? 1 : 0), true);
  view.setInt32(8, hour, true);
  view.setUint32(12, answers.length, true);
  view.setUint32(16, seek, true);
  let at = 24;
  for (const a of answers) {
    view.setUint32(at, a.id, true);
    view.setInt32(at + 4, a.hour, true);
    view.setUint32(at + 8, a.body.length, true);
    out.set(a.body, at + 16);
    at += 16 + a.body.length;
  }
  return out;
}

let ports: readonly MessagePort[] = [];
let seek = 0;
const stamp = resetStamps();
const send = (bytes: Uint8Array) => {
  const resets = stamp(bytes);
  const split = splitForLanes(bytes, ports.length);
  if (split.kind !== 'failure') {
    split.lanes.forEach((part, lane) => {
      if (part) ports[lane]!.postMessage(packet('follow', part, resets), [part.buffer]);
    });
  }
};

const source = {
  id: 'dom-test',
  revision: 'r1',
  path: 'p',
  metadata: { branches: [{ id: '1', firstHour: 0, lastHour: 2000 }] },
} as unknown as DashboardSource;

const client = {
  available: true,
  openFollow: (opened: readonly MessagePort[]) => { ports = opened; return Promise.resolve(); },
  product: async (request: { product: string }) => {
    if (request.product === 'follow') return { data: { available: true, route: NATIVE } };
    return { data: { available: true } };
  },
} as unknown as DashboardClient;

const clock = new PlaybackController();
clock.setBounds({ firstHour: 0, lastHour: 2000 });
const workers = [0, 1].map(() => new Worker(new URL('../../draw/lane.worker.ts', import.meta.url), { type: 'module' }));
const host: PipelineHost = {
  lanes: workers,
  open: list => client.openFollow!(list),
  channel: () => new MessageChannel(),
  frameRate: () => 60,
  rate: () => 1,
};
const pipeline = pipelineFor(clock, client, () => host);

const runtime = {
  client,
  clock,
  tracks: [{ id: 't1', source, branch: { id: '1', firstHour: 0, lastHour: 2000 }, axisOffset: 0 }],
} as unknown as WidgetRuntime;

const widget: WidgetSpec = {
  id: 'w-scoreboard',
  kind: 'scoreboard',
  trackIds: ['t1'],
  time: { mode: 'follow' },
  countries: { kind: 'all' },
  options: {},
  layout: { x: 0, y: 0, w: 12, h: 12 },
} as unknown as WidgetSpec;

const LOOKS = {
  GER: [{ from: 0, name: 'Germany' }],
  FRA: [{ from: 0, name: 'France' }],
  ITA: [{ from: 0, name: 'Italy' }],
  ENG: [{ from: 0, name: 'United Kingdom' }],
};

const SPECS = scoreboardColumnsSpec(SCOREBOARD_FIXTURE_COLUMNS, null);
const SETTINGS = {
  ...tableSettingsOf(undefined, SPECS, { density: 'comfortable', overflow: 'tooltip', sort: { column: 'factories', direction: 'descending' } }),
  order: SPECS.map(s => s.key),
  sort: { column: 'factories', direction: 'descending' as const },
};

const OPTIONS = {
  shown: SCOREBOARD_FIXTURE_COLUMNS,
  candidates: ['GER', 'FRA', 'ITA', 'ENG'],
  top: null,
  rankMarkers: false,
  looks: LOOKS,
};

const SUBSCRIPTIONS = [
  { name: 'factories', source, branch: '1', offset: 0, query: { product: 'series-state', params: { branchId: '1', col: 'factories' } }, channel: 'followed' as const },
  { name: 'casualties', source, branch: '1', offset: 0, query: { product: 'series-state', params: { branchId: '1', col: 'casualties' } }, channel: 'followed' as const },
];

let commits = 0;
let activated: string[] = [];
let hostNode: HTMLDivElement | null = null;
let subIds: Record<string, number> = {};
let pipeId = -1;

const waitTick = () => new Promise<void>(resolve => setTimeout(resolve, 16));
async function until(predicate: () => boolean, limitMs = 6000) {
  const deadline = performance.now() + limitMs;
  while (!predicate() && performance.now() < deadline) await waitTick();
  return predicate();
}

function measureLayoutReadsDuring<T>(fn: () => T): { result: T; reads: number } {
  let reads = 0;
  const origGetRect = Element.prototype.getBoundingClientRect;
  const origComputed = window.getComputedStyle;
  const clientWidthDesc = Object.getOwnPropertyDescriptor(Element.prototype, 'clientWidth')!;
  const clientHeightDesc = Object.getOwnPropertyDescriptor(Element.prototype, 'clientHeight')!;
  const offsetWidthDesc = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'offsetWidth')!;
  const offsetHeightDesc = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'offsetHeight')!;

  Element.prototype.getBoundingClientRect = function (this: Element) { reads++; return origGetRect.call(this); };
  window.getComputedStyle = function (...args: Parameters<typeof window.getComputedStyle>) { reads++; return origComputed.apply(window, args); };
  Object.defineProperty(Element.prototype, 'clientWidth', { configurable: true, get() { reads++; return clientWidthDesc.get!.call(this); } });
  Object.defineProperty(Element.prototype, 'clientHeight', { configurable: true, get() { reads++; return clientHeightDesc.get!.call(this); } });
  Object.defineProperty(HTMLElement.prototype, 'offsetWidth', { configurable: true, get() { reads++; return offsetWidthDesc.get!.call(this); } });
  Object.defineProperty(HTMLElement.prototype, 'offsetHeight', { configurable: true, get() { reads++; return offsetHeightDesc.get!.call(this); } });

  try {
    const result = fn();
    return { result, reads };
  } finally {
    Element.prototype.getBoundingClientRect = origGetRect;
    window.getComputedStyle = origComputed;
    Object.defineProperty(Element.prototype, 'clientWidth', clientWidthDesc);
    Object.defineProperty(Element.prototype, 'clientHeight', clientHeightDesc);
    Object.defineProperty(HTMLElement.prototype, 'offsetWidth', offsetWidthDesc);
    Object.defineProperty(HTMLElement.prototype, 'offsetHeight', offsetHeightDesc);
  }
}

function sendHour(hour: number, fac: [bigint, bigint, bigint, bigint], cas: [bigint, bigint, bigint, bigint], reset = false) {
  const names = ['GER', 'FRA', 'ITA', 'ENG'];
  const keys = [1n, 2n, 3n, 4n];
  const facBody = answerBody({
    keys,
    columns: 1,
    names,
    values: [...fac],
    states: [0, 0, 0, fac[3] < 0n ? 2 : 0],
    reasons: [null, null, null, fac[3] < 0n ? 'ENG factories not recorded' : null],
  });
  const casBody = answerBody({
    keys,
    columns: 1,
    names,
    values: [...cas],
    states: [0, 0, 0, 0],
  });
  send(followMessage(hour, seek, [
    { id: subIds.factories!, hour, body: facBody },
    { id: subIds.casualties!, hour, body: casBody },
  ], reset));
}

declare global {
  interface Window {
    tableFamilyDom: unknown;
  }
}

window.tableFamilyDom = {
  async mount() {
    commits = 0;
    activated = [];
    hostNode = document.createElement('div');
    hostNode.style.cssText = 'width:460px;height:260px;display:flex;flex-direction:column;';
    document.body.append(hostNode);
    const root = createRoot(hostNode);
    flushSync(() => root.render(
      <FollowedWidgetKey.Provider value="dom-scoreboard">
        <Profiler id="table-family" onRender={() => { commits++; }}>
          <FamilyTableHost
            declaration={scoreboardProjection}
            options={OPTIONS}
            label="Scoreboard"
            subject="scoreboard"
            widget={widget}
            runtime={runtime}
            active
            columns={SPECS}
            settings={SETTINGS}
            subscriptions={SUBSCRIPTIONS}
            offset={0}
            pin={null}
            bounds={[0, 2000]}
            onActivate={key => { activated.push(key); }}
          />
        </Profiler>
      </FollowedWidgetKey.Provider>,
    ));

    const ready = await until(() => {
      const canvas = hostNode?.querySelector<HTMLCanvasElement>('canvas.tf-canvas');
      if (!canvas?.dataset.placement) return false;
      pipeId = Number(canvas.dataset.placement);
      return pipeId > 0;
    });
    if (!ready) throw new Error('Table canvas did not mount');

    const internal = pipeline as unknown as { widgets: Map<number, { subs: Map<object, number> }> };
    const subList = [...(internal.widgets.get(pipeId)?.subs.values() ?? [])];
    subIds = { factories: subList[0]!, casualties: subList[1]! };
    await until(() => pipeline.status(subIds.factories!).status === 'followed' && pipeline.status(subIds.casualties!).status === 'followed');

    clock.seek(10);
    seek = clock.getSnapshot().seekSequence;
    sendHour(10, [50n, 40n, 30n, -1n], [0n, 100n, 200n, 0n], true);
    const painted = await until(() => {
      const view = pipeline.view(pipeId);
      return view?.mode === 'picture' && !view.dim && (hostNode?.querySelectorAll('.table-accessibility tbody tr').length ?? 0) === 4;
    });
    return { painted, initialRows: [...hostNode.querySelectorAll('.table-accessibility tbody tr')].map(tr => tr.getAttribute('data-row-key')) };
  },

  async playAndCount(steps: number) {
    commits = 0;
    let totalInstallReads = 0;
    let installs = 0;
    const internal = pipeline as unknown as { page: { frame(): void }; flush(): void };
    const origFlush = internal.flush.bind(pipeline);
    internal.flush = () => {
      const { reads } = measureLayoutReadsDuring(() => origFlush());
      totalInstallReads += reads;
    };
    const stopInstall = pipeline.onInstall(pipeId, () => { installs++; });

    try {
      for (let i = 1; i <= steps; i++) {
        const hour = 10 + i;
        const swap = i % 2 === 1;
        sendHour(hour, swap ? [35n, 60n, 30n, -1n] : [70n, 40n, 30n, -1n], [BigInt(i * 10), BigInt(i * 20), 0n, 0n]);
        clock.step(1);
        await until(() => pipeline.view(pipeId)?.h === hour && !pipeline.view(pipeId)?.dim);
      }
    } finally {
      internal.flush = origFlush;
      stopInstall();
    }

    const view = pipeline.view(pipeId);
    const mirrorRows = [...(hostNode?.querySelectorAll('.table-accessibility tbody tr') ?? [])].map(tr => ({
      key: tr.getAttribute('data-row-key'),
      cells: [...tr.querySelectorAll('td')].map(td => td.textContent),
    }));
    return { commits, totalInstallReads, installs, finalHour: view?.h ?? null, mirrorRows };
  },

  async verifyRepaintAndHeld() {
    const hashCanvas = async () => {
      const canvas = hostNode!.querySelector<HTMLCanvasElement>('canvas.tf-canvas')!;
      const bitmap = await createImageBitmap(canvas);
      const out = new OffscreenCanvas(bitmap.width, bitmap.height);
      const ctx = out.getContext('2d')!;
      ctx.drawImage(bitmap, 0, 0);
      const data = ctx.getImageData(0, 0, bitmap.width, bitmap.height).data;
      let hash = 2166136261;
      for (let i = 0; i < data.length; i++) hash = Math.imul(hash ^ data[i]!, 16777619);
      return `${bitmap.width}x${bitmap.height}:${(hash >>> 0).toString(16)}`;
    };
    const beforeHash = await hashCanvas();
    sendHour(23, [75n, 40n, 30n, -1n], [100n, 200n, 0n, 0n]);
    clock.step(1);
    await until(() => pipeline.view(pipeId)?.h === 23 && !pipeline.view(pipeId)?.dim);
    const afterHash = await hashCanvas();
    for (let s = 0; s < 5; s++) clock.step(1);
    await until(() => pipeline.view(pipeId)?.dim === true, 3000);
    const dimView = pipeline.view(pipeId);
    const held = !!hostNode!.querySelector('[data-held]');
    sendHour(28, [70n, 40n, 30n, -1n], [100n, 200n, 0n, 0n]);
    await until(() => pipeline.view(pipeId)?.h === 28 && !pipeline.view(pipeId)?.dim, 3000);
    return { pixelsChanged: beforeHash !== afterHash, dim: !!dimView?.dim, held, through: dimView?.through ?? null };
  },

  bodyBox() {
    const box = hostNode!.querySelector<HTMLDivElement>('.tf-body')!.getBoundingClientRect();
    return { left: box.left, top: box.top, width: box.width, height: box.height };
  },

  async waitInspected() {
    return await until(() => pipeline.view(pipeId)?.fields?.tooltip !== undefined);
  },

  async resortUnderPointer() {
    activated = [];
    clock.seek(30);
    seek = clock.getSnapshot().seekSequence;
    sendHour(30, [20n, 90n, 80n, -1n], [1n, 2n, 3n, 4n], true);
    await until(() => pipeline.view(pipeId)?.h === 30 && !pipeline.view(pipeId)?.dim);
    return [...(hostNode?.querySelectorAll('.table-accessibility tbody tr') ?? [])].map(tr => tr.getAttribute('data-row-key'));
  },

  readActivated() {
    return [...activated];
  },

  async testKeyboardAndNarrow() {
    activated = [];
    const firstRow = hostNode!.querySelector<HTMLTableRowElement>('.table-accessibility tbody tr')!;
    firstRow.focus();
    firstRow.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
    await waitTick();
    const afterDown = (document.activeElement as HTMLElement | null)?.getAttribute('data-row-key') ?? null;
    document.activeElement?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    await waitTick();
    document.activeElement?.dispatchEvent(new KeyboardEvent('keydown', { key: 'p', bubbles: true }));
    await waitTick();
    const liveStatus = hostNode!.querySelector('.tf-live-region')?.textContent ?? '';

    hostNode!.style.width = '210px';
    await until(() => !!hostNode!.querySelector('.tf-more-cols'), 3000);
    const moreBtn = hostNode!.querySelector<HTMLButtonElement>('.tf-more-cols');
    const moreText = moreBtn?.textContent ?? '';
    moreBtn?.click();
    await waitTick();
    const menuOpen = !!document.querySelector('.tf-chooser');
    return { afterDown, activated: [...activated], liveStatus, moreText, menuOpen };
  },
};
