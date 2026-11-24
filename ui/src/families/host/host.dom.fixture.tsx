import { Profiler } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { flushSync } from 'react-dom';
import { APPEARANCE_EVENT } from '../../charts/tokens.ts';
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
import { WidgetFrameContext } from '../../dashboard/widgetFrame.tsx';
import type { FromLane, ToLane } from '../../draw/laneMessages';
import { laneDispatch } from '../../draw/laneDispatch.ts';
import { measureWith, PerfRecorder } from '../../shell/perf.ts';
import { familyTableLaneView } from '../table/laneView.ts';
import { tableSettingsOf } from '../table/settings.ts';
import { SCOREBOARD_FIXTURE_COLUMNS, scoreboardColumnsSpec, scoreboardProjection } from '../table/tableFamilyDomShared.ts';
import FamilyTableHost from '../table/FamilyTableHost.tsx';
import '../../styles.css';

const NATIVE = 9;

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

function makeSource(id: string): DashboardSource {
  return {
    id,
    revision: 'r1',
    path: id,
    metadata: { branches: [{ id: '1', firstHour: 0, lastHour: 2000 }] },
  } as unknown as DashboardSource;
}

const sourceA = makeSource('camp-A');
const sourceB = makeSource('camp-B');

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

function createLane(): LaneWorker {
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
      views: (_kind, wake) => familyTableLaneView(wake),
      crop: sources => Promise.all(sources.map(source => createImageBitmap(source as OffscreenCanvas))),
      later: run => requestAnimationFrame(() => run()),
    },
  );
  return worker;
}

const workers = [0, 1].map(() => createLane());
const host: PipelineHost = {
  lanes: workers,
  open: list => client.openFollow!(list),
  channel: () => new MessageChannel(),
  frameRate: () => 60,
  rate: () => 1,
};
const pipeline = pipelineFor(clock, client, () => host);

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

function makeRuntime(src: DashboardSource): WidgetRuntime {
  return {
    client,
    clock,
    tracks: [{ id: 't1', source: src, branch: { id: '1', firstHour: 0, lastHour: 2000 }, axisOffset: 0 }],
  } as unknown as WidgetRuntime;
}

function makeSubs(src: DashboardSource) {
  return [
    { name: 'factories', source: src, branch: '1', offset: 0, query: { product: 'series-state', params: { branchId: '1', col: 'factories' } }, channel: 'followed' as const },
    { name: 'casualties', source: src, branch: '1', offset: 0, query: { product: 'series-state', params: { branchId: '1', col: 'casualties' } }, channel: 'followed' as const },
  ];
}

const widget: WidgetSpec = {
  id: 'w-host-test',
  kind: 'scoreboard',
  trackIds: ['t1'],
  time: { mode: 'follow' },
  countries: { kind: 'all' },
  options: {},
  layout: { x: 0, y: 0, w: 12, h: 12 },
} as unknown as WidgetSpec;

const nextPoll = () => new Promise<void>(resolve => setTimeout(resolve, 16));
async function until(predicate: () => boolean, limitMs = 5000) {
  const deadline = performance.now() + limitMs;
  while (!predicate() && performance.now() < deadline) await nextPoll();
  return predicate();
}

function sendHour(subIds: readonly number[], hour: number, fac: [bigint, bigint, bigint, bigint], reset = false) {
  const names = ['GER', 'FRA', 'ITA', 'ENG'];
  const keys = [1n, 2n, 3n, 4n];
  const facBody = answerBody({ keys, columns: 1, names, values: [...fac], states: [0, 0, 0, 0] });
  const casBody = answerBody({ keys, columns: 1, names, values: [10n, 20n, 30n, 40n], states: [0, 0, 0, 0] });
  const answers: { id: number; hour: number; body: Uint8Array }[] = [];
  for (let i = 0; i < subIds.length; i += 2) {
    answers.push({ id: subIds[i]!, hour, body: facBody });
    answers.push({ id: subIds[i + 1]!, hour, body: casBody });
  }
  send(followMessage(hour, seek, answers, reset));
}

function subsForPipe(pipeId: number): number[] {
  const internal = pipeline as unknown as { widgets: Map<number, { subs: Map<object, number> }> };
  return [...(internal.widgets.get(pipeId)?.subs.values() ?? [])];
}

let root: Root | null = null;
let container: HTMLDivElement | null = null;
let commits = 0;
let bindCount = 0;
let lastBoundModel: unknown = null;
let activated: string[] = [];
let currentState = {
  source: sourceA,
  active: true,
  frameToken: 1,
  secondMounted: false,
};

const origBind = pipeline.bind.bind(pipeline);
pipeline.bind = ((...args: Parameters<typeof pipeline.bind>) => {
  bindCount++;
  lastBoundModel = args[4];
  return origBind(...args);
}) as typeof pipeline.bind;

const perf = new PerfRecorder();
measureWith(perf);

const NO_EXACT = async () => null;
const BOUNDS: readonly [number, number] = [0, 2000];
const RUNTIME_BY_SOURCE = new Map<DashboardSource, WidgetRuntime>([
  [sourceA, makeRuntime(sourceA)],
  [sourceB, makeRuntime(sourceB)],
]);
const SUBS_BY_SOURCE = new Map<DashboardSource, ReturnType<typeof makeSubs>>([
  [sourceA, makeSubs(sourceA)],
  [sourceB, makeSubs(sourceB)],
]);

function renderApp() {
  const rt = RUNTIME_BY_SOURCE.get(currentState.source)!;
  const subs = SUBS_BY_SOURCE.get(currentState.source)!;
  const frameCtx = {
    token: currentState.frameToken,
    announce: () => {},
    clearAnnouncement: () => {},
    setSkeleton: () => {},
    reportState: () => {},
    setDynamicViewSwitch: () => {},
  };
  flushSync(() => root!.render(
    <FollowedWidgetKey.Provider value="shared-widget-key">
      <WidgetFrameContext.Provider value={frameCtx as never}>
        <Profiler id="host-dom" onRender={() => { commits++; }}>
          <div data-slot="primary" style={{ width: '460px', height: '240px', display: 'flex', flexDirection: 'column' }}>
            <FamilyTableHost
              declaration={scoreboardProjection}
              options={OPTIONS}
              label="Scoreboard"
              subject="scoreboard"
              widget={widget}
              runtime={rt}
              active={currentState.active}
              columns={SPECS}
              settings={SETTINGS}
              subscriptions={subs}
              exactSources={NO_EXACT}
              offset={0}
              pin={null}
              bounds={BOUNDS}
              onActivate={key => { activated.push(key); }}
            />
          </div>
          {currentState.secondMounted && (
            <div data-slot="secondary" style={{ width: '460px', height: '240px', display: 'flex', flexDirection: 'column' }}>
              <FamilyTableHost
                declaration={scoreboardProjection}
                options={OPTIONS}
                label="Scoreboard Secondary"
                subject="scoreboard"
                widget={widget}
                runtime={rt}
                active
                columns={SPECS}
                settings={SETTINGS}
                subscriptions={subs}
                exactSources={NO_EXACT}
                offset={0}
                pin={null}
                bounds={BOUNDS}
              />
            </div>
          )}
          <TooltipLayer />
        </Profiler>
      </WidgetFrameContext.Provider>
    </FollowedWidgetKey.Provider>,
  ));
}

declare global {
  interface Window {
    hostDom: unknown;
  }
}

window.hostDom = {
  async mountPrimary() {
    container = document.createElement('div');
    document.body.append(container);
    root = createRoot(container);
    renderApp();
    let pipeId = -1;
    await until(() => {
      const canvas = container!.querySelector<HTMLCanvasElement>('[data-slot="primary"] canvas.tf-canvas');
      if (!canvas?.dataset.placement) return false;
      pipeId = Number(canvas.dataset.placement);
      return pipeId > 0;
    });
    const subs = subsForPipe(pipeId);
    await until(() => subs.length === 2 && subs.every(id => pipeline.status(id).status === 'followed'));
    clock.seek(20);
    seek = clock.getSnapshot().seekSequence;
    sendHour(subs, 20, [90n, 70n, 50n, 30n], true);
    await until(() => pipeline.view(pipeId)?.mode === 'picture' && (container!.querySelectorAll('[data-slot="primary"] .table-accessibility tbody tr').length === 4));
    return { pipeId };
  },

  bodyBox() {
    const box = container!.querySelector<HTMLElement>('[data-slot="primary"] .tf-body')!.getBoundingClientRect();
    return { left: box.left, top: box.top };
  },

  async testF1PinClearsOnCampaignSwitch() {
    const firstRow = container!.querySelector<HTMLTableRowElement>('[data-slot="primary"] .table-accessibility tbody tr')!;
    firstRow.focus();
    const pipeId = Number(container!.querySelector<HTMLCanvasElement>('[data-slot="primary"] canvas.tf-canvas')!.dataset.placement);
    await until(() => !!(pipeline.view(pipeId)?.fields?.focusTooltip as { text?: unknown } | undefined)?.text, 2000);
    firstRow.dispatchEvent(new KeyboardEvent('keydown', { key: 'p', bubbles: true }));
    await until(() => (tooltipStore.get()?.content.subtitle ?? '').startsWith('Pinned at '), 2000);
    const pinnedBefore = tooltipStore.get()?.content.title ?? null;

    currentState = { ...currentState, source: sourceB };
    renderApp();
    await until(() => tooltipStore.get() === null, 2000);
    const afterSwitch = tooltipStore.get();

    currentState = { ...currentState, source: sourceA };
    renderApp();
    const nextPipe = Number(container!.querySelector<HTMLCanvasElement>('[data-slot="primary"] canvas.tf-canvas')!.dataset.placement);
    const subs = subsForPipe(nextPipe);
    await until(() => subs.length === 2 && subs.every(id => pipeline.status(id).status === 'followed'));
    clock.seek(25);
    seek = clock.getSnapshot().seekSequence;
    sendHour(subs, 25, [90n, 70n, 50n, 30n], true);
    await until(() => pipeline.view(nextPipe)?.mode === 'picture' && !pipeline.view(nextPipe)?.dim);
    return { pinnedBefore, cleared: afterSwitch === null };
  },

  async testF2LeaveThenDeactivateClearsHover() {
    const pipeId = Number(container!.querySelector<HTMLCanvasElement>('[data-slot="primary"] canvas.tf-canvas')!.dataset.placement);
    const tipVisible = await until(() => tooltipStore.get() !== null, 2000);
    const bodyEl = container!.querySelector<HTMLElement>('[data-slot="primary"] .tf-body')!;
    bodyEl.dispatchEvent(new PointerEvent('pointerleave', { bubbles: false }));
    currentState = { ...currentState, active: false };
    renderApp();
    const afterDeactivate = tooltipStore.get();

    currentState = { ...currentState, active: true };
    renderApp();
    let newPipe = -1;
    await until(() => {
      const c = container!.querySelector<HTMLCanvasElement>('[data-slot="primary"] canvas.tf-canvas');
      if (!c?.dataset.placement) return false;
      newPipe = Number(c.dataset.placement);
      return newPipe > 0;
    });
    const subs = subsForPipe(newPipe);
    await until(() => subs.length === 2 && subs.every(id => pipeline.status(id).status === 'followed'));
    clock.seek(30);
    seek = clock.getSnapshot().seekSequence;
    sendHour(subs, 30, [90n, 70n, 50n, 30n], true);
    await until(() => pipeline.view(newPipe)?.mode === 'picture' && !pipeline.view(newPipe)?.dim);
    return { tipVisible, clearedOnDeactivate: afterDeactivate === null, oldPipe: pipeId, newPipe };
  },

  async testF3UnrelatedAppearanceAndF4ReducedMotion() {
    commits = 0;
    bindCount = 0;
    window.dispatchEvent(new Event(APPEARANCE_EVENT));
    await nextPoll();
    const unrelatedCommits = commits;
    const unrelatedBinds = bindCount;

    document.documentElement.dataset.motion = 'reduced';
    window.dispatchEvent(new Event(APPEARANCE_EVENT));
    await until(() => (lastBoundModel as { reducedMotion?: boolean } | null)?.reducedMotion === true, 2000);
    const bindsAfterToggle = bindCount;
    const boundReduced = (lastBoundModel as { reducedMotion?: boolean } | null)?.reducedMotion === true;

    delete document.documentElement.dataset.motion;
    window.dispatchEvent(new Event(APPEARANCE_EVENT));
    await until(() => (lastBoundModel as { reducedMotion?: boolean } | null)?.reducedMotion === false, 2000);
    return { unrelatedCommits, unrelatedBinds, bindsAfterToggle, boundReduced };
  },

  async waitInspected(expectedKey: string) {
    const pipeId = Number(container!.querySelector<HTMLCanvasElement>('[data-slot="primary"] canvas.tf-canvas')!.dataset.placement);
    return until(() => {
      const tip = pipeline.view(pipeId)?.fields?.tooltip as { text?: { rowKey?: string } | null } | undefined;
      return tip?.text?.rowKey === expectedKey;
    }, 2000);
  },

  async testF5FrameUpdateDuringPress() {
    activated = [];
    currentState = { ...currentState, frameToken: currentState.frameToken + 1 };
    renderApp();
    return { frameToken: currentState.frameToken };
  },

  async waitActivated() {
    await until(() => activated.length > 0, 2000);
    return [...activated];
  },

  async testF12DualPlacementAndTypeahead() {
    currentState = { ...currentState, secondMounted: true };
    renderApp();
    let pipeA = -1;
    let pipeB = -1;
    await until(() => {
      const cA = container!.querySelector<HTMLCanvasElement>('[data-slot="primary"] canvas.tf-canvas');
      const cB = container!.querySelector<HTMLCanvasElement>('[data-slot="secondary"] canvas.tf-canvas');
      if (!cA?.dataset.placement || !cB?.dataset.placement) return false;
      pipeA = Number(cA.dataset.placement);
      pipeB = Number(cB.dataset.placement);
      return pipeA > 0 && pipeB > 0;
    });
    const cA = container!.querySelector<HTMLCanvasElement>('[data-slot="primary"] canvas.tf-canvas')!;
    const cB = container!.querySelector<HTMLCanvasElement>('[data-slot="secondary"] canvas.tf-canvas')!;
    const distinctCanvases = cA !== cB;
    const distinctPlacements = pipeA !== pipeB;
    const readers = (perf as unknown as { readers: Map<string, unknown> }).readers;
    const hasReaderA = readers.has(`${pipeline.generation}/${pipeA}/-1`);
    const hasReaderB = readers.has(`${pipeline.generation}/${pipeB}/-1`);

    const subsB = subsForPipe(pipeB);
    await until(() => subsB.length === 2 && subsB.every(id => pipeline.status(id).status === 'followed'));
    clock.seek(35);
    seek = clock.getSnapshot().seekSequence;
    sendHour([...subsForPipe(pipeA), ...subsB], 35, [90n, 70n, 50n, 30n], true);
    await until(() => pipeline.view(pipeB)?.h === 35 && pipeline.view(pipeA)?.h === 35);

    currentState = { ...currentState, secondMounted: false };
    renderApp();
    await nextPoll();
    const subsA = subsForPipe(pipeA);
    clock.seek(36);
    seek = clock.getSnapshot().seekSequence;
    sendHour(subsA, 36, [95n, 70n, 50n, 30n], true);
    const primaryStillInstalls = await until(() => pipeline.view(pipeA)?.h === 36 && !pipeline.view(pipeA)?.dim, 2000);

    const firstRow = container!.querySelector<HTMLTableRowElement>('[data-slot="primary"] .table-accessibility tbody tr')!;
    firstRow.focus();
    firstRow.dispatchEvent(new KeyboardEvent('keydown', { key: 'i', bubbles: true }));
    const typeaheadKey = await until(() => (document.activeElement as HTMLElement | null)?.getAttribute('data-row-key') === 'ITA', 2000)
      ? (document.activeElement as HTMLElement | null)?.getAttribute('data-row-key')
      : null;

    return { distinctCanvases, distinctPlacements, hasReaderA, hasReaderB, primaryStillInstalls, typeaheadKey };
  },
};
