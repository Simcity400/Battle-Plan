import { Profiler } from 'react';
import { createRoot } from 'react-dom/client';
import { flushSync } from 'react-dom';
import { tooltipStore } from '../../charts/tooltip.ts';
import TooltipLayer from '../../charts/TooltipLayer.tsx';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { pipelineFor, type LaneWorker, type PipelineHost } from '../../dashboard/follow/pipeline.ts';
import { FollowedWidgetKey } from '../../dashboard/follow/store.ts';
import { answerBody } from '../../dashboard/follow/messages.fixture.ts';
import { packet, resetStamps, splitForLanes } from '../../dashboard/follow/split.ts';
import type { WidgetSpec } from '../../dashboard/model';
import { PlaybackController } from '../../dashboard/playback.ts';
import type { FromLane, ToLane } from '../../draw/laneMessages';
import { laneDispatch } from '../../draw/laneDispatch.ts';
import type { CadenceTimers } from '../cadence.ts';
import { tableLane } from './laneView.ts';
import { tableSettingsOf } from './settings.ts';
import { SCOREBOARD_FIXTURE_COLUMNS, scoreboardColumnsSpec, scoreboardProjection } from './tableFamilyDom.fixture.ts';
import FamilyHost from '../host/FamilyHost.tsx';
import { DetailWindowHost } from '../host/DetailWindowHost.tsx';
import { mirrorRowSelector } from '../host/mirror.ts';
import { createLaneView } from '../laneRuntime.ts';
import { tableHostAdapter } from './hostAdapter.ts';
import TableChrome from './TableChrome.tsx';
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

let cadenceHold = false;
let frozenNow = 0;
let timerIdSeq = 1;
let timerFiredCount = 0;
const heldTimers = new Map<number, () => void>();
const liveTimers = new Map<number, { readonly fn: () => void; readonly handle: ReturnType<typeof setTimeout> }>();
const controlledClock = () => (cadenceHold ? frozenNow : performance.now());
const controlledTimers: CadenceTimers = {
  set(fn, ms) {
    const id = timerIdSeq++;
    if (cadenceHold) heldTimers.set(id, fn);
    else liveTimers.set(id, { fn, handle: setTimeout(() => { liveTimers.delete(id); timerFiredCount++; fn(); }, ms) });
    return id as unknown as ReturnType<typeof setTimeout>;
  },
  clear(id) {
    const num = id as unknown as number;
    if (heldTimers.delete(num)) return;
    const live = liveTimers.get(num);
    if (live) clearTimeout(live.handle);
    liveTimers.delete(num);
  },
};

function holdCadence(now: number): void {
  frozenNow = now;
  cadenceHold = true;
  heldTimers.clear();
  for (const [id, live] of liveTimers) {
    clearTimeout(live.handle);
    heldTimers.set(id, live.fn);
  }
  liveTimers.clear();
}

function createControlledLane(): LaneWorker {
  const worker: LaneWorker = {
    onmessage: null,
    postMessage(message: ToLane) {
      queueMicrotask(() => dispatch(message));
    },
  };
  const dispatch = laneDispatch(
    (message: FromLane) => {
      queueMicrotask(() => worker.onmessage?.({ data: message } as MessageEvent<FromLane>));
    },
    {
      views: (_kind, wake) => createLaneView(tableLane, wake, { timers: controlledTimers }),
      clock: controlledClock,
      crop: sources => Promise.all(sources.map(source => createImageBitmap(source as OffscreenCanvas))),
      later: run => requestAnimationFrame(() => run()),
    },
  );
  return worker;
}

const workers = [0, 1].map(() => createControlledLane());
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
  ...tableSettingsOf(undefined, SPECS, { density: 'comfortable', sort: { column: 'factories', direction: 'descending' } }),
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
let totalInstallReads = 0;
let installs = 0;
let flushWrapped = false;

const nextPoll = () => new Promise<void>(resolve => setTimeout(resolve, 16));
async function until(predicate: () => boolean, limitMs = 6000) {
  const deadline = performance.now() + limitMs;
  while (!predicate() && performance.now() < deadline) await nextPoll();
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

function ensureFlushInstrumentation() {
  if (flushWrapped) return;
  flushWrapped = true;
  const internal = pipeline as unknown as { flush(): void };
  const origFlush = internal.flush.bind(pipeline);
  internal.flush = () => {
    const { reads } = measureLayoutReadsDuring(() => origFlush());
    totalInstallReads += reads;
  };
  pipeline.onInstall(pipeId, () => { installs++; });
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
    totalInstallReads = 0;
    installs = 0;
    hostNode = document.createElement('div');
    hostNode.style.cssText = 'width:460px;height:260px;display:flex;flex-direction:column;';
    document.body.append(hostNode);
    const root = createRoot(hostNode);
    flushSync(() => root.render(
      <FollowedWidgetKey.Provider value="dom-scoreboard">
        <Profiler id="table-family" onRender={() => { commits++; }}>
          <FamilyHost
            adapter={tableHostAdapter}
            chrome={TableChrome}
            view="default"
            declaration={scoreboardProjection}
            options={OPTIONS}
            label="Scoreboard"
            subject="scoreboard"
            widget={widget}
            runtime={runtime}
            active
            settings={SETTINGS}
            subscriptions={SUBSCRIPTIONS}
            offset={0}
            pin={null}
            bounds={[0, 2000]}
            onActivate={action => { activated.push(action.subject); }}
            contextMenuItems={hit => [{ label: `Jump to date (${hit.key ?? 'empty'}@${hit.hour})`, run: () => {} }]}
          />
          <TooltipLayer />
        </Profiler>
      </FollowedWidgetKey.Provider>,
    ));

    const ready = await until(() => {
      const canvas = hostNode?.querySelector<HTMLCanvasElement>('canvas.fh-canvas');
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
      return view?.mode === 'picture' && !view.dim && (hostNode?.querySelectorAll('table tbody tr').length ?? 0) === 4;
    });
    ensureFlushInstrumentation();
    return { painted, initialRows: [...hostNode.querySelectorAll('table tbody tr')].map(tr => tr.getAttribute('data-row-key')) };
  },

  async playLive(targetHour: number) {
    commits = 0;
    totalInstallReads = 0;
    installs = 0;
    let sentThrough = 10;
    let flipFromHour = Number.MAX_SAFE_INTEGER;
    const feed = (hour: number) => {
      while (sentThrough < hour + 6) {
        sentThrough++;
        const fac: [bigint, bigint, bigint, bigint] = sentThrough >= flipFromHour
          ? [15n, 95n, 30n, -1n]
          : [85n, 40n, 30n, -1n];
        sendHour(sentThrough, fac, [BigInt(sentThrough * 10), BigInt(sentThrough * 20), 0n, 0n]);
      }
    };
    feed(10);
    const stopFeed = clock.onPublish((_prev, next) => feed(next.hour));
    clock.setSpeed(48);
    clock.setPlaying(true);

    let playingDuringAdvance = false;
    try {
      const reached = await until(() => {
        const snap = clock.getSnapshot();
        const view = pipeline.view(pipeId);
        if (snap.playing && (view?.h ?? 0) >= 14) playingDuringAdvance = true;
        return snap.playing && (view?.h ?? 0) >= targetHour && !view?.dim;
      }, 8000);
      if (!reached) throw new Error(`Live playback did not reach hour ${targetHour} (at ${pipeline.view(pipeId)?.h})`);
      const playingBeforePause = clock.getSnapshot().playing;
      const playCommits = commits;
      const playReads = totalInstallReads;
      const playInstalls = installs;

      const wantTop = 'FRA';
      const pauseHour = sentThrough + 1;
      holdCadence(performance.now() + 50);
      timerFiredCount = 0;
      flipFromHour = pauseHour;
      feed(pauseHour);
      const reachedFlip = await until(() => (pipeline.view(pipeId)?.h ?? 0) >= pauseHour && !pipeline.view(pipeId)?.dim, 4000);
      if (!reachedFlip) throw new Error(`Live playback did not reach flipped hour ${pauseHour} (at ${pipeline.view(pipeId)?.h})`);
      const beforePauseTop = hostNode?.querySelector('table tbody tr')?.getAttribute('data-row-key') ?? null;
      const pendingTimersBeforePause = heldTimers.size;
      const timerFiredBeforePause = timerFiredCount;

      clock.setPlaying(false);
      const flushedOnPause = await until(
        () => hostNode?.querySelector('table tbody tr')?.getAttribute('data-row-key') === wantTop,
        800,
      );
      if (!flushedOnPause) {
        throw new Error(`Mirror did not flush to ${wantTop} on pause (top=${hostNode?.querySelector('table tbody tr')?.getAttribute('data-row-key')})`);
      }
      const timerFiredAfterPause = timerFiredCount;
      const pendingTimersAfterPause = heldTimers.size;

      const view = pipeline.view(pipeId);
      const mirrorRows = [...(hostNode?.querySelectorAll('table tbody tr') ?? [])].map(tr => ({
        key: tr.getAttribute('data-row-key'),
        cells: [...tr.querySelectorAll('td')].map(td => td.textContent),
      }));
      return {
        playingDuringAdvance,
        playingBeforePause,
        beforePauseTop,
        pendingTimersBeforePause,
        pendingTimersAfterPause,
        timerFiredBeforePause,
        timerFiredAfterPause,
        commits: playCommits,
        totalInstallReads: playReads,
        pauseInstallReads: totalInstallReads,
        installs: playInstalls,
        flushedOnPause,
        wantTop,
        finalHour: view?.h ?? null,
        mirrorRows,
      };
    } finally {
      cadenceHold = false;
      heldTimers.clear();
      stopFeed();
      clock.setPlaying(false);
    }
  },

  async mirrorSyncsOverFrames(frames: number) {
    const body = hostNode?.querySelector('table tbody');
    if (!body) throw new Error('No accessible mirror body');
    const fac: [bigint, bigint, bigint, bigint] = [15n, 95n, 30n, -1n];
    const cas: [bigint, bigint, bigint, bigint] = [1n, 2n, 0n, 0n];
    let hour = pipeline.view(pipeId)?.h ?? 0;
    const step = async () => {
      hour++;
      sendHour(hour, fac, cas);
      const before = installs;
      clock.step(1);
      return until(() => installs > before && pipeline.view(pipeId)?.h === hour && !pipeline.view(pipeId)?.dim && body.children.length > 0, 2000);
    };
    await step();
    let mutations = 0;
    const observer = new MutationObserver(records => { mutations += records.length; });
    observer.observe(body, { subtree: true, childList: true, attributes: true, characterData: true });
    const installsBefore = installs;
    let stepped = 0;
    for (let i = 0; i < frames; i++) if (await step()) stepped++;
    observer.disconnect();
    return { mutations, installs: installs - installsBefore, stepped };
  },

  async focusAcrossWithheldMirror() {
    const body = hostNode!.querySelector<HTMLTableSectionElement>('table tbody')!;
    const table = body.closest('table')!;
    const row = body.querySelector<HTMLTableRowElement>('tr[data-row-key]')!;
    const key = row.dataset.rowKey!;
    const focusedKey = () => (document.activeElement as HTMLElement | null)?.dataset.rowKey ?? null;
    row.focus();
    await until(() => (pipeline.view(pipeId)?.fields?.focusTooltip as { text?: { rowKey?: string } | null } | undefined)?.text?.rowKey === key, 2000);
    let sent = pipeline.view(pipeId)?.h ?? 0;
    const feed = (hour: number) => {
      while (sent < hour + 6) {
        sent++;
        sendHour(sent, [BigInt(85 + sent), 40n, 30n, -1n], [BigInt(sent * 10), BigInt(sent * 20), 0n, 0n]);
      }
    };
    feed(sent);
    const stopFeed = clock.onPublish((_prev, next) => feed(next.hour));
    try {
      holdCadence(performance.now());
      clock.setSpeed(48);
      clock.setPlaying(true);
      const withheld = await until(() => body.children.length === 0, 4000);
      const parked = document.activeElement === table;
      clock.setPlaying(false);
      const returned = await until(() => body.children.length > 0, 2000);
      const restored = focusedKey();
      document.activeElement?.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
      await until(() => focusedKey() !== null && focusedKey() !== key, 2000);
      return { key, withheld, parked, returned, restored, afterDown: focusedKey() };
    } finally {
      cadenceHold = false;
      heldTimers.clear();
      stopFeed();
      clock.setPlaying(false);
      (document.activeElement as HTMLElement | null)?.blur();
    }
  },

  async verifyRepaintAndHeld() {
    const hashCanvas = async () => {
      const canvas = hostNode!.querySelector<HTMLCanvasElement>('canvas.fh-canvas')!;
      const bitmap = await createImageBitmap(canvas);
      const out = new OffscreenCanvas(bitmap.width, bitmap.height);
      const ctx = out.getContext('2d')!;
      ctx.drawImage(bitmap, 0, 0);
      const data = ctx.getImageData(0, 0, bitmap.width, bitmap.height).data;
      let hash = 2166136261;
      for (let i = 0; i < data.length; i++) hash = Math.imul(hash ^ data[i]!, 16777619);
      return `${bitmap.width}x${bitmap.height}:${(hash >>> 0).toString(16)}`;
    };
    const baseHour = (pipeline.view(pipeId)?.h ?? 30) + 5;
    clock.seek(baseHour);
    seek = clock.getSnapshot().seekSequence;
    sendHour(baseHour, [70n, 40n, 30n, -1n], [100n, 200n, 0n, 0n], true);
    await until(() => pipeline.view(pipeId)?.h === baseHour && !pipeline.view(pipeId)?.dim);
    let prevHash = await hashCanvas();
    let stableCount = 0;
    const settleDeadline = performance.now() + 2000;
    while (stableCount < 3 && performance.now() < settleDeadline) {
      await nextPoll();
      const curHash = await hashCanvas();
      if (curHash === prevHash) stableCount++;
      else { stableCount = 0; prevHash = curHash; }
    }
    const beforeHash = prevHash;
    sendHour(baseHour + 1, [75n, 40n, 30n, -1n], [100n, 200n, 0n, 0n]);
    clock.step(1);
    await until(() => pipeline.view(pipeId)?.h === baseHour + 1 && !pipeline.view(pipeId)?.dim);
    const afterHash = await hashCanvas();
    for (let s = 0; s < 5; s++) clock.step(1);
    await until(() => pipeline.view(pipeId)?.dim === true, 3000);
    const dimView = pipeline.view(pipeId);
    const held = !!hostNode!.querySelector('[data-held]');
    sendHour(baseHour + 6, [70n, 40n, 30n, -1n], [100n, 200n, 0n, 0n]);
    const recovered = await until(() => pipeline.view(pipeId)?.h === baseHour + 6 && !pipeline.view(pipeId)?.dim, 3000);
    return { pixelsChanged: beforeHash !== afterHash, dim: !!dimView?.dim, held, recovered, through: dimView?.through ?? null, expectedThrough: baseHour + 1 };
  },

  bodyBox() {
    const box = hostNode!.querySelector<HTMLDivElement>('.fh-body')!.getBoundingClientRect();
    return { left: box.left, top: box.top, width: box.width, height: box.height };
  },

  async waitInspected(expectedKey?: string) {
    const ok = await until(() => {
      const tip = pipeline.view(pipeId)?.fields?.tooltip as { text?: { rowKey?: string } | null } | undefined;
      if (!tip?.text?.rowKey) return false;
      return expectedKey ? tip.text.rowKey === expectedKey : true;
    }, 3000);
    if (!ok) {
      const view = pipeline.view(pipeId);
      throw new Error(`waitInspected(${expectedKey}) failed: mode=${view?.mode} h=${view?.h} dim=${view?.dim} inspect=${(pipeline as unknown as { page: { widgets: Map<number, { inspect: string | null }> } }).page.widgets.get(pipeId)?.inspect} tip=${JSON.stringify(view?.fields?.tooltip)}`);
    }
    return ok;
  },

  async resortUnderPointer() {
    activated = [];
    const nextHour = (pipeline.view(pipeId)?.h ?? 50) + 1;
    sendHour(nextHour, [20n, 90n, 80n, -1n], [1n, 2n, 3n, 4n]);
    clock.step(1);
    await until(() => pipeline.view(pipeId)?.h === nextHour && !pipeline.view(pipeId)?.dim);
    await until(() => hostNode?.querySelector('table tbody tr')?.getAttribute('data-row-key') === 'FRA');
    const hashCanvas = async () => {
      const canvas = hostNode!.querySelector<HTMLCanvasElement>('canvas.fh-canvas')!;
      const bitmap = await createImageBitmap(canvas);
      const out = new OffscreenCanvas(bitmap.width, bitmap.height);
      const ctx = out.getContext('2d')!;
      ctx.drawImage(bitmap, 0, 0);
      const data = ctx.getImageData(0, 0, bitmap.width, bitmap.height).data;
      let hash = 2166136261;
      for (let i = 0; i < data.length; i++) hash = Math.imul(hash ^ data[i]!, 16777619);
      return `${bitmap.width}x${bitmap.height}:${(hash >>> 0).toString(16)}`;
    };
    let prevHash = await hashCanvas();
    let stableCount = 0;
    const settleDeadline = performance.now() + 2000;
    while (stableCount < 3 && performance.now() < settleDeadline) {
      await nextPoll();
      const curHash = await hashCanvas();
      if (curHash === prevHash) stableCount++;
      else { stableCount = 0; prevHash = curHash; }
    }
    return [...(hostNode?.querySelectorAll('table tbody tr') ?? [])].map(tr => tr.getAttribute('data-row-key'));
  },

  readActivated() {
    return [...activated];
  },

  async focusFirstAndWaitTooltip() {
    const firstRow = hostNode!.querySelector<HTMLTableRowElement>('table tbody tr')!;
    const key = firstRow.getAttribute('data-row-key')!;
    firstRow.focus();
    const ready = await until(() => {
      const tip = pipeline.view(pipeId)?.fields?.focusTooltip as { text?: { rowKey?: string } | null } | undefined;
      return tip?.text?.rowKey === key;
    }, 2000);
    if (!ready) {
      const view = pipeline.view(pipeId);
      throw new Error(`focusFirstAndWaitTooltip(${key}) failed: active=${(document.activeElement as HTMLElement | null)?.outerHTML?.slice(0, 120)} dim=${view?.dim} focusTooltip=${JSON.stringify(view?.fields?.focusTooltip)} tooltip=${JSON.stringify(view?.fields?.tooltip)}`);
    }
    return { key, ready };
  },

  async pinFocusedRow() {
    totalInstallReads = 0;
    const active = document.activeElement as HTMLElement | null;
    active?.dispatchEvent(new KeyboardEvent('keydown', { key: 'p', bubbles: true }));
    await until(() => (tooltipStore.get()?.content.subtitle ?? '').startsWith('Pinned at '), 2000);
    const pinned = tooltipStore.get();
    const result = {
      title: pinned?.content.title ?? null,
      subtitle: pinned?.content.subtitle ?? null,
      installReads: totalInstallReads,
    };
    active?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await until(() => tooltipStore.get() === null || !(tooltipStore.get()?.content.subtitle ?? '').startsWith('Pinned at '), 2000);
    return result;
  },

  async testOverflowTooltipDuringPlayback() {
    const origHeightDesc = Object.getOwnPropertyDescriptor(window, 'innerHeight');
    Object.defineProperty(window, 'innerHeight', { configurable: true, get: () => 90 });
    window.dispatchEvent(new Event('resize'));
    const startHour = (pipeline.view(pipeId)?.h ?? 50) + 1;
    sendHour(startHour, [20n, 90n, 80n, -1n], [BigInt(startHour), BigInt(startHour * 2), 3n, 4n]);
    clock.step(1);
    const tipVisible = await until(() => !!tooltipStore.get()?.content.spoken, 2000);
    if (!tipVisible) throw new Error(`Overflowing tooltip did not truncate with spoken fallback (tip=${JSON.stringify(tooltipStore.get()?.content)})`);
    let sent = startHour;
    const feed = (hour: number) => {
      while (sent < hour + 6) {
        sent++;
        sendHour(sent, [20n, 90n, 80n, -1n], [BigInt(sent), BigInt(sent * 2), 3n, 4n]);
      }
    };
    const stopFeed = clock.onPublish((_prev, next) => feed(next.hour));
    try {
      commits = 0;
      totalInstallReads = 0;
      installs = 0;
      feed(startHour + 1);
      clock.setSpeed(168);
      clock.setPlaying(true);
      const advanced = await until(() => (pipeline.view(pipeId)?.h ?? 0) >= startHour + 6 && installs >= 3, 2000);
      if (!advanced) throw new Error(`Playback with overflowing tooltip did not advance (h=${pipeline.view(pipeId)?.h}, installs=${installs})`);
      return { tipOverflowed: true, commits, installReads: totalInstallReads, installs };
    } finally {
      stopFeed();
      clock.setPlaying(false);
      if (origHeightDesc) Object.defineProperty(window, 'innerHeight', origHeightDesc);
      window.dispatchEvent(new Event('resize'));
    }
  },

  async testKeyboardAndNarrow() {
    activated = [];
    const firstRow = hostNode!.querySelector<HTMLTableRowElement>('table tbody tr')!;
    const firstKey = firstRow.getAttribute('data-row-key');
    firstRow.focus();
    firstRow.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
    const moved = await until(() => {
      const cur = (document.activeElement as HTMLElement | null)?.getAttribute('data-row-key');
      return !!cur && cur !== firstKey;
    }, 2000);
    if (!moved) throw new Error(`ArrowDown did not move focus off ${firstKey}`);
    const afterDown = (document.activeElement as HTMLElement | null)?.getAttribute('data-row-key') ?? null;
    document.activeElement?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    const didActivate = await until(() => activated.length === 1, 2000);
    if (!didActivate) throw new Error('Enter did not activate focused row');
    const liveStatus = hostNode!.querySelector('[role="status"]')?.textContent ?? '';
    const openedFrom = afterDown ?? '';
    const windowNode = document.createElement('div');
    document.body.append(windowNode);
    const windowRoot = createRoot(windowNode);
    const anchor = { current: hostNode };
    flushSync(() => windowRoot.render(
      <DetailWindowHost open subject={openedFrom} hour={10} title="Opened row" anchor={anchor}
        returnFocus={mirrorRowSelector(openedFrom)} onClose={() => {}}>
        <button type="button" className="opened-row-inside">Inside</button>
      </DetailWindowHost>,
    ));
    document.querySelector<HTMLElement>('.opened-row-inside')?.focus();
    document.querySelector<HTMLElement>('.opened-row-inside')?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    const focusReturned = await until(() => (document.activeElement as HTMLElement | null)?.getAttribute('data-row-key') === openedFrom, 2000);
    windowRoot.unmount();
    windowNode.remove();

    hostNode!.style.width = '210px';
    const scrollsSideways = await until(() => !!hostNode!.querySelector('.tf-edges[data-right]'), 3000);
    return { firstKey, afterDown, activated: [...activated], liveStatus, focusReturned, scrollsSideways, totalInstallReads };
  },

  async testEmptySpaceContextMenuAndValueStateTooltip() {
    hostNode!.style.width = '460px';
    const nextHour = Math.floor(clock.getSnapshot().hour) + 2;
    clock.seek(nextHour);
    seek = clock.getSnapshot().seekSequence;
    sendHour(nextHour, [20n, 90n, 80n, -1n], [BigInt(nextHour), BigInt(nextHour * 2), 3n, 4n], true);
    await until(() => {
      const view = pipeline.view(pipeId);
      return view?.mode === 'picture' && view.h === nextHour && !view.dim;
    }, 2000);
    (document.activeElement as HTMLElement | null)?.blur?.();
    const bodyEl = hostNode!.querySelector<HTMLElement>('.fh-body')!;
    bodyEl.dispatchEvent(new PointerEvent('pointerleave', { bubbles: false }));
    await until(() => pipeline.view(pipeId)?.fields?.tooltip == null, 1500);
    const frameEl = hostNode!.querySelector<HTMLElement>('.fh-frame')!;
    const box = frameEl.getBoundingClientRect();
    const ev = new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: box.left + 12, clientY: box.bottom - 8 });
    frameEl.dispatchEvent(ev);
    const opened = await until(() => !!document.querySelector('[role="menu"]:popover-open [role="menuitem"]'), 2000);
    const menuLabel = document.querySelector('[role="menu"]:popover-open [role="menuitem"]')?.textContent ?? null;
    document.querySelector<HTMLElement>('[role="menu"]:popover-open')?.hidePopover?.();

    const engRow = hostNode!.querySelector<HTMLTableRowElement>('table tbody tr[data-row-key="ENG"]')!;
    engRow.focus();
    const tipShown = await until(() => !!document.querySelector('.chart-tooltip [data-value-state="missing"]'), 2000);
    const hasMissingState = !!document.querySelector('.chart-tooltip [data-value-state="missing"]');
    const hasKnownState = !!document.querySelector('.chart-tooltip [data-value-state="known"]');
    return { opened, menuLabel, tipShown, hasMissingState, hasKnownState };
  },
};
