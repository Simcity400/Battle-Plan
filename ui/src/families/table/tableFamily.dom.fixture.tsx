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
import type { IdentitySnapshot } from '../identity/types.ts';
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

let motionFrames = 0;
let cropsInFlight = 0;

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
      crop: sources => {
        cropsInFlight++;
        return Promise.all(sources.map(source => createImageBitmap(source as OffscreenCanvas))).finally(() => { cropsInFlight--; });
      },
      later: run => {
        motionFrames++;
        requestAnimationFrame(() => { motionFrames--; run(); });
      },
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
};

const SUBSCRIPTIONS = [
  { name: 'factories', source, branch: '1', offset: 0, query: { product: 'series-state', params: { branchId: '1', col: 'factories' } }, channel: 'followed' as const },
  { name: 'casualties', source, branch: '1', offset: 0, query: { product: 'series-state', params: { branchId: '1', col: 'casualties' } }, channel: 'followed' as const },
];

const IDENTITY_SNAPSHOT: IdentitySnapshot = {
  scope: { sourceId: 'dom-test', revision: 'r1', branchId: '1', content: 1 },
  branchBounds: [0, 2000],
  facts: {
    branchId: '1',
    known: [[0, 2000]],
    names: ['Germany', 'France', 'Italy', 'United Kingdom'],
    reasons: [],
    flags: [
      { keys: ['GER', null, null], reasons: [null, null, null] },
      { keys: ['FRA', null, null], reasons: [null, null, null] },
      { keys: ['ITA', null, null], reasons: [null, null, null] },
      { keys: ['ENG', null, null], reasons: [null, null, null] },
    ],
    countries: [
      { tag: 'GER', runs: [[0, 0, 0x444444, -1, 0]] },
      { tag: 'FRA', runs: [[0, 1, 0x3366aa, -1, 1]] },
      { tag: 'ITA', runs: [[0, 2, 0x338844, -1, 2]] },
      { tag: 'ENG', runs: [[0, 3, 0xcc4444, -1, 3]] },
    ],
    warnings: [],
  },
  factsFinal: true,
};

let commits = 0;
let tooltipCommits = 0;
let activated: string[] = [];
let hostNode: HTMLDivElement | null = null;
let subIds: Record<string, number> = {};
let pipeId = -1;
let totalInstallReads = 0;
let installs = 0;
let flushWrapped = false;
let mirrorRun: { body: Element; observer: MutationObserver; hour: number; mutations: number; installsBefore: number } | null = null;

const HANG_GUARD_MS = 20_000;
const MIRROR_FACTORIES: [bigint, bigint, bigint, bigint] = [15n, 95n, 30n, -1n];
const MIRROR_CASUALTIES: [bigint, bigint, bigint, bigint] = [1n, 2n, 0n, 0n];
const nextPoll = () => new Promise<void>(resolve => setTimeout(resolve, 16));
const nextFrame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
const failure = (what: string | (() => string)) => new Error(typeof what === 'string' ? `Gave up waiting for ${what}` : what());

async function until(predicate: () => boolean, what: string | (() => string)) {
  const guard = performance.now() + HANG_GUARD_MS;
  while (!predicate()) {
    if (performance.now() > guard) throw failure(what);
    await nextPoll();
  }
  return true;
}

async function quietFrame(count: () => number, what: string) {
  const guard = performance.now() + HANG_GUARD_MS;
  for (;;) {
    const seen = count();
    await nextFrame();
    await nextFrame();
    if (count() === seen) return;
    if (performance.now() > guard) throw failure(what);
  }
}

const pictureSettled = () => motionFrames === 0 && cropsInFlight === 0 && !(pipeline as unknown as { framing: boolean }).framing;

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
            identitySnapshot={IDENTITY_SNAPSHOT}
            offset={0}
            pin={null}
            bounds={[0, 2000]}
            onActivate={action => { activated.push(action.subject); }}
            contextMenuItems={hit => [{ label: `Jump to date (${hit.key ?? 'empty'}@${hit.hour})`, run: () => {} }]}
          />
        </Profiler>
        <Profiler id="table-tooltip" onRender={() => { tooltipCommits++; }}>
          <TooltipLayer />
        </Profiler>
      </FollowedWidgetKey.Provider>,
    ));

    await until(() => {
      const canvas = hostNode?.querySelector<HTMLCanvasElement>('canvas.fh-canvas');
      if (!canvas?.dataset.placement) return false;
      pipeId = Number(canvas.dataset.placement);
      return pipeId > 0;
    }, 'the table canvas to mount');

    const internal = pipeline as unknown as { widgets: Map<number, { subs: Map<object, number> }> };
    const subList = [...(internal.widgets.get(pipeId)?.subs.values() ?? [])];
    subIds = { factories: subList[0]!, casualties: subList[1]! };
    await until(() => pipeline.status(subIds.factories!).status === 'followed' && pipeline.status(subIds.casualties!).status === 'followed',
      'both subscriptions to be followed');

    clock.seek(10);
    seek = clock.getSnapshot().seekSequence;
    sendHour(10, [50n, 40n, 30n, -1n], [0n, 100n, 200n, 0n], true);
    const painted = await until(() => {
      const view = pipeline.view(pipeId);
      return view?.mode === 'picture' && !view.dim && (hostNode?.querySelectorAll('table tbody tr').length ?? 0) === 4;
    }, 'the first picture and its four mirror rows');
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
      await until(() => {
        const snap = clock.getSnapshot();
        const view = pipeline.view(pipeId);
        if (snap.playing && (view?.h ?? 0) >= 14) playingDuringAdvance = true;
        return snap.playing && (view?.h ?? 0) >= targetHour && !view?.dim && installs >= 6;
      }, () => `Live playback did not reach hour ${targetHour} with six installs (at ${pipeline.view(pipeId)?.h}, ${installs} installs)`);
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
      await until(() => (pipeline.view(pipeId)?.h ?? 0) >= pauseHour && !pipeline.view(pipeId)?.dim,
        () => `Live playback did not reach flipped hour ${pauseHour} (at ${pipeline.view(pipeId)?.h})`);
      const beforePauseTop = hostNode?.querySelector('table tbody tr')?.getAttribute('data-row-key') ?? null;
      const pendingTimersBeforePause = heldTimers.size;
      const timerFiredBeforePause = timerFiredCount;

      clock.setPlaying(false);
      const flushedOnPause = await until(
        () => hostNode?.querySelector('table tbody tr')?.getAttribute('data-row-key') === wantTop,
        () => `Mirror did not flush to ${wantTop} on pause (top=${hostNode?.querySelector('table tbody tr')?.getAttribute('data-row-key')})`,
      );
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

  async mirrorBegin() {
    const body = hostNode?.querySelector('table tbody');
    if (!body) throw new Error('No accessible mirror body');
    const hour = clock.getSnapshot().hour + 1;
    clock.seek(hour);
    seek = clock.getSnapshot().seekSequence;
    sendHour(hour, MIRROR_FACTORIES, MIRROR_CASUALTIES, true);
    await until(() => pipeline.view(pipeId)?.h === hour && !pipeline.view(pipeId)?.dim
      && body.querySelector('[data-row-key="FRA"] td:last-child')?.textContent === String(MIRROR_CASUALTIES[1]),
      () => `Unchanged-content baseline did not install: ${body.textContent}`);
    const observer = new MutationObserver(records => { mirrorRun!.mutations += records.length; });
    mirrorRun = { body, observer, hour, mutations: 0, installsBefore: installs };
    observer.observe(body, { subtree: true, childList: true, attributes: true, characterData: true });
    return true;
  },

  async mirrorStep() {
    const run = mirrorRun!;
    const hour = ++run.hour;
    sendHour(hour, MIRROR_FACTORIES, MIRROR_CASUALTIES);
    const before = installs;
    clock.step(1);
    return until(() => installs > before && pipeline.view(pipeId)?.h === hour && !pipeline.view(pipeId)?.dim && run.body.children.length > 0,
      `the paused step to hour ${hour} to install`);
  },

  mirrorEnd() {
    const run = mirrorRun!;
    mirrorRun = null;
    run.mutations += run.observer.takeRecords().length;
    run.observer.disconnect();
    return { mutations: run.mutations, installs: installs - run.installsBefore };
  },

  async focusAcrossWithheldMirror() {
    const body = hostNode!.querySelector<HTMLTableSectionElement>('table tbody')!;
    const table = body.closest('table')!;
    const row = body.querySelector<HTMLTableRowElement>('tr[data-row-key]')!;
    const key = row.dataset.rowKey!;
    const focusedKey = () => (document.activeElement as HTMLElement | null)?.dataset.rowKey ?? null;
    row.focus();
    await until(() => (pipeline.view(pipeId)?.fields?.focusTooltip as { text?: { rowKey?: string } | null } | undefined)?.text?.rowKey === key,
      `the focus tooltip of ${key}`);
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
      const withheld = await until(() => body.children.length === 0, 'playback with a held cadence to withhold the mirror rows');
      const parked = document.activeElement === table;
      clock.setPlaying(false);
      const returned = await until(() => body.children.length > 0, 'the mirror rows to return on pause');
      const restored = focusedKey();
      document.activeElement?.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
      await until(() => focusedKey() !== null && focusedKey() !== key, `ArrowDown to move focus off ${key}`);
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
    await until(() => pipeline.view(pipeId)?.h === baseHour && !pipeline.view(pipeId)?.dim, `hour ${baseHour} to install`);
    await until(pictureSettled, `the picture of hour ${baseHour} to settle`);
    const beforeHash = await hashCanvas();
    sendHour(baseHour + 1, [75n, 40n, 30n, -1n], [100n, 200n, 0n, 0n]);
    clock.step(1);
    await until(() => pipeline.view(pipeId)?.h === baseHour + 1 && !pipeline.view(pipeId)?.dim, `hour ${baseHour + 1} to install`);
    await until(pictureSettled, `the picture of hour ${baseHour + 1} to settle`);
    const afterHash = await hashCanvas();
    for (let s = 0; s < 5; s++) clock.step(1);
    await until(() => pipeline.view(pipeId)?.dim === true, 'the view to dim past its confirmed hour');
    const dimView = pipeline.view(pipeId);
    const held = !!hostNode!.querySelector('[data-held]');
    sendHour(baseHour + 6, [70n, 40n, 30n, -1n], [100n, 200n, 0n, 0n]);
    const recovered = await until(() => pipeline.view(pipeId)?.h === baseHour + 6 && !pipeline.view(pipeId)?.dim, `hour ${baseHour + 6} to arrive and clear the dim`);
    return { pixelsChanged: beforeHash !== afterHash, dim: !!dimView?.dim, held, recovered, through: dimView?.through ?? null, expectedThrough: baseHour + 1 };
  },

  bodyBox() {
    const box = hostNode!.querySelector<HTMLDivElement>('.fh-body')!.getBoundingClientRect();
    return { left: box.left, top: box.top, width: box.width, height: box.height };
  },

  async waitInspected(expectedKey?: string) {
    return until(() => {
      const tip = pipeline.view(pipeId)?.fields?.tooltip as { text?: { rowKey?: string } | null } | undefined;
      if (!tip?.text?.rowKey) return false;
      return expectedKey ? tip.text.rowKey === expectedKey : true;
    }, () => {
      const view = pipeline.view(pipeId);
      return `waitInspected(${expectedKey}) failed: mode=${view?.mode} h=${view?.h} dim=${view?.dim} inspect=${(pipeline as unknown as { page: { widgets: Map<number, { inspect: string | null }> } }).page.widgets.get(pipeId)?.inspect} tip=${JSON.stringify(view?.fields?.tooltip)}`;
    });
  },

  async resortUnderPointer() {
    activated = [];
    const nextHour = (pipeline.view(pipeId)?.h ?? 50) + 1;
    sendHour(nextHour, [20n, 90n, 80n, -1n], [1n, 2n, 3n, 4n]);
    clock.step(1);
    await until(() => pipeline.view(pipeId)?.h === nextHour && !pipeline.view(pipeId)?.dim, `hour ${nextHour} to install`);
    await until(() => hostNode?.querySelector('table tbody tr')?.getAttribute('data-row-key') === 'FRA', 'FRA to lead the mirror');
    await until(pictureSettled, 'the re-sorted rows to finish moving');
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
    }, () => {
      const view = pipeline.view(pipeId);
      return `focusFirstAndWaitTooltip(${key}) failed: active=${(document.activeElement as HTMLElement | null)?.outerHTML?.slice(0, 120)} dim=${view?.dim} focusTooltip=${JSON.stringify(view?.fields?.focusTooltip)} tooltip=${JSON.stringify(view?.fields?.tooltip)}`;
    });
    return { key, ready };
  },

  async pinFocusedRow() {
    totalInstallReads = 0;
    const active = document.activeElement as HTMLElement | null;
    active?.dispatchEvent(new KeyboardEvent('keydown', { key: 'p', bubbles: true }));
    await until(() => (tooltipStore.get()?.content.subtitle ?? '').startsWith('Pinned at '), 'P to pin the focused row');
    const pinned = tooltipStore.get();
    const result = {
      title: pinned?.content.title ?? null,
      subtitle: pinned?.content.subtitle ?? null,
      installReads: totalInstallReads,
    };
    active?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await until(() => tooltipStore.get() === null || !(tooltipStore.get()?.content.subtitle ?? '').startsWith('Pinned at '), 'Escape to release the pin');
    return result;
  },

  async testOverflowTooltipDuringPlayback() {
    const origHeightDesc = Object.getOwnPropertyDescriptor(window, 'innerHeight');
    const restoreHeight = () => { if (origHeightDesc) Object.defineProperty(window, 'innerHeight', origHeightDesc); };
    const resized = async (what: string) => {
      const before = commits;
      window.dispatchEvent(new Event('resize'));
      await until(() => commits > before, what);
    };
    Object.defineProperty(window, 'innerHeight', { configurable: true, get: () => 90 });
    await resized('the host to take the short viewport height');
    const startHour = (pipeline.view(pipeId)?.h ?? 50) + 1;
    sendHour(startHour, [20n, 90n, 80n, -1n], [BigInt(startHour), BigInt(startHour * 2), 3n, 4n]);
    clock.step(1);
    await until(() => !!tooltipStore.get()?.content.spoken,
      () => `Overflowing tooltip did not truncate with spoken fallback (tip=${JSON.stringify(tooltipStore.get()?.content)})`);
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
      tooltipCommits = 0;
      totalInstallReads = 0;
      installs = 0;
      feed(startHour + 1);
      clock.setSpeed(168);
      clock.setPlaying(true);
      await until(() => (pipeline.view(pipeId)?.h ?? 0) >= startHour + 6 && installs >= 3,
        () => `Playback with overflowing tooltip did not advance (h=${pipeline.view(pipeId)?.h}, installs=${installs})`);
      const hostCommits = commits;
      const playInstalls = installs;
      stopFeed();
      clock.setPlaying(false);
      await until(() => !clock.getSnapshot().playing, 'the clock to pause');
      const settledHour = Math.floor(clock.getSnapshot().hour) + 1;
      const casVal = BigInt(settledHour);
      const casVal2 = BigInt(settledHour * 2);
      clock.seek(settledHour);
      seek = clock.getSnapshot().seekSequence;
      sendHour(settledHour, [20n, 90n, 80n, -1n], [casVal, casVal2, 3n, 4n], true);
      await until(() => pipeline.view(pipeId)?.h === settledHour && !pipeline.view(pipeId)?.dim && !!tooltipStore.get()?.content.spoken,
        `hour ${settledHour} to install with its overflowing tooltip`);
      await quietFrame(() => tooltipCommits, 'the tooltip layer to settle after the seek');
      tooltipCommits = 0;
      totalInstallReads = 0;
      installs = 0;
      for (let i = 0; i < 4; i++) {
        const before = installs;
        sendHour(settledHour, [21n + BigInt(i), 90n, 80n, -1n], [casVal, casVal2, 3n, 4n]);
        await until(() => installs > before, 'the changed Germany row to install');
      }
      await quietFrame(() => tooltipCommits, 'the tooltip layer to settle after the unchanged installs');
      const result = {
        tipOverflowed: true,
        commits: hostCommits,
        installs: playInstalls,
        unchangedInstalls: installs,
        unchangedTooltipCommits: tooltipCommits,
        unchangedInstallReads: totalInstallReads,
      };
      restoreHeight();
      await resized('the host to take back the full viewport height');
      return result;
    } finally {
      stopFeed();
      clock.setPlaying(false);
      restoreHeight();
      window.dispatchEvent(new Event('resize'));
    }
  },

  async testKeyboardAndNarrow() {
    activated = [];
    const firstRow = hostNode!.querySelector<HTMLTableRowElement>('table tbody tr')!;
    const firstKey = firstRow.getAttribute('data-row-key');
    firstRow.focus();
    firstRow.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true }));
    await until(() => {
      const cur = (document.activeElement as HTMLElement | null)?.getAttribute('data-row-key');
      return !!cur && cur !== firstKey;
    }, `ArrowDown to move focus off ${firstKey}`);
    const afterDown = (document.activeElement as HTMLElement | null)?.getAttribute('data-row-key') ?? null;
    document.activeElement?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    await until(() => activated.length === 1, 'Enter to activate the focused row');
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
    const focusReturned = await until(() => (document.activeElement as HTMLElement | null)?.getAttribute('data-row-key') === openedFrom,
      `focus to return to ${openedFrom} when its window closes`);
    windowRoot.unmount();
    windowNode.remove();

    hostNode!.style.width = '210px';
    const scrollsSideways = await until(() => !!hostNode!.querySelector('.tf-edges[data-right]'), 'the narrow table to show its right fade');
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
    }, `hour ${nextHour} to install`);
    (document.activeElement as HTMLElement | null)?.blur?.();
    const bodyEl = hostNode!.querySelector<HTMLElement>('.fh-body')!;
    bodyEl.dispatchEvent(new PointerEvent('pointerleave', { bubbles: false }));
    await until(() => pipeline.view(pipeId)?.fields?.tooltip == null, 'the hover tooltip to clear when the pointer leaves');
    const frameEl = hostNode!.querySelector<HTMLElement>('.fh-frame')!;
    const box = frameEl.getBoundingClientRect();
    const ev = new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: box.left + 12, clientY: box.bottom - 8 });
    frameEl.dispatchEvent(ev);
    const opened = await until(() => !!document.querySelector('[role="menu"]:popover-open [role="menuitem"]'), 'the context menu to open');
    const menuLabel = document.querySelector('[role="menu"]:popover-open [role="menuitem"]')?.textContent ?? null;
    document.querySelector<HTMLElement>('[role="menu"]:popover-open')?.hidePopover?.();

    const engRow = hostNode!.querySelector<HTMLTableRowElement>('table tbody tr[data-row-key="ENG"]')!;
    engRow.focus();
    const tipShown = await until(() => !!document.querySelector('.chart-tooltip [data-value-state="missing"]'), 'the ENG tooltip to show its missing value');
    const hasMissingState = !!document.querySelector('.chart-tooltip [data-value-state="missing"]');
    const hasKnownState = !!document.querySelector('.chart-tooltip [data-value-state="known"]');
    return { opened, menuLabel, tipShown, hasMissingState, hasKnownState };
  },
};
