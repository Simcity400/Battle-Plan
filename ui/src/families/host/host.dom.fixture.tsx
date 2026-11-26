import { Profiler } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { flushSync } from 'react-dom';
import { APPEARANCE_EVENT } from '../../shell/appearance/events.ts';
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
import type { InstalledAction } from '../contract.ts';
import { createLaneView } from '../laneRuntime.ts';
import { tableLane } from '../table/laneView.ts';
import { tableSettingsOf } from '../table/settings.ts';
import { SCOREBOARD_FIXTURE_COLUMNS, scoreboardColumnsSpec, scoreboardProjection } from '../table/tableFamilyDom.fixture.ts';
import { tableHostAdapter } from '../table/hostAdapter.ts';
import TableChrome from '../table/TableChrome.tsx';
import FamilyHost from './FamilyHost.tsx';
import DeclaredFamilyWidget, { type DeclaredFamilyWidgetProps } from './DeclaredFamilyWidget.tsx';
import type { FamilyPlanContext, SourcePlan } from '../contract.ts';
import type { IdentitySnapshot } from '../identity/types.ts';
import type { FamilyStaticSources } from './adapter.ts';
import type { TiledLine } from '../../dashboard/tiles/view.ts';
import type { PagedRows } from '../table/rowSource.ts';
import type { ArtResolutions } from '../art/provider.ts';
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
  product: async (request: { product: string; sourceId: string; revision: string; params: { hour?: number } }) => {
    if (request.product === 'follow') return { data: { available: true, route: NATIVE } };
    if (request.product === 'span-history') return { sourceId: request.sourceId, revision: request.revision, product: request.product, data: { through: request.params.hour } };
    return { data: { available: true } };
  },
} as unknown as DashboardClient;

const clock = new PlaybackController();
const paintsByWidget = new Map<number, number>();
let payloadTips = false;

type PaintedField = { readonly value: unknown; readonly content: string };
function asPayloadTip(field: unknown): unknown {
  const tip = field as PaintedField | undefined;
  const value = tip?.value as { key: string; text: { hour: number; rowKey: string; content: { title: string } } | null } | undefined;
  if (!tip || !value?.text) return field;
  const { hour, rowKey, content } = value.text;
  return { value: { key: value.key, text: { hour, rowKey, payload: { title: content.title } } }, content: tip.content };
}
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
      views: (_kind, wake) => {
        const view = createLaneView(tableLane, wake);
        const paint = view.paint.bind(view);
        view.paint = (input, state, content) => {
          paintsByWidget.set(input.widget, (paintsByWidget.get(input.widget) ?? 0) + 1);
          const painted = paint(input, state, content);
          if (painted.mode !== undefined && painted.mode !== 'picture') return painted;
          const fields = { ...painted.fields, accessibleDescription: { value: 'Four countries ranked by factories', content: 'description' } };
          if (payloadTips && 'focusTooltip' in fields) fields.focusTooltip = asPayloadTip(fields.focusTooltip) as typeof fields.focusTooltip;
          return { ...painted, fields };
        };
        return view;
      },
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


const { chromeState: _chrome, ...tableWithoutChrome } = tableHostAdapter;
const BODY_FOCUS_ADAPTER = { ...tableWithoutChrome, mirrorFocus: 'body' as const };

let root: Root | null = null;
let genericCommits = 0;
let genericActivated: InstalledAction[] = [];
let genericViewports: number[] = [];
const installCalls = new Map<number, number>();
const origOnInstall = pipeline.onInstall.bind(pipeline);
pipeline.onInstall = ((id: number, listener: () => void) => origOnInstall(id, () => {
  installCalls.set(id, (installCalls.get(id) ?? 0) + 1);
  listener();
})) as typeof pipeline.onInstall;
let container: HTMLDivElement | null = null;
let commits = 0;
let bindCount = 0;
let boundIds: number[] = [];
let staticCommits = 0;
let staticSupplies: { readonly id: number; readonly statics: boolean; readonly tiledHead?: string; readonly hoverHour?: number; readonly paged?: unknown; readonly art?: unknown }[] = [];
let lastBoundModel: unknown = null;
let activated: string[] = [];
let currentState = {
  source: sourceA,
  active: true,
  frameToken: 1,
  secondMounted: false,
  genericMounted: false,
  genericBlocked: false,
  genericBodyFocus: false,
  statics: null as FamilyStaticSources | null,
  tiled: undefined as Readonly<Record<string, ReadonlyMap<string, TiledLine>>> | undefined,
  hovers: undefined as Readonly<Record<string, { readonly hour: number; readonly data: unknown }>> | undefined,
  paged: undefined as PagedRows | undefined,
  spanMounted: false,
};

const origBind = pipeline.bind.bind(pipeline);
pipeline.bind = ((...args: Parameters<typeof pipeline.bind>) => {
  bindCount++;
  boundIds.push(args[0]);
  lastBoundModel = args[4];
  return origBind(...args);
}) as typeof pipeline.bind;

const origSupply = pipeline.supply.bind(pipeline);
pipeline.supply = ((...args: Parameters<typeof pipeline.supply>) => {
  const statics = args[1].statics as { tiledBySource?: { main?: { GER?: { head?: string } } }; hovers?: { hover?: { hour?: number } }; pagedRows?: { log?: { mark?: unknown } }; artResolutions?: { mark?: unknown } } | null | undefined;
  const tiledHead = statics?.tiledBySource?.main?.GER?.head, hoverHour = statics?.hovers?.hover?.hour, paged = statics?.pagedRows?.log?.mark, art = statics?.artResolutions?.mark;
  staticSupplies.push({ id: args[0], statics: args[1].statics !== undefined, ...(tiledHead !== undefined ? { tiledHead } : {}), ...(hoverHour !== undefined ? { hoverHour } : {}), ...(paged !== undefined ? { paged } : {}), ...(art !== undefined ? { art } : {}) });
  return origSupply(...args);
}) as typeof pipeline.supply;

const perf = new PerfRecorder();
measureWith(perf);

const staticArt = (() => {
  const listeners = new Set<() => void>();
  let resolutions: ArtResolutions | undefined;
  return {
    get resolutions() { return resolutions; },
    subscribe(changed: () => void) { listeners.add(changed); return () => { listeners.delete(changed); }; },
    land(next: ArtResolutions) { resolutions = next; for (const listener of listeners) listener(); },
  };
})();

const NO_EXACT = async () => null;
const NO_SUBSCRIPTIONS: ReturnType<typeof makeSubs> = [];
const BOUNDS: readonly [number, number] = [0, 2000];
const RUNTIME_BY_SOURCE = new Map<DashboardSource, WidgetRuntime>([
  [sourceA, makeRuntime(sourceA)],
  [sourceB, makeRuntime(sourceB)],
]);
function makeIdentity(sourceId: string): IdentitySnapshot {
  return {
    scope: { sourceId, revision: 'r1', branchId: '1', content: 1 },
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
}

const IDENTITY_BY_SOURCE = new Map<DashboardSource, IdentitySnapshot>([
  [sourceA, makeIdentity('camp-A')],
  [sourceB, makeIdentity('camp-B')],
]);

const SUBS_BY_SOURCE = new Map<DashboardSource, ReturnType<typeof makeSubs>>([
  [sourceA, makeSubs(sourceA)],
  [sourceB, makeSubs(sourceB)],
]);

const SPAN_ADAPTER = { ...tableHostAdapter, settings: { resolve: () => SETTINGS } };
const SPAN_DECLARATION = {
  family: 'table',
  kind: 'scoreboard',
  label: 'Span',
  subject: 'span',
  emptyCampaign: 'No campaign',
  countries: { mode: 'none' },
  sources: (ctx: FamilyPlanContext): SourcePlan => ({
    owner: 'span',
    entries: [{
      name: 'unread',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: 0,
      read: {
        shape: 'branch-to-hour',
        coverage: { mode: 'population', form: 'all' },
        product: 'span-history' as never,
        followQuery: null,
        params: { branchId: ctx.branch.id },
        buildSpanParams: (params, span) => ({ ...params, hour: span.lastHour }),
        dynamicKeys: ['hour'],
      },
    }],
  }),
  projection: scoreboardProjection,
  projectionOptions: (_ctx: FamilyPlanContext, extra: { readonly extent: readonly [number, number] }) => ({ ...OPTIONS, firstHour: extra.extent[0] }),
};

function renderApp() {
  const rt = RUNTIME_BY_SOURCE.get(currentState.source)!;
  const subs = SUBS_BY_SOURCE.get(currentState.source)!;
  const identity = IDENTITY_BY_SOURCE.get(currentState.source)!;
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
            <FamilyHost
              adapter={tableHostAdapter}
              chrome={TableChrome}
              view="default"
              declaration={scoreboardProjection}
              options={OPTIONS}
              label="Scoreboard"
              subject="scoreboard"
              widget={widget}
              runtime={rt}
              active={currentState.active}
              settings={SETTINGS}
              subscriptions={subs}
              identitySnapshot={identity}
              exactReadView={NO_EXACT}
              offset={0}
              pin={null}
              bounds={BOUNDS}
              onActivate={action => { activated.push(action.subject); }}
            />
          </div>
          {currentState.secondMounted && (
            <div data-slot="secondary" style={{ width: '460px', height: '240px', display: 'flex', flexDirection: 'column' }}>
              <FamilyHost
              adapter={tableHostAdapter}
              chrome={TableChrome}
              view="default"
                declaration={scoreboardProjection}
                options={OPTIONS}
                label="Scoreboard Secondary"
                subject="scoreboard"
                widget={widget}
                runtime={rt}
                active
                settings={SETTINGS}
                subscriptions={subs}
                identitySnapshot={identity}
                exactReadView={NO_EXACT}
                offset={0}
                pin={null}
                bounds={BOUNDS}
              />
            </div>
          )}
          {currentState.genericMounted && (
            <Profiler id="generic-host" onRender={() => { genericCommits++; }}>
              <div data-slot="generic" style={{ width: '460px', height: '240px', display: 'flex', flexDirection: 'column' }}>
                <FamilyHost
                  adapter={currentState.genericBodyFocus ? BODY_FOCUS_ADAPTER : tableHostAdapter}
                  {...(currentState.genericBodyFocus ? {} : { chrome: TableChrome })}
                  settings={SETTINGS}
                  declaration={scoreboardProjection}
                  view="default"
                  options={OPTIONS}
                  label="Scoreboard Generic"
                  subject="scoreboard"
                  widget={widget}
                  runtime={rt}
                  active
                  subscriptions={subs}
                  identitySnapshot={identity}
                  offset={0}
                  pin={null}
                  bounds={BOUNDS}
                  onActivate={action => { genericActivated.push(action); }}
                  blockedState={currentState.genericBlocked ? <div data-blocked="true">Not yet</div> : null}
                  onViewportChange={viewport => { genericViewports.push(viewport.width); }}
                />
              </div>
            </Profiler>
          )}
          {currentState.spanMounted && (
            <div data-slot="span" style={{ width: '460px', height: '240px', display: 'flex', flexDirection: 'column' }}>
              <DeclaredFamilyWidget {...({ adapter: SPAN_ADAPTER, chrome: TableChrome, declaration: SPAN_DECLARATION, widget, options: {}, runtime: rt, active: true } as unknown as DeclaredFamilyWidgetProps)} />
            </div>
          )}
          {currentState.statics && (
            <Profiler id="static-host" onRender={() => { staticCommits++; }}>
              <div data-slot="static" style={{ width: '460px', height: '240px', display: 'flex', flexDirection: 'column' }}>
                <FamilyHost
                  adapter={tableHostAdapter}
                  chrome={TableChrome}
                  settings={SETTINGS}
                  declaration={scoreboardProjection}
                  view="default"
                  options={OPTIONS}
                  label="Scoreboard Static"
                  subject="scoreboard"
                  widget={widget}
                  runtime={rt}
                  active
                  subscriptions={NO_SUBSCRIPTIONS}
                  staticSources={currentState.statics}
                  {...currentState.tiled ? { tiledSeries: currentState.tiled } : {}}
                  {...currentState.hovers ? { hovers: currentState.hovers } : {}}
                  {...currentState.paged ? { pagedRows: currentState.paged } : {}}
                  laneArt={staticArt}
                  identitySnapshot={identity}
                  offset={0}
                  pin={null}
                  bounds={BOUNDS}
                />
              </div>
            </Profiler>
          )}
          <Profiler id="tooltip-layer" onRender={() => { genericCommits++; }}>
            <TooltipLayer />
          </Profiler>
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
    await document.fonts?.ready;
    await new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
    container = document.createElement('div');
    document.body.append(container);
    root = createRoot(container);
    renderApp();
    let pipeId = -1;
    await until(() => {
      const canvas = container!.querySelector<HTMLCanvasElement>('[data-slot="primary"] canvas.fh-canvas');
      if (!canvas?.dataset.placement) return false;
      pipeId = Number(canvas.dataset.placement);
      return pipeId > 0;
    });
    const subs = subsForPipe(pipeId);
    await until(() => subs.length === 2 && subs.every(id => pipeline.status(id).status === 'followed'));
    clock.seek(20);
    seek = clock.getSnapshot().seekSequence;
    sendHour(subs, 20, [90n, 70n, 50n, 30n], true);
    await until(() => pipeline.view(pipeId)?.mode === 'picture' && (container!.querySelectorAll('[data-slot="primary"] table tbody tr').length === 4));
    return { pipeId };
  },

  bodyBox() {
    const box = container!.querySelector<HTMLElement>('[data-slot="primary"] .fh-body')!.getBoundingClientRect();
    return { left: box.left, top: box.top };
  },

  async testF1PinClearsOnCampaignSwitch() {
    const firstRow = container!.querySelector<HTMLTableRowElement>('[data-slot="primary"] table tbody tr')!;
    firstRow.focus();
    const pipeId = Number(container!.querySelector<HTMLCanvasElement>('[data-slot="primary"] canvas.fh-canvas')!.dataset.placement);
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
    const nextPipe = Number(container!.querySelector<HTMLCanvasElement>('[data-slot="primary"] canvas.fh-canvas')!.dataset.placement);
    const subs = subsForPipe(nextPipe);
    await until(() => subs.length === 2 && subs.every(id => pipeline.status(id).status === 'followed'));
    clock.seek(25);
    seek = clock.getSnapshot().seekSequence;
    sendHour(subs, 25, [90n, 70n, 50n, 30n], true);
    await until(() => pipeline.view(nextPipe)?.mode === 'picture' && !pipeline.view(nextPipe)?.dim);
    return { pinnedBefore, cleared: afterSwitch === null };
  },

  async testF2LeaveThenDeactivateClearsHover() {
    const pipeId = Number(container!.querySelector<HTMLCanvasElement>('[data-slot="primary"] canvas.fh-canvas')!.dataset.placement);
    const tipVisible = await until(() => tooltipStore.get() !== null, 2000);
    const bodyEl = container!.querySelector<HTMLElement>('[data-slot="primary"] .fh-body')!;
    bodyEl.dispatchEvent(new PointerEvent('pointerleave', { bubbles: false }));
    currentState = { ...currentState, active: false };
    renderApp();
    const afterDeactivate = tooltipStore.get();

    currentState = { ...currentState, active: true };
    renderApp();
    let newPipe = -1;
    await until(() => {
      const c = container!.querySelector<HTMLCanvasElement>('[data-slot="primary"] canvas.fh-canvas');
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
    const pipeId = Number(container!.querySelector<HTMLCanvasElement>('[data-slot="primary"] canvas.fh-canvas')!.dataset.placement);
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

  async testGenericHostFrames() {
    currentState = { ...currentState, genericMounted: true };
    renderApp();
    let pipeG = -1;
    await until(() => {
      const canvas = container!.querySelector<HTMLCanvasElement>('[data-slot="generic"] canvas.fh-canvas');
      if (!canvas?.dataset.placement) return false;
      pipeG = Number(canvas.dataset.placement);
      return pipeG > 0;
    });
    const subsG = subsForPipe(pipeG);
    await until(() => subsG.length === 2 && subsG.every(id => pipeline.status(id).status === 'followed'));
    clock.seek(100);
    seek = clock.getSnapshot().seekSequence;
    sendHour(subsG, 100, [90n, 70n, 50n, 30n], true);
    const firstPicture = await until(() => pipeline.view(pipeG)?.mode === 'picture' && !pipeline.view(pipeG)?.dim
      && container!.querySelectorAll('[data-slot="generic"] table tbody tr').length === 4);

    const frames = async (count: number) => {
      for (let i = 0; i < count; i++) await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
    };
    await frames(4);
    const mountCommits = genericCommits;
    genericCommits = 0;
    bindCount = 0;
    paintsByWidget.set(pipeG, 0);
    clock.setSpeed(24);
    clock.setPlaying(true);
    await frames(2);
    const startCommits = genericCommits;
    const startBinds = bindCount;
    const startPaints = paintsByWidget.get(pipeG) ?? 0;
    await frames(30);
    const quiet = {
      hoursAdvanced: clock.getSnapshot().hour - 100,
      commits: genericCommits - startCommits,
      binds: bindCount - startBinds,
      paints: (paintsByWidget.get(pipeG) ?? 0) - startPaints,
    };

    const steadyStartCommits = genericCommits;
    const steadyStartBinds = bindCount;
    const steadyStartPaints = paintsByWidget.get(pipeG) ?? 0;
    let factories = 90n;
    for (let i = 0; i < 20; i++) {
      factories += 5n;
      sendHour(subsG, clock.getSnapshot().hour, [factories, 70n, 50n, 30n]);
      await frames(1);
    }
    await frames(4);
    const steady = {
      commits: genericCommits - steadyStartCommits,
      binds: bindCount - steadyStartBinds,
      paints: (paintsByWidget.get(pipeG) ?? 0) - steadyStartPaints,
    };

    clock.setPlaying(false);
    await frames(4);
    const settledBinds = bindCount;
    const settledPaints = paintsByWidget.get(pipeG) ?? 0;
    const settledCommits = genericCommits;
    await frames(20);
    const paused = {
      commits: genericCommits - settledCommits,
      binds: bindCount - settledBinds,
      paints: (paintsByWidget.get(pipeG) ?? 0) - settledPaints,
    };

    sendHour(subsG, clock.getSnapshot().hour, [factories, 70n, 50n, 30n]);
    await until(() => pipeline.view(pipeG)?.dim === false && pipeline.view(pipeG)?.h === clock.getSnapshot().hour, 2000);
    genericActivated = [];
    const row = container!.querySelector<HTMLElement>('[data-slot="generic"] table tbody tr')!;
    row.focus();
    row.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    const installed = pipeline.view(pipeG);
    const activated = genericActivated[0] ?? null;

    currentState = { ...currentState, genericMounted: false };
    renderApp();
    return {
      firstPicture,
      mountCommits,
      quiet,
      steady,
      paused,
      activatedSubject: activated?.subject ?? null,
      activatedMatchesInstalled: !!activated && activated.installedId === installed?.id,
    };
  },

  async testGenericStationaryTooltip() {
    const frames = async (count: number) => {
      for (let i = 0; i < count; i++) await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
    };
    payloadTips = true;
    currentState = { ...currentState, genericMounted: true };
    renderApp();
    let pipeG = -1;
    await until(() => {
      const canvas = container!.querySelector<HTMLCanvasElement>('[data-slot="generic"] canvas.fh-canvas');
      if (!canvas?.dataset.placement) return false;
      pipeG = Number(canvas.dataset.placement);
      return pipeG > 0;
    });
    const subsG = subsForPipe(pipeG);
    await until(() => subsG.length === 2 && subsG.every(id => pipeline.status(id).status === 'followed'));
    clock.seek(150);
    seek = clock.getSnapshot().seekSequence;
    sendHour(subsG, 150, [90n, 70n, 50n, 30n], true);
    await until(() => pipeline.view(pipeG)?.mode === 'picture' && !pipeline.view(pipeG)?.dim
      && container!.querySelectorAll('[data-slot="generic"] table tbody tr').length === 4);
    container!.querySelector<HTMLElement>('[data-slot="generic"] table tbody tr[data-row-key="ENG"]')!.focus();
    const shown = await until(() => tooltipStore.get()?.content.title === 'United Kingdom', 2000);
    await frames(4);

    let storeUpdates = 0;
    const unsubscribe = tooltipStore.subscribe(() => { storeUpdates++; });
    genericCommits = 0;
    const installsBefore = installCalls.get(pipeG) ?? 0;
    for (let i = 0; i < 20; i++) {
      const before = installCalls.get(pipeG);
      sendHour(subsG, 150, [91n + BigInt(i), 70n, 50n, 30n]);
      if (!await until(() => installCalls.get(pipeG) !== before)) throw new Error('The changed Germany row did not install');
    }
    const installs = (installCalls.get(pipeG) ?? 0) - installsBefore;
    const commitsDuring = genericCommits;
    unsubscribe();

    (document.activeElement as HTMLElement | null)?.blur();
    payloadTips = false;
    currentState = { ...currentState, genericMounted: false };
    renderApp();
    return { shown, installs, storeUpdates, commits: commitsDuring };
  },

  async testGenericInstalledTuple() {
    const frames = async (count: number) => {
      for (let i = 0; i < count; i++) await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
    };
    genericViewports = [];
    currentState = { ...currentState, genericMounted: true, genericBlocked: true, genericBodyFocus: true };
    renderApp();
    await frames(3);
    const blockedMeasured = genericViewports.some(width => width > 0);
    const blockedShown = !!container!.querySelector('[data-slot="generic"] [data-blocked="true"]');

    currentState = { ...currentState, genericBlocked: false };
    renderApp();
    let pipeG = -1;
    await until(() => {
      const canvas = container!.querySelector<HTMLCanvasElement>('[data-slot="generic"] canvas.fh-canvas');
      if (!canvas?.dataset.placement) return false;
      pipeG = Number(canvas.dataset.placement);
      return pipeG > 0;
    });
    const subsG = subsForPipe(pipeG);
    await until(() => subsG.length === 2 && subsG.every(id => pipeline.status(id).status === 'followed'));
    clock.seek(200);
    seek = clock.getSnapshot().seekSequence;
    sendHour(subsG, 200, [90n, 70n, 50n, 30n], true);
    await until(() => pipeline.view(pipeG)?.mode === 'picture' && !pipeline.view(pipeG)?.dim);
    await frames(4);
    const body = container!.querySelector<HTMLElement>('[data-slot="generic"] .fh-body')!;
    const describedBy = body.getAttribute('aria-describedby');
    const description = describedBy ? document.getElementById(describedBy)?.textContent ?? null : null;

    body.focus();
    await frames(2);
    body.blur();
    await frames(2);
    clock.setSpeed(24);
    clock.setPlaying(true);
    for (let i = 0; i < 40; i++) {
      sendHour(subsG, clock.getSnapshot().hour, [90n, 70n, 50n, 30n]);
      await frames(1);
    }
    genericActivated = [];
    body.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    const view = pipeline.view(pipeG);
    const activated = genericActivated[0] ?? null;
    clock.setPlaying(false);

    currentState = { ...currentState, genericBlocked: true };
    renderApp();
    await frames(2);
    const clearedDescription = body.getAttribute('aria-describedby') === null;

    currentState = { ...currentState, genericMounted: false, genericBlocked: false, genericBodyFocus: false };
    renderApp();
    return {
      blockedMeasured,
      blockedShown,
      description,
      clearedDescription,
      activatedId: activated?.installedId ?? null,
      activatedHour: activated?.hour ?? null,
      viewId: view?.id ?? null,
      viewHour: view?.painted ?? null,
    };
  },

  async testStaticPanelsReplaced() {
    const frames = async (count: number) => {
      for (let i = 0; i < count; i++) await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
    };
    const stamped = (round: number): FamilyStaticSources => ({ stamp: {}, proof: null, panels: { unread: { round } }, hour: 40 });
    currentState = { ...currentState, statics: stamped(0) };
    renderApp();
    let pipeS = -1;
    await until(() => {
      const canvas = container!.querySelector<HTMLCanvasElement>('[data-slot="static"] canvas.fh-canvas');
      if (!canvas?.dataset.placement) return false;
      pipeS = Number(canvas.dataset.placement);
      return pipeS > 0;
    });
    await frames(4);
    const generation = pipeline.binding(pipeS);
    staticCommits = 0;
    bindCount = 0;
    staticSupplies = [];
    const rounds = 10;
    for (let round = 1; round <= rounds; round++) {
      currentState = { ...currentState, statics: stamped(round) };
      renderApp();
      await frames(2);
    }
    const result = {
      rounds,
      commits: staticCommits,
      binds: bindCount,
      staticsSupplied: staticSupplies.filter(item => item.id === pipeS && item.statics).length,
      generationKept: pipeline.binding(pipeS) === generation,
      mode: `${pipeline.view(pipeS)?.mode} ${pipeline.view(pipeS)?.reason ?? ''}`,
    };
    currentState = { ...currentState, statics: null };
    renderApp();
    return result;
  },

  async testDataSupplied() {
    const frames = async (count: number) => {
      for (let i = 0; i < count; i++) await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
    };
    const tiledLine = (round: number) => ({ tag: 'GER', head: `GER:${round}`, decimals: 0, summary: { from: 0, to: 100 + round, width: 10, buckets: [] }, missing: [], parts: [], marks: null }) as unknown as TiledLine;
    currentState = { ...currentState, statics: { stamp: {}, proof: null, panels: { unread: { round: 0 } }, hour: 40 }, tiled: { main: new Map([['GER', tiledLine(0)]]) } };
    renderApp();
    let pipeS = -1;
    await until(() => {
      const canvas = container!.querySelector<HTMLCanvasElement>('[data-slot="static"] canvas.fh-canvas');
      if (!canvas?.dataset.placement) return false;
      pipeS = Number(canvas.dataset.placement);
      return pipeS > 0;
    });
    await frames(4);
    const generation = pipeline.binding(pipeS);
    bindCount = 0;
    staticSupplies = [];
    const rounds = 10;
    for (let round = 1; round <= rounds; round++) {
      currentState = {
        ...currentState,
        tiled: { main: new Map([['GER', tiledLine(round)]]) },
        hovers: { hover: { hour: round, data: { round } } },
        paged: { log: { mark: round } } as unknown as PagedRows,
      };
      renderApp();
      await frames(2);
      staticArt.land({ mark: round } as unknown as ArtResolutions);
      await frames(2);
    }
    const ours = staticSupplies.filter(item => item.id === pipeS && item.statics);
    const result = {
      rounds,
      binds: bindCount,
      supplies: ours.length,
      tilesSupplied: [...new Set(ours.map(item => item.tiledHead ?? null))],
      hoursSupplied: [...new Set(ours.map(item => item.hoverHour ?? null))],
      pagesSupplied: [...new Set(ours.map(item => item.paged ?? null))],
      artSupplied: [...new Set(ours.map(item => item.art ?? null))].filter(mark => mark !== null),
      generationKept: pipeline.binding(pipeS) === generation,
    };
    currentState = { ...currentState, statics: null, tiled: undefined, hovers: undefined, paged: undefined };
    renderApp();
    return result;
  },

  async testSpanEndOnly() {
    const frames = async (count: number) => {
      for (let i = 0; i < count; i++) await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
    };
    clock.setPlaying(false);
    clock.seek(300);
    currentState = { ...currentState, spanMounted: true };
    renderApp();
    let pipeS = -1;
    await until(() => {
      const canvas = container!.querySelector<HTMLCanvasElement>('[data-slot="span"] canvas.fh-canvas');
      if (!canvas?.dataset.placement) return false;
      pipeS = Number(canvas.dataset.placement);
      return pipeS > 0;
    });
    await until(() => staticSupplies.some(item => item.id === pipeS && item.statics));
    await frames(4);
    boundIds = [];
    staticSupplies = [];
    const from = clock.getSnapshot().hour;
    clock.setSpeed(24);
    clock.setPlaying(true);
    await frames(60);
    const result = {
      hoursAdvanced: clock.getSnapshot().hour - from,
      answers: staticSupplies.filter(item => item.id === pipeS && item.statics).length,
      binds: boundIds.filter(id => id === pipeS).length,
      mode: `${pipeline.view(pipeS)?.mode} ${pipeline.view(pipeS)?.reason ?? ''}`,
    };
    clock.setPlaying(false);
    await frames(4);
    currentState = { ...currentState, spanMounted: false };
    renderApp();
    return result;
  },

  async testF12DualPlacementAndTypeahead() {
    currentState = { ...currentState, secondMounted: true };
    renderApp();
    let pipeA = -1;
    let pipeB = -1;
    await until(() => {
      const cA = container!.querySelector<HTMLCanvasElement>('[data-slot="primary"] canvas.fh-canvas');
      const cB = container!.querySelector<HTMLCanvasElement>('[data-slot="secondary"] canvas.fh-canvas');
      if (!cA?.dataset.placement || !cB?.dataset.placement) return false;
      pipeA = Number(cA.dataset.placement);
      pipeB = Number(cB.dataset.placement);
      return pipeA > 0 && pipeB > 0;
    });
    const cA = container!.querySelector<HTMLCanvasElement>('[data-slot="primary"] canvas.fh-canvas')!;
    const cB = container!.querySelector<HTMLCanvasElement>('[data-slot="secondary"] canvas.fh-canvas')!;
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

    const firstRow = container!.querySelector<HTMLTableRowElement>('[data-slot="primary"] table tbody tr')!;
    firstRow.focus();
    firstRow.dispatchEvent(new KeyboardEvent('keydown', { key: 'i', bubbles: true }));
    const typeaheadKey = await until(() => (document.activeElement as HTMLElement | null)?.getAttribute('data-row-key') === 'ITA', 2000)
      ? (document.activeElement as HTMLElement | null)?.getAttribute('data-row-key')
      : null;

    return { distinctCanvases, distinctPlacements, hasReaderA, hasReaderB, primaryStillInstalls, typeaheadKey };
  },
};
