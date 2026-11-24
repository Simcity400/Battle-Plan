import { Profiler, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { flushSync } from 'react-dom';
import type { DashboardClient, DashboardSource } from '../../dashboard/client';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { HOUR_EPOCH } from '../../dashboard/calendar.ts';
import { datedPanel } from '../../dashboard/follow/messages.fixture.ts';
import { pipelineFor, type LaneWorker, type PipelineHost } from '../../dashboard/follow/pipeline.ts';
import type { FromLane } from '../../draw/laneMessages';
import { packet, resetStamps, splitForLanes } from '../../dashboard/follow/split.ts';
import { FollowedWidgetKey } from '../../dashboard/follow/store.ts';
import type { WidgetSpec } from '../../dashboard/model';
import { PlaybackController } from '../../dashboard/playback.ts';
import { FrameEngine } from '../../engine/frame.ts';
import type { LogCell, LogPageRow } from '../../generated/dashboard';
import { marketLedgerProjection, type MarketLedgerProjectionOptions } from '../../widgets/market-ledger/marketLedgerProjection.ts';
import WidgetState from '../../components/WidgetState.tsx';
import FamilyLogHost from './FamilyLogHost.tsx';
import type { LogMetaField } from './laneView.ts';
import type { LogTailReplyWire } from './tailReply.ts';
import '../../styles.css';

const ROUTE = 11;
const BASE_HOUR = HOUR_EPOCH + 1939 * 365 * 24;
const MARKET_COLUMNS = [
  'id', 'occurrence', 'kind', 'mode', 'contract', 'seller', 'buyer', 'variant',
  'equipment', 'equipmentMissing', 'amountRaw', 'auxiliaryRaw', 'totalRaw', 'lostTotalRaw', 'reason',
];

const numCell = (value: string | number): LogCell => ({ kind: 'number', value: String(value) });
const txtCell = (value: string): LogCell => ({ kind: 'text', value });

function makeRow(ordinal: number, hour: number): LogPageRow {
  const map: Record<string, LogCell> = {
    id: txtCell(String(ordinal)),
    occurrence: numCell(ordinal),
    kind: numCell(ordinal % 2 === 0 ? 20 : 21),
    mode: numCell(ordinal % 2 === 0 ? 20 : 21),
    contract: txtCell(`000000000000${String(ordinal).padStart(4, '0')}`),
    seller: txtCell('GER'),
    buyer: txtCell('FRA'),
    variant: txtCell('7'),
    equipment: txtCell('Infantry Equipment I'),
    equipmentMissing: { kind: 'missing' },
    amountRaw: numCell(String(1000 * (ordinal + 1))),
    auxiliaryRaw: { kind: 'missing' },
    totalRaw: numCell(String(1000 * (ordinal + 1))),
    lostTotalRaw: { kind: 'missing' },
    reason: { kind: 'missing' },
  };
  return {
    hour,
    key: `m:${ordinal}`,
    ordinal: String(ordinal),
    cells: MARKET_COLUMNS.map(col => map[col] ?? { kind: 'missing' }),
  };
}

function makeTailReply(total: number, upToHour: number, gapUntil?: number): LogTailReplyWire {
  const count = Math.min(64, total);
  const rows: LogPageRow[] = [];
  for (let i = 0; i < count; i++) {
    const ord = total - 1 - i;
    rows.push(makeRow(ord, upToHour - i));
  }
  return {
    branchId: '1',
    log: 'market-events',
    columns: MARKET_COLUMNS,
    baseOrdinal: '0',
    firstOrdinal: String(total - count),
    offset: '0',
    total: String(total),
    rows,
    facetTotals: [],
    missing: gapUntil !== undefined ? [{ firstHour: BASE_HOUR + 100, lastHour: gapUntil, reason: 'Ongoing coverage gap' }] : [],
    warnings: [],
  };
}

let pendingFrame: ((now: number) => void) | null = null;
let virtualNow = 0;
const controlledEngine = new FrameEngine({
  request(run) {
    pendingFrame = run;
  },
});

function syncVirtualClock(): void {
  virtualNow = performance.now();
}

function tickFrame(dtMs: number): void {
  const base = Math.max(virtualNow, performance.now());
  virtualNow = base + dtMs;
  const run = pendingFrame;
  pendingFrame = null;
  if (run) run(virtualNow);
  else controlledEngine.tick(virtualNow);
}

let ports: readonly MessagePort[] = [];
let seek = 0;
let followResolved: (() => void) | null = null;
const followReady = new Promise<void>(resolve => { followResolved = resolve; });
const stamp = resetStamps();
const send = (bytes: Uint8Array) => {
  const resets = stamp(bytes);
  const split = splitForLanes(bytes, ports.length);
  if (split.kind !== 'failure') {
    split.lanes.forEach((part, lane) => {
      if (part) ports[lane]!.postMessage(packet(split.kind, part, resets), [part.buffer]);
    });
  }
};

const source = {
  id: 'dom-log',
  revision: 'r1',
  path: 'p',
  metadata: { branches: [{ id: '1', firstHour: BASE_HOUR, lastHour: BASE_HOUR + 4000 }] },
} as unknown as DashboardSource;

const client = {
  available: true,
  openFollow: (opened: readonly MessagePort[]) => { ports = opened; return Promise.resolve(); },
  product: async (request: { product: string }) => {
    if (request.product === 'follow') {
      return { data: { available: true, route: ROUTE, epoch: 1, generation: 1 } };
    }
    if (request.product === 'follow-clock') {
      queueMicrotask(() => followResolved?.());
      return { data: { available: true } };
    }
    return { data: { available: true } };
  },
} as unknown as DashboardClient;

const clock = new PlaybackController(controlledEngine);
clock.setBounds({ firstHour: BASE_HOUR, lastHour: BASE_HOUR + 4000 });
const workers = [0, 1].map(() => new Worker(new URL('../../draw/lane.worker.ts', import.meta.url), { type: 'module' }));
const laneListeners = new Set<(lane: number, message: FromLane) => void>();
const lanes: LaneWorker[] = workers.map((worker, laneIndex) => {
  let handler: ((event: MessageEvent<FromLane>) => void) | null = null;
  return {
    postMessage(message, transfer) {
      if (transfer) worker.postMessage(message, transfer);
      else worker.postMessage(message);
    },
    get onmessage() { return handler; },
    set onmessage(next) {
      handler = next;
      worker.onmessage = next
        ? (event: MessageEvent<FromLane>) => {
            next(event);
            for (const listener of [...laneListeners]) listener(laneIndex, event.data);
          }
        : null;
    },
  };
});

function waitForLane(predicate: (lane: number, message: FromLane) => boolean): Promise<void> {
  return new Promise<void>(resolve => {
    const listener = (lane: number, message: FromLane) => {
      if (predicate(lane, message)) {
        laneListeners.delete(listener);
        resolve();
      }
    };
    laneListeners.add(listener);
  });
}

const host: PipelineHost = {
  lanes,
  open: list => client.openFollow!(list),
  channel: () => new MessageChannel(),
  frameRate: () => 60,
  rate: () => 1,
};
const pipeline = pipelineFor(clock, client, () => host);

const runtime = {
  client,
  clock,
  tracks: [{ id: 't1', source, branch: { id: '1', firstHour: BASE_HOUR, lastHour: BASE_HOUR + 4000 }, axisOffset: 0 }],
} as unknown as WidgetRuntime;

const widget: WidgetSpec = {
  id: 'w-market-ledger',
  kind: 'market-ledger',
  configVersion: 1,
  title: 'Market ledger',
  trackIds: ['t1'],
  time: { mode: 'follow' },
  countries: { kind: 'all' },
  options: {},
} as unknown as WidgetSpec;

const OPTIONS: MarketLedgerProjectionOptions = {
  fromHour: BASE_HOUR,
  looks: {
    GER: [{ from: 0, name: 'Germany' }],
    FRA: [{ from: 0, name: 'France' }],
  },
};

const SUBSCRIPTIONS = [
  {
    name: 'tail',
    source,
    branch: '1',
    offset: 0,
    query: { panel: 'market-ledger-tail', branchId: '1', facets: [{ name: 'kind', values: [20, 21], countries: [] }], limit: 64, countFacets: [] },
    channel: 'panel' as const,
    product: 'market-ledger-tail',
  },
];

let commits = 0;
let totalInstallReads = 0;
let installs = 0;
let scrollPageFetches = 0;
let hostNode: HTMLDivElement | null = null;
let subId = -1;
let pipeId = -1;
let flushWrapped = false;
let toggleChipExternal: ((on: boolean) => void) | null = null;

function waitForInstall(predicate: () => boolean): Promise<void> {
  if (predicate()) return Promise.resolve();
  return new Promise<void>(resolve => {
    const stop = pipeline.onInstall(pipeId, () => {
      if (predicate()) {
        stop();
        resolve();
      }
    });
  });
}

function waitForDom(root: HTMLElement, predicate: () => boolean): Promise<void> {
  if (predicate()) return Promise.resolve();
  return new Promise<void>(resolve => {
    const observer = new MutationObserver(() => {
      if (predicate()) {
        observer.disconnect();
        resolve();
      }
    });
    observer.observe(root, { childList: true, subtree: true, characterData: true, attributes: true });
  });
}

function measureLayoutReadsDuring<T>(fn: () => T): { result: T; reads: number } {
  let reads = 0;
  const origGetRect = Element.prototype.getBoundingClientRect;
  const origComputed = window.getComputedStyle;
  const clientWidthDesc = Object.getOwnPropertyDescriptor(Element.prototype, 'clientWidth')!;
  const clientHeightDesc = Object.getOwnPropertyDescriptor(Element.prototype, 'clientHeight')!;
  const offsetWidthDesc = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'offsetWidth')!;
  const offsetHeightDesc = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'offsetHeight')!;
  const scrollHeightDesc = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'scrollHeight')!;

  Element.prototype.getBoundingClientRect = function (this: Element) { reads++; return origGetRect.call(this); };
  window.getComputedStyle = function (...args: Parameters<typeof window.getComputedStyle>) { reads++; return origComputed.apply(window, args); };
  Object.defineProperty(Element.prototype, 'clientWidth', { configurable: true, get() { reads++; return clientWidthDesc.get!.call(this); } });
  Object.defineProperty(Element.prototype, 'clientHeight', { configurable: true, get() { reads++; return clientHeightDesc.get!.call(this); } });
  Object.defineProperty(HTMLElement.prototype, 'offsetWidth', { configurable: true, get() { reads++; return offsetWidthDesc.get!.call(this); } });
  Object.defineProperty(HTMLElement.prototype, 'offsetHeight', { configurable: true, get() { reads++; return offsetHeightDesc.get!.call(this); } });
  Object.defineProperty(HTMLElement.prototype, 'scrollHeight', { configurable: true, get() { reads++; return scrollHeightDesc.get!.call(this); } });

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
    Object.defineProperty(HTMLElement.prototype, 'scrollHeight', scrollHeightDesc);
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

function FixtureHarness() {
  const [chipOn, setChipOn] = useState(true);
  toggleChipExternal = setChipOn;
  return (
    <FollowedWidgetKey.Provider value="dom-market-ledger">
      <Profiler id="log-family" onRender={() => { commits++; }}>
        <FamilyLogHost
          projection={marketLedgerProjection}
          options={OPTIONS}
          label="Market ledger"
          subject="market activity"
          widget={widget}
          runtime={runtime}
          active
          density="comfortable"
          months
          twoLine
          subscriptions={chipOn ? SUBSCRIPTIONS : []}
          queryKey={`dom-market:${chipOn ? 'on' : 'off'}`}
          onLoadScrollPage={async (frozenHour, offset, pageSize) => {
            scrollPageFetches++;
            return makeTailReply(Math.max(1, 200 - offset), frozenHour - pageSize);
          }}
          controls={{
            chipGroups: [{
              key: 'kinds',
              label: 'Market occurrences',
              chips: [{ value: 'deliveries', label: 'Deliveries' }],
              selected: chipOn ? ['deliveries'] : [],
              max: 1,
              onToggle: () => setChipOn(v => !v),
            }],
          }}
          contextMenuItems={(_key, hour) => [
            { label: 'Show contract', run: () => {} },
            { label: `Jump to ${hour}`, run: () => {} },
          ]}
          gapsSubject="Market activity"
          bodyBlockedState={!chipOn ? <WidgetState kind="empty" icon="history" message="Choose the occurrences to show in the widget settings." /> : null}
          offset={0}
          pin={null}
          bounds={[BASE_HOUR, BASE_HOUR + 4000]}
        />
      </Profiler>
    </FollowedWidgetKey.Provider>
  );
}

declare global {
  interface Window {
    logFamilyDom: unknown;
  }
}

window.logFamilyDom = {
  async mount() {
    commits = 0;
    totalInstallReads = 0;
    installs = 0;
    scrollPageFetches = 0;
    hostNode = document.createElement('div');
    hostNode.style.cssText = 'width:520px;height:280px;display:flex;flex-direction:column;';
    document.body.append(hostNode);
    const root = createRoot(hostNode);
    flushSync(() => root.render(<FixtureHarness />));

    await waitForDom(hostNode, () => {
      const canvas = hostNode?.querySelector<HTMLCanvasElement>('canvas.family-log-canvas');
      if (!canvas?.dataset.placement) return false;
      pipeId = Number(canvas.dataset.placement);
      return pipeId > 0;
    });

    const beforePaintMeta = pipeline.view(pipeId)?.fields?.meta as LogMetaField | undefined;
    const initialHour = beforePaintMeta?.hour ?? null;

    const internal = pipeline as unknown as { widgets: Map<number, { subs: Map<object, number> }> };
    subId = [...(internal.widgets.get(pipeId)?.subs.values() ?? [])][0]!;
    await followReady;

    clock.seek(BASE_HOUR + 100);
    seek = clock.getSnapshot().seekSequence;
    const firstPaintDone = waitForInstall(() => {
      const view = pipeline.view(pipeId);
      return view?.mode === 'picture' && view.through === BASE_HOUR + 100;
    });
    send(datedPanel(subId, BASE_HOUR + 100, makeTailReply(120, BASE_HOUR + 100), seek, ROUTE));
    await firstPaintDone;
    ensureFlushInstrumentation();
    const afterMeta = pipeline.view(pipeId)?.fields?.meta as LogMetaField | undefined;
    return {
      painted: true,
      initialHour,
      afterFirstHour: afterMeta?.hour ?? null,
      expectedFirstHour: BASE_HOUR + 100,
      initialTotal: afterMeta?.total ?? 0,
    };
  },

  async playLive(advanceHours: number) {
    commits = 0;
    totalInstallReads = 0;
    installs = 0;
    scrollPageFetches = 0;
    const startHour = BASE_HOUR + 100;
    const targetHour = startHour + advanceHours;
    let sentThrough = startHour;
    const totalFor = (h: number) => 120 + (h - startHour) * 2;
    const feed = (hour: number) => {
      while (sentThrough < hour + 6) {
        sentThrough++;
        send(datedPanel(subId, sentThrough, makeTailReply(totalFor(sentThrough), sentThrough, sentThrough), seek, ROUTE));
      }
    };
    feed(startHour);
    const stopFeed = clock.onPublish((_prev, next) => feed(next.hour));
    clock.setSpeed(24);
    clock.setPlaying(true);
    syncVirtualClock();

    const widgetLane = pipeline.lane(pipeId);
    let playingDuringAdvance = false;
    try {
      while (clock.getHour() < targetHour) {
        const nextMinTarget = clock.getHour() + 1;
        const served = waitForLane((lane, msg) => lane === widgetLane && msg.type === 'served' && msg.target >= nextMinTarget);
        tickFrame(42);
        await served;
        if (clock.getSnapshot().playing && clock.getHour() >= startHour + 4) {
          playingDuringAdvance = true;
        }
      }
      const playingBeforePause = clock.getSnapshot().playing;
      const playReads = totalInstallReads;
      const playInstalls = installs;

      totalInstallReads = 0;
      const expectedHour = clock.getHour();
      const expectedTotal = totalFor(expectedHour);
      const expectedTopKey = `m:${expectedTotal - 1}`;
      const mirrorUpdated = waitForDom(
        hostNode!,
        () => hostNode?.querySelector<HTMLDivElement>('.family-log-mirror [role="option"][data-row-key^="m:"]')?.dataset.rowKey === expectedTopKey,
      );
      clock.setPlaying(false);
      await mirrorUpdated;
      const pausedHour = Math.floor(clock.getHour());
      const pauseReads = totalInstallReads;
      const commitsThroughPause = commits;
      const afterMeta = pipeline.view(pipeId)?.fields?.meta as LogMetaField | undefined;
      const gapsText = hostNode?.querySelector('.family-log-gaps-slot')?.textContent ?? '';
      return {
        playingDuringAdvance,
        playingBeforePause,
        commitsThroughPause,
        playReads,
        pauseReads,
        playInstalls,
        scrollPageFetches,
        gapsVisible: gapsText.includes('Not recorded'),
        flushedOnPause: hostNode?.querySelector<HTMLDivElement>('.family-log-mirror [role="option"][data-row-key^="m:"]')?.dataset.rowKey === expectedTopKey,
        mirrorTopKey: hostNode?.querySelector<HTMLDivElement>('.family-log-mirror [role="option"][data-row-key^="m:"]')?.dataset.rowKey ?? null,
        expectedTopKey,
        finalTotal: afterMeta?.total ?? 0,
        expectedTotal,
        finalHour: afterMeta?.hour ?? null,
        pausedHour,
      };
    } finally {
      stopFeed();
    }
  },

  async testScrollBackAndJump() {
    const scrollEl = hostNode?.querySelector<HTMLDivElement>('.family-log-scroll');
    if (!scrollEl) throw new Error('Missing scroll container');
    const baseHour = Math.floor(clock.getHour());
    const baseMeta = pipeline.view(pipeId)?.fields?.meta as LogMetaField | undefined;
    const baseTotal = baseMeta?.total ?? 144;
    clock.setSpeed(1);
    clock.setPlaying(true);
    syncVirtualClock();

    const jumpMounted = waitForDom(hostNode!, () => !!hostNode?.querySelector('.family-log-jump'));
    scrollEl.scrollTop = 180;
    scrollEl.dispatchEvent(new Event('scroll'));
    await jumpMounted;
    const jumpBefore = hostNode?.querySelector('.family-log-jump')?.textContent ?? '';

    const jumpUpdated = waitForDom(hostNode!, () => (hostNode?.querySelector('.family-log-jump')?.textContent ?? '').includes('new'));
    commits = 0;
    const nextHour = baseHour + 1;
    send(datedPanel(subId, nextHour, makeTailReply(baseTotal + 10, nextHour), seek, ROUTE));
    send(datedPanel(subId, nextHour + 1, makeTailReply(baseTotal + 10, nextHour + 1), seek, ROUTE));
    let jumped = false;
    void jumpUpdated.then(() => { jumped = true; });
    const widgetLane = pipeline.lane(pipeId);
    while (!jumped && clock.getHour() < nextHour + 5) {
      const wantTarget = clock.getHour();
      const served = waitForLane((lane, msg) => lane === widgetLane && msg.type === 'served' && msg.target >= wantTarget);
      tickFrame(500);
      await served;
    }
    await jumpUpdated;
    const jumpUpdateCommits = commits;
    const jumpAfter = hostNode?.querySelector('.family-log-jump')?.textContent ?? '';

    const jumpCleared = waitForDom(hostNode!, () => !hostNode?.querySelector('.family-log-jump'));
    const btn = hostNode?.querySelector<HTMLButtonElement>('.family-log-jump');
    btn?.click();
    await jumpCleared;
    const scrollAfterJump = scrollEl.scrollTop;
    clock.setPlaying(false);
    return { jumpBefore, jumpAfter, jumpUpdateCommits, scrollAfterJump };
  },

  async testEnterMenuAndLastChipToolbar() {
    const scrollEl = hostNode?.querySelector<HTMLDivElement>('.family-log-scroll');
    if (!scrollEl) throw new Error('Missing scroll container');
    scrollEl.focus();
    scrollEl.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    await waitForDom(document.body, () => document.body.textContent?.includes('Show contract') ?? false);
    const hasContractAction = document.body.textContent?.includes('Show contract') ?? false;
    const hasJumpAction = document.body.textContent?.includes('Jump to ') ?? false;
    scrollEl.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));

    flushSync(() => toggleChipExternal?.(false));
    const toolbarStillMounted = !!hostNode?.querySelector('.family-log-toolbar');
    const emptyVisible = hostNode?.querySelector('.family-log-state-overlay')?.textContent?.includes('Choose the occurrences') ?? false;
    flushSync(() => toggleChipExternal?.(true));
    await waitForDom(hostNode!, () => {
      const canvas = hostNode?.querySelector<HTMLCanvasElement>('canvas.family-log-canvas');
      const nextPipe = canvas?.dataset.placement ? Number(canvas.dataset.placement) : 0;
      if (!nextPipe) return false;
      pipeId = nextPipe;
      return true;
    });
    const internal = pipeline as unknown as { widgets: Map<number, { subs: Map<object, number> }> };
    subId = [...(internal.widgets.get(pipeId)?.subs.values() ?? [])][0]!;
    pipeline.onInstall(pipeId, () => { installs++; });
    return { hasContractAction, hasJumpAction, toolbarStillMounted, emptyVisible };
  },

  async testDualPlacementAndDrillSurvival() {
    const drillHost = document.createElement('div');
    drillHost.style.cssText = 'width:400px;height:200px;display:flex;flex-direction:column;';
    document.body.append(drillHost);
    const drillRoot = createRoot(drillHost);
    const drillWidget: WidgetSpec = { ...widget, id: 'w-market-drill' };
    flushSync(() => drillRoot.render(
      <FollowedWidgetKey.Provider value="dom-market-ledger">
        <FamilyLogHost
          projection={marketLedgerProjection}
          options={OPTIONS}
          label="Deliveries"
          subject="market activity"
          widget={drillWidget}
          runtime={runtime}
          active
          density="compact"
          months={false}
          twoLine
          subscriptions={SUBSCRIPTIONS}
          queryKey="dom-market-drill"
          offset={0}
          pin={null}
          bounds={[BASE_HOUR, BASE_HOUR + 4000]}
        />
      </FollowedWidgetKey.Provider>,
    ));

    let drillPipeId = -1;
    await waitForDom(drillHost, () => {
      const canvas = drillHost.querySelector<HTMLCanvasElement>('canvas.family-log-canvas');
      if (!canvas?.dataset.placement) return false;
      drillPipeId = Number(canvas.dataset.placement);
      return drillPipeId > 0;
    });

    const distinctPlacements = drillPipeId !== pipeId;
    flushSync(() => drillRoot.unmount());
    drillHost.remove();

    const beforeInstalls = installs;
    const nextHour = Math.floor(clock.getHour()) + 1;
    const nextPainted = waitForInstall(() => {
      const meta = pipeline.view(pipeId)?.fields?.meta as LogMetaField | undefined;
      return meta?.hour === nextHour;
    });
    clock.seek(nextHour);
    seek = clock.getSnapshot().seekSequence;
    send(datedPanel(subId, nextHour, makeTailReply(180, nextHour), seek, ROUTE));
    await nextPainted;
    return {
      distinctPlacements,
      mainPipeId: pipeId,
      drillPipeId,
      mainStillInstalled: installs > beforeInstalls,
    };
  },
};
