import { createElement, useLayoutEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { createPortal } from 'react-dom';
import '../styles.css';
import './dashboard.css';
import '../charts/charts.css';
import '../charts/drill/drill.css';
import '../charts/table/table.css';
import '../widgets/army-losses-by-month/armyLosses.css';
import DashboardGrid from './DashboardGrid.tsx';
import WidgetPreview from './WidgetPreview.tsx';
import { registerWidgets } from './definitions.ts';
import type { WidgetProps, WidgetRuntime } from './definitions.ts';
import { MapViewsStore } from './geometry.ts';
import type { BoundTrack, DashboardTemplate, WidgetSpec } from './model.ts';
import { PlaybackController } from './playback.ts';
import type { FrameEngine } from '../engine/frame.ts';
import { useWidgetViewSwitch, WidgetCaption } from './widgetFrame.tsx';
import WidgetState from '../components/WidgetState.tsx';
import { useHeadProduct } from './headProduct.ts';
import { useGate } from './regionGate.ts';
import { SPEC } from './regionTime.ts';
import { classifyFailure, ShownError } from './queryError.ts';
import { useProduct } from './products.ts';
import { productRequest } from './request.ts';
import ActiveResearchWidget from '../widgets/active-research/ActiveResearchWidget.tsx';
import ScoreboardWidget from '../widgets/scoreboard/ScoreboardWidget.tsx';
import FactionTotalsView from '../charts/race/FactionTotalsView.tsx';

declare global { interface Window { captionLayout: unknown } }

function createVirtualTimers() {
  const realSetTimeout = window.setTimeout.bind(window);
  const realClearTimeout = window.clearTimeout.bind(window);
  let now = 0;
  let nextId = 1;
  const tasks = new Map<number, { at: number; fn: () => void }>();
  window.setTimeout = ((handler: TimerHandler, timeout = 0, ...args: unknown[]) => {
    const id = nextId++;
    const fn = typeof handler === 'function' ? () => (handler as (...a: unknown[]) => void)(...args) : () => {};
    tasks.set(id, { at: now + Math.max(0, Number(timeout) || 0), fn });
    return id;
  }) as unknown as typeof window.setTimeout;
  window.clearTimeout = ((id: number) => { tasks.delete(id); }) as unknown as typeof window.clearTimeout;
  return {
    advance(ms: number) {
      const target = now + ms;
      while (true) {
        let earliest: [number, { at: number; fn: () => void }] | null = null;
        for (const entry of tasks.entries()) {
          if (entry[1].at <= target && (!earliest || entry[1].at < earliest[1].at)) earliest = entry;
        }
        if (!earliest) break;
        tasks.delete(earliest[0]);
        now = earliest[1].at;
        earliest[1].fn();
      }
      now = target;
    },
    restore() {
      window.setTimeout = realSetTimeout;
      window.clearTimeout = realClearTimeout;
    },
  };
}

function MeasureBody({ caption, empty, layout }: { caption: string | null; empty: boolean; layout: 'windowed' | 'chart' | 'table' }) {
  const body = useRef<HTMLDivElement>(null);
  const [height, setHeight] = useState(0);
  useLayoutEffect(() => {
    const element = body.current;
    if (!element) return;
    const measure = () => setHeight(element.clientHeight);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [empty, layout]);
  return <>
    {caption && <WidgetCaption>{caption}</WidgetCaption>}
    {empty
      ? <WidgetState kind="empty" message="No recorded losses yet." />
      : layout === 'chart'
        ? <div ref={body} className="caption-test-body army-losses">
          <div className="drill-host">
            <div className="drill-face drill-main">
              <div className="army-losses-body">
                <div className="army-losses-chart"><div className="time-chart" /></div>
              </div>
            </div>
          </div>
        </div>
        : layout === 'table'
          ? <div ref={body} className="caption-test-body chart-table">
            <div style={{ height: Math.max(0, height - 8) }} />
          </div>
          : <div ref={body} className="caption-test-body" style={{ display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0, overflow: 'hidden' }}>
            <div className="drill-host">
              <div className="drill-face drill-main">
                <div className="windowed-rows">
                  <div className="windowed-rows-body" style={{ height }} />
                </div>
              </div>
            </div>
          </div>}
  </>;
}

const Body = ({ widget }: WidgetProps) => createElement(MeasureBody, {
  caption: typeof widget.options?.caption === 'string' ? widget.options.caption : null,
  empty: widget.options?.empty === true,
  layout: widget.options?.layout === 'chart' || widget.options?.layout === 'table' ? widget.options.layout : 'windowed',
});

let switchClicks: string[] = [];
function DynamicSwitchBody({ widget, setOption }: WidgetProps) {
  const version = String(widget.options?.version ?? 'v1');
  useWidgetViewSwitch(setOption ? {
    label: 'Mode',
    value: 'a',
    options: [{ value: 'a', label: 'Alpha' }, { value: 'b', label: 'Beta' }],
    onChange: next => { switchClicks.push(`${version}:${next}`); },
  } : null);
  return <div className="dynamic-switch-body">{version}</div>;
}

function FrameProbeBody({ widget, headerSlot }: WidgetProps) {
  const pill = typeof widget.options?.pill === 'string' ? widget.options.pill : null;
  const held = typeof widget.options?.held === 'string' ? widget.options.held : undefined;
  const mode = typeof widget.options?.mode === 'string' ? widget.options.mode : 'picture';
  return <>
    {headerSlot && pill !== null && createPortal(<span className="pill">{pill}</span>, headerSlot)}
    {mode === 'empty'
      ? <WidgetState kind="empty" message="Nothing here yet." />
      : mode === 'loading'
        ? <WidgetState kind="loading" label="Loading probe…" />
        : mode === 'error'
          ? <WidgetState kind="error" what="probe" failure={new Error('Synthetic failure')} onRetry={() => {}} />
          : mode === 'nothing-chosen'
            ? <WidgetState kind="nothing-chosen" prompt="Choose a country." />
            : <div className="frame-probe-body" data-held={held}>Picture</div>}
  </>;
}

function MapBadgeProbeBody({ widget }: WidgetProps) {
  const held = typeof widget.options?.held === 'string' ? widget.options.held : undefined;
  return <div className="dash-map-widget" data-held={held}><div className="dash-map-surface" /></div>;
}

const headRetryState: { retry: (() => void) | null; error: unknown } = { retry: null, error: null };
function HeadRetryProbeBody({ widget, runtime, active }: WidgetProps) {
  const gate = useGate(runtime, widget, SPEC, active);
  const head = useHeadProduct<{ ok: boolean }, 'research-slots'>(runtime, gate, 'research-slots', gate.track ? { country: 'GER' } as never : null, {
    active,
    validate: value => value as { ok: boolean },
  });
  headRetryState.retry = head.retry;
  headRetryState.error = head.error ?? null;
  if (head.error) return <WidgetState kind="error" what="research" failure={head.error} onRetry={head.retry} />;
  return <WidgetState kind="loading" label="Loading research…" />;
}

let nextRemountToken = 0;
const remountProbeState: { token: number; retry: (() => void) | null } = { token: 0, retry: null };
function RemountProbeBody({ widget, runtime, active }: WidgetProps) {
  const token = useRef(0);
  if (!token.current) token.current = ++nextRemountToken;
  remountProbeState.token = token.current;
  const gate = useGate(runtime, widget, SPEC, active);
  const head = useHeadProduct<{ ok: boolean }, 'research-slots'>(runtime, gate, 'research-slots', gate.track ? { country: 'GER' } as never : null, {
    active,
    validate: value => value as { ok: boolean },
  });
  remountProbeState.retry = head.retry;
  if (head.error) return <WidgetState kind="error" what="probe" failure={head.error} onRetry={head.retry} />;
  return <WidgetState kind="loading" label="Loading probe…" />;
}

const siblingRetryState: { retry: (() => void) | null } = { retry: null };
function SiblingRetryProbeBody({ widget, runtime, active }: WidgetProps) {
  const gate = useGate(runtime, widget, SPEC, active);
  const track = gate.track;
  const catalogue = useProduct<{ ok: boolean }>(runtime.client, track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'equipment-stockpile-catalogue', params: { branchId: track.branch.id, holders: [{ country: 'GER' }], included: [] } }) : null, active, v => v as { ok: boolean });
  const at = useHeadProduct<{ ok: boolean }, 'equipment-stockpile-at'>(runtime, gate, 'equipment-stockpile-at', track ? { country: 'GER' } as never : null, {
    active,
    validate: v => v as { ok: boolean },
  });
  const retryAll = () => { catalogue.retry(); at.retry(); };
  siblingRetryState.retry = retryAll;
  const err = catalogue.error ?? at.error;
  if (err) return <WidgetState kind="error" what="stockpile" failure={err} onRetry={retryAll} />;
  return <div className="sibling-retry-ok">Ready</div>;
}

function FactionRestartProbeBody(props: WidgetProps) {
  return <FactionTotalsView {...props} options={props.widget.options ?? {}} kind="country-factories" measure="total" label="Factions" measureLabel="Factories" format={v => String(v)} />;
}

registerWidgets([{
  kind: 'caption-layout-test',
  version: 1,
  title: 'Supply health',
  icon: 'shield',
  description: 'Layout test widget.',
  data: true,
  history: true,
  countrySelection: false,
  countries: { max: 1 },
  defaultCountries: { kind: 'all' },
  timeModes: ['follow'],
  defaultTime: 'follow',
  scope: 'history',
  minSize: { w: 2, h: 2 },
  defaultSize: { w: 6, h: 6 },
  render: Body,
}, {
  kind: 'dynamic-switch-test',
  version: 1,
  title: 'Dynamic switch',
  icon: 'grid',
  description: 'Dynamic switch callback freshness test.',
  data: true,
  history: true,
  countrySelection: false,
  defaultCountries: { kind: 'all' },
  timeModes: ['follow'],
  defaultTime: 'follow',
  scope: 'history',
  minSize: { w: 2, h: 2 },
  defaultSize: { w: 6, h: 6 },
  render: DynamicSwitchBody,
}, {
  kind: 'header-drop-test',
  version: 1,
  title: 'Army losses',
  icon: 'grid',
  description: 'Header drop measurement test.',
  data: true,
  history: true,
  countrySelection: true,
  countries: { max: 1 },
  defaultCountries: { kind: 'tags', tags: ['GER'] },
  timeModes: ['follow', 'campaign'],
  defaultTime: 'follow',
  scope: 'history',
  options: [{
    key: 'view', section: 'options', label: 'View', kind: 'choice', default: 'wide',
    choices: [
      { value: 'wide', label: 'Very long breakdown label' },
      { value: 'alt', label: 'Second breakdown label' },
    ],
  }],
  viewSwitch: { option: 'view' },
  minSize: { w: 2, h: 2 },
  defaultSize: { w: 6, h: 6 },
  playsBackDated: true,
  render: FrameProbeBody,
}, {
  kind: 'frame-probe-test',
  version: 1,
  title: 'Frame probe',
  icon: 'grid',
  description: 'Reading badge and announcer test.',
  data: true,
  history: true,
  countrySelection: true,
  countries: { max: 1 },
  defaultCountries: { kind: 'all' },
  timeModes: ['follow'],
  defaultTime: 'follow',
  scope: 'history',
  minSize: { w: 2, h: 2 },
  defaultSize: { w: 6, h: 6 },
  playsBackDated: true,
  render: FrameProbeBody,
}, {
  kind: 'map',
  version: 1,
  title: 'Campaign map',
  icon: 'map',
  description: 'Map badge placement test.',
  data: true,
  history: false,
  countrySelection: true,
  countries: { max: 1 },
  defaultCountries: { kind: 'all' },
  timeModes: ['follow'],
  defaultTime: 'follow',
  scope: 'moment',
  minSize: { w: 2, h: 2 },
  defaultSize: { w: 6, h: 6 },
  playsBackDated: true,
  render: MapBadgeProbeBody,
}, {
  kind: 'head-retry-test',
  version: 1,
  title: 'Head retry',
  icon: 'grid',
  description: 'Head product follow retry test.',
  data: true,
  history: true,
  countrySelection: false,
  defaultCountries: { kind: 'all' },
  timeModes: ['follow'],
  defaultTime: 'follow',
  scope: 'moment',
  minSize: { w: 2, h: 2 },
  defaultSize: { w: 6, h: 6 },
  playsBackDated: true,
  render: HeadRetryProbeBody,
}, {
  kind: 'remount-probe-test',
  version: 1,
  title: 'Remount probe',
  icon: 'grid',
  description: 'Body remount on pipeline restart test.',
  data: true,
  history: true,
  countrySelection: false,
  defaultCountries: { kind: 'all' },
  timeModes: ['follow'],
  defaultTime: 'follow',
  scope: 'history',
  minSize: { w: 2, h: 2 },
  defaultSize: { w: 6, h: 6 },
  playsBackDated: true,
  render: RemountProbeBody,
}, {
  kind: 'scoreboard-restart-test',
  version: 1,
  title: 'Scoreboard',
  icon: 'grid',
  description: 'Scoreboard restart test.',
  data: true,
  history: true,
  countrySelection: true,
  defaultCountries: { kind: 'tags', tags: ['GER'] },
  timeModes: ['follow'],
  defaultTime: 'follow',
  scope: 'history',
  minSize: { w: 2, h: 2 },
  defaultSize: { w: 6, h: 6 },
  playsBackDated: true,
  render: ScoreboardWidget,
}, {
  kind: 'faction-restart-test',
  version: 1,
  title: 'Factions',
  icon: 'shield',
  description: 'Faction totals restart test.',
  data: true,
  history: true,
  countrySelection: true,
  defaultCountries: { kind: 'factions', count: 5, membership: 'end' },
  timeModes: ['follow'],
  defaultTime: 'follow',
  scope: 'history',
  minSize: { w: 2, h: 2 },
  defaultSize: { w: 6, h: 6 },
  playsBackDated: true,
  render: FactionRestartProbeBody,
}, {
  kind: 'sibling-retry-test',
  version: 1,
  title: 'Sibling retry',
  icon: 'grid',
  description: 'Healthy followed sibling retry test.',
  data: true,
  history: true,
  countrySelection: false,
  defaultCountries: { kind: 'all' },
  timeModes: ['follow'],
  defaultTime: 'follow',
  scope: 'history',
  minSize: { w: 2, h: 2 },
  defaultSize: { w: 6, h: 6 },
  playsBackDated: true,
  render: SiblingRetryProbeBody,
}]);

const branch = { id: 'b1', sessionId: 'b1', firstHour: 43800000, lastHour: 43800200, main: true, fork: null, joins: [], sampleBoundary: '', gaps: [] };
const tracks: BoundTrack[] = [{
  id: 't1', roleId: 'primary', label: 'Primary', branchIndex: 0, anchorOffset: null, axisOffset: 0,
  selectedTag: 'GER', selectedSlot: null,
  source: {
    id: 's1', path: 'synthetic', revision: '1',
    metadata: {
      path: 'synthetic', name: 'Test', format: 145, captureMode: 'full', campaignKey: 'test',
      branches: [branch],
      countries: [{ id: '1', tag: 'GER', name: 'German Reich', observedBranchId: 'b1', observedHour: 43800100 }],
      playerCountryId: null, contentVerified: true, partial: null, warnings: [], buildWarnings: [], revision: '1',
    },
  },
  branch,
} as unknown as BoundTrack];

const makeClock = () => {
  const clock = new PlaybackController({ every: () => ({ wake() {}, stop() {} }) } as unknown as FrameEngine);
  clock.setBounds({ firstHour: 43800000, lastHour: 43800200 });
  clock.seek(43800100);
  return clock;
};

const baseClient = { available: true, setMapViews: async () => {}, onMapStates: async () => () => {} };

const makeRuntime = (clock: PlaybackController, loadedTracks: BoundTrack[] = tracks, clientOverride: Record<string, unknown> = baseClient) => ({
  tracks: loadedTracks,
  campaignTracks: loadedTracks,
  defaultRoleId: 'primary',
  alignment: 'calendar',
  campaigns: [],
  recordings: [],
  maps: new MapViewsStore(clientOverride as never, clock),
  client: clientOverride,
  clock,
} as unknown as WidgetRuntime);

const settle = () => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
async function waitFor<T>(read: () => T | null, host: HTMLElement): Promise<T> {
  for (let frame = 0; frame < 60; frame++) {
    const value = read();
    if (value) return value;
    await settle();
  }
  throw new Error(`widget body never mounted: ${host.innerHTML.slice(0, 600)}`);
}

window.captionLayout = {
  async measure(caption: string | null, empty = false, layout: 'windowed' | 'chart' | 'table' = 'windowed') {
    document.body.innerHTML = '';
    const host = document.createElement('div');
    host.className = 'dash-grid-scroll';
    host.style.width = '1280px';
    host.style.height = '400px';
    document.body.appendChild(host);
    const clock = makeClock();
    const runtime = makeRuntime(clock);
    const widget: WidgetSpec = {
      id: 'w1', kind: 'caption-layout-test', configVersion: 1, title: 'Supply health', campaign: 'default',
      trackIds: ['t1'], time: { mode: 'follow' }, countries: { kind: 'all' },
      options: { ...(caption ? { caption } : {}), ...(empty ? { empty: true } : {}), layout },
    };
    const template: DashboardTemplate = {
      version: 1, name: 'Test', alignment: 'calendar',
      roles: [{ id: 'primary', label: 'Primary', selectedTag: null }],
      tracks: [{ id: 't1', roleId: 'primary', label: 'Primary', branchIndex: 0, anchorOffset: null }],
      widgets: [widget],
      layouts: { lg: [{ i: 'w1', x: 0, y: 0, w: 6, h: 8 }] },
    };
    let caught = '';
    const root = createRoot(host, { onUncaughtError: error => { caught = error instanceof Error ? `${error.message}\n${error.stack}` : String(error); } });
    root.render(createElement(DashboardGrid, { template, runtime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
    await waitFor(() => {
      if (caught) throw new Error(caught);
      return host.querySelector<HTMLElement>(empty ? '.widget-state' : '.caption-test-body');
    }, host);
    const article = host.querySelector<HTMLElement>('article.dash-widget')!;
    await settle();
    await settle();
    const header = article.querySelector<HTMLElement>('.dash-widget-header')!;
    const slot = article.querySelector<HTMLElement>('.dash-widget-caption-slot')!;
    const content = article.querySelector<HTMLElement>('.dash-widget-content')!;
    const body = article.querySelector<HTMLElement>('.caption-test-body');
    const scroller = article.querySelector<HTMLElement>('.windowed-rows, .chart-table');
    const result = {
      slotOutsideContent: slot.parentElement === article && !content.contains(slot),
      articleHeight: article.getBoundingClientRect().height,
      headerHeight: header.getBoundingClientRect().height,
      captionHeight: slot.getBoundingClientRect().height,
      contentHeight: content.getBoundingClientRect().height,
      bodyHeight: body ? body.getBoundingClientRect().height : 0,
      bodyClientWidth: body ? body.clientWidth : 0,
      bodyScrollHeight: body ? body.scrollHeight : 0,
      bodyClientHeight: body ? body.clientHeight : 0,
      contentScrollHeight: content.scrollHeight,
      contentClientHeight: content.clientHeight,
      scrollerScrollHeight: scroller ? scroller.scrollHeight : 0,
      scrollerClientHeight: scroller ? scroller.clientHeight : 0,
      scrollerClientWidth: scroller ? scroller.clientWidth : 0,
    };
    root.unmount();
    return result;
  },

  async testDynamicSwitchFreshness() {
    switchClicks = [];
    document.body.innerHTML = '';
    const host = document.createElement('div');
    host.className = 'dash-grid-scroll';
    host.style.width = '1280px';
    host.style.height = '400px';
    document.body.appendChild(host);
    const clock = makeClock();
    const runtime = makeRuntime(clock);
    const makeTemplate = (version: string): DashboardTemplate => ({
      version: 1, name: 'Test', alignment: 'calendar',
      roles: [{ id: 'primary', label: 'Primary', selectedTag: null }],
      tracks: [{ id: 't1', roleId: 'primary', label: 'Primary', branchIndex: 0, anchorOffset: null }],
      widgets: [{
        id: 'sw1', kind: 'dynamic-switch-test', configVersion: 1, title: 'Dynamic switch', campaign: 'default',
        trackIds: ['t1'], time: { mode: 'follow' }, countries: { kind: 'all' }, options: { version },
      }],
      layouts: { lg: [{ i: 'sw1', x: 0, y: 0, w: 6, h: 8 }] },
    });
    const root = createRoot(host);
    root.render(createElement(DashboardGrid, { template: makeTemplate('v1'), runtime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
    await waitFor(() => host.querySelector('.dash-view-switch button'), host);
    root.render(createElement(DashboardGrid, { template: makeTemplate('v2'), runtime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
    await settle();
    const buttons = [...host.querySelectorAll<HTMLButtonElement>('.dash-view-switch button')];
    buttons[1]?.click();
    await settle();
    const previewHost = document.createElement('div');
    previewHost.style.width = '500px';
    document.body.appendChild(previewHost);
    const previewRoot = createRoot(previewHost);
    const { widgetDefinition } = await import('./definitions.ts');
    previewRoot.render(createElement(WidgetPreview, { definition: widgetDefinition('dynamic-switch-test')!, runtime }));
    await settle();
    await settle();
    const previewHasDynamicSwitch = !!previewHost.querySelector('[data-view-switch]');
    const previewSwitchButtons = [...previewHost.querySelectorAll<HTMLButtonElement>('.dash-view-switch button')];
    const previewSwitchDisabled = previewSwitchButtons.length > 0 && previewSwitchButtons.every(b => b.disabled);
    previewRoot.unmount();
    root.unmount();
    return { switchClicks: [...switchClicks], previewHasDynamicSwitch, previewSwitchDisabled };
  },

  async testHeaderDropStability() {
    document.body.innerHTML = '';
    const host = document.createElement('div');
    host.className = 'dash-grid-scroll';
    host.style.width = '420px';
    host.style.height = '400px';
    document.body.appendChild(host);
    const clock = makeClock();
    const unloadedRuntime = makeRuntime(clock, []);
    const makeTemplate = (widthCols: number, options: Record<string, string> = {}): DashboardTemplate => ({
      version: 1, name: 'Test', alignment: 'calendar',
      roles: [{ id: 'primary', label: 'Primary', selectedTag: null }],
      tracks: [{ id: 't1', roleId: 'primary', label: 'Primary', branchIndex: 0, anchorOffset: null }],
      widgets: [{
        id: 'hd1', kind: 'header-drop-test', configVersion: 1, title: 'Army losses', campaign: 'default',
        trackIds: ['t1'], time: { mode: 'follow' }, countries: { kind: 'tags', tags: ['GER'] }, options,
      }],
      layouts: { lg: [{ i: 'hd1', x: 0, y: 0, w: widthCols, h: 8 }], md: [{ i: 'hd1', x: 0, y: 0, w: widthCols, h: 8 }], sm: [{ i: 'hd1', x: 0, y: 0, w: widthCols, h: 8 }], xs: [{ i: 'hd1', x: 0, y: 0, w: widthCols, h: 8 }] },
    });
    const root = createRoot(host);
    root.render(createElement(DashboardGrid, { template: makeTemplate(12), runtime: unloadedRuntime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
    const header = await waitFor(() => host.querySelector<HTMLElement>('.dash-widget-header'), host);
    await settle();
    const stages: string[] = [];
    const mo = new MutationObserver(() => { stages.push(header.dataset.headerStage ?? ''); });
    mo.observe(header, { attributes: true, attributeFilter: ['data-header-stage'] });
    for (let i = 0; i < 6; i++) await settle();
    mo.disconnect();
    const unloadedOscillations = stages.length;

    host.style.width = '480px';
    const loadedRuntime = makeRuntime(clock, tracks);
    root.render(createElement(DashboardGrid, { template: makeTemplate(12), runtime: loadedRuntime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
    await waitFor(() => host.querySelector('.dash-widget-country-button'), host);
    await settle();
    await settle();
    const menuTrigger = header.querySelector<HTMLElement>('.dash-view-menu-trigger[data-view-switch]');
    const triggerWidth = menuTrigger ? Math.ceil(menuTrigger.getBoundingClientRect().width) : 0;
    const loadedStage = Number(header.dataset.headerStage);
    const subjectCompact = !!header.querySelector('.dash-widget-country-button.is-compact');

    host.style.width = '840px';
    root.render(createElement(DashboardGrid, { template: makeTemplate(24, { pill: '1' }), runtime: loadedRuntime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
    await waitFor(() => header.querySelector('.dash-widget-summary .pill'), host);
    await settle();
    await settle();
    const stageBeforePlay = Number(header.dataset.headerStage);
    clock.setPlaying(true);
    root.render(createElement(DashboardGrid, { template: makeTemplate(24, { pill: '123,456,789 completed items' }), runtime: loadedRuntime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
    await settle();
    await settle();
    const stageAfterContentGrowWhilePlaying = Number(header.dataset.headerStage);
    clock.setPlaying(false);
    await settle();
    await settle();
    const stageAfterPauseFlush = Number(header.dataset.headerStage);

    host.style.width = '900px';
    root.render(createElement(DashboardGrid, { template: makeTemplate(24, { pill: '1 completed' }), runtime: loadedRuntime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
    await settle();
    await settle();
    const wideStage = Number(header.dataset.headerStage);
    clock.setPlaying(true);
    host.style.width = '420px';
    await settle();
    await settle();
    const stageAfterResizeWhilePlaying = Number(header.dataset.headerStage);
    host.style.width = '180px';
    await settle();
    await settle();
    const titleTabIndexAfterPlayResize = header.querySelector<HTMLElement>('h2')?.tabIndex ?? -1;
    host.style.width = '520px';
    await settle();
    await settle();
    const titleTabIndexAfterStageUnclip = header.querySelector<HTMLElement>('h2')?.tabIndex ?? -1;
    clock.setPlaying(false);
    root.unmount();
    return {
      unloadedOscillations, triggerWidth, loadedStage, subjectCompact,
      stageBeforePlay, stageAfterContentGrowWhilePlaying, stageAfterPauseFlush,
      wideStage, stageAfterResizeWhilePlaying, titleTabIndexAfterPlayResize, titleTabIndexAfterStageUnclip,
    };
  },

  async testReadingBadgeAndAnnouncer() {
    document.body.innerHTML = '';
    const host = document.createElement('div');
    host.className = 'dash-grid-scroll';
    host.style.width = '900px';
    host.style.height = '400px';
    document.body.appendChild(host);
    const clock = makeClock();
    const runtime = makeRuntime(clock, tracks);
    const makeTemplate = (options: Record<string, string>, kind = 'frame-probe-test'): DashboardTemplate => ({
      version: 1, name: 'Test', alignment: 'calendar',
      roles: [{ id: 'primary', label: 'Primary', selectedTag: null }],
      tracks: [{ id: 't1', roleId: 'primary', label: 'Primary', branchIndex: 0, anchorOffset: null }],
      widgets: [{
        id: 'fp1', kind, configVersion: 1, title: kind === 'map' ? 'Campaign map' : 'Frame probe', campaign: 'default',
        trackIds: ['t1'], time: { mode: 'follow' }, countries: { kind: 'all' }, options,
      }],
      layouts: { lg: [{ i: 'fp1', x: 0, y: 0, w: 6, h: 8 }] },
    });
    const timers = createVirtualTimers();
    const root = createRoot(host);
    try {
      root.render(createElement(DashboardGrid, { template: makeTemplate({ mode: 'loading' }), runtime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
      await waitFor(() => host.querySelector('[data-state="loading"]'), host);
      await settle();
      const article = host.querySelector<HTMLElement>('article.dash-widget')!;
      const loadingRole = article.querySelector('[data-state="loading"]')?.getAttribute('role') ?? null;
      const loadingAnnouncement = article.querySelector('.sr-only[aria-live="polite"]')?.textContent ?? '';

      root.render(createElement(DashboardGrid, { template: makeTemplate({ mode: 'error' }), runtime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
      await waitFor(() => article.querySelector('[data-state="error"]'), host);
      await settle();
      const announcedDuringError = article.querySelector('.sr-only[aria-live="polite"]')?.textContent ?? '';
      root.render(createElement(DashboardGrid, { template: makeTemplate({ mode: 'loading' }), runtime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
      await waitFor(() => article.querySelector('[data-state="loading"]'), host);
      await settle();
      const loadingAfterRetryError = article.querySelector('.sr-only[aria-live="polite"]')?.textContent ?? '';

      root.render(createElement(DashboardGrid, { template: makeTemplate({ mode: 'picture', held: 'stale' }), runtime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
      await waitFor(() => article.querySelector('[data-held="stale"]'), host);
      timers.advance(220);
      await settle();
      const badgeBeforeThreshold = article.querySelector('.dash-reading-badge')?.textContent ?? null;
      root.render(createElement(DashboardGrid, { template: makeTemplate({ mode: 'picture', held: 'stale', tick: '1' }), runtime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
      await settle();
      timers.advance(200);
      await settle();
      const badgeAfterRerender = article.querySelector('.dash-reading-badge')?.textContent ?? null;
      const badgeRole = article.querySelector('.dash-reading-badge')?.getAttribute('role') ?? null;
      const badgeAnnouncement = article.querySelector('.sr-only[aria-live="polite"]')?.textContent ?? '';

      clock.setPlaying(true);
      root.render(createElement(DashboardGrid, { template: makeTemplate({ mode: 'picture', held: 'behind' }), runtime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
      await settle();
      timers.advance(420);
      await settle();
      const behindBadge = article.querySelector('.dash-reading-badge')?.textContent ?? null;
      clock.setPlaying(false);
      await settle();
      const pausedAfterBehindBadge = article.querySelector('.dash-reading-badge')?.textContent ?? null;

      root.render(createElement(DashboardGrid, { template: makeTemplate({ mode: 'empty' }), runtime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
      await waitFor(() => article.querySelector('[data-state="empty"]'), host);
      await settle();
      const firstEmptyAnnounced = article.querySelector('.sr-only[aria-live="polite"]')?.textContent ?? '';
      root.render(createElement(DashboardGrid, { template: makeTemplate({ mode: 'picture' }), runtime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
      await waitFor(() => article.querySelector('.frame-probe-body'), host);
      await settle();
      const clearedOnPicture = article.querySelector('.sr-only[aria-live="polite"]')?.textContent ?? '';
      root.render(createElement(DashboardGrid, { template: makeTemplate({ mode: 'empty' }), runtime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
      await waitFor(() => article.querySelector('[data-state="empty"]'), host);
      await settle();
      const repeatedEmptyAnnounced = article.querySelector('.sr-only[aria-live="polite"]')?.textContent ?? '';

      root.render(createElement(DashboardGrid, { template: makeTemplate({ held: 'behind' }, 'map'), runtime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
      await waitFor(() => host.querySelector('.dash-map-surface'), host);
      timers.advance(420);
      await settle();
      const mapBadge = host.querySelector<HTMLElement>('.dash-reading-badge');
      const mapSurface = host.querySelector<HTMLElement>('.dash-map-surface')!;
      const mapBadgeOutsideSurface = !!mapBadge && mapBadge.getBoundingClientRect().bottom <= mapSurface.getBoundingClientRect().top + 1;

      const unloadedRuntime = makeRuntime(clock, []);
      root.render(createElement(DashboardGrid, { template: makeTemplate({ mode: 'nothing-chosen' }), runtime: unloadedRuntime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
      await waitFor(() => host.querySelector('[data-state="empty"]'), host);
      await settle();
      const chooseButtonWhenUnloaded = [...host.querySelectorAll('button')].some(b => b.textContent?.includes('Choose country'));
      root.render(createElement(DashboardGrid, { template: makeTemplate({ mode: 'nothing-chosen' }), runtime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
      await settle();
      const chooseButtonWhenLoaded = [...host.querySelectorAll('button')].some(b => b.textContent?.includes('Choose country'));

      return {
        loadingRole, loadingAnnouncement, announcedDuringError, loadingAfterRetryError,
        badgeBeforeThreshold, badgeAfterRerender, badgeRole, badgeAnnouncement, behindBadge, pausedAfterBehindBadge,
        firstEmptyAnnounced, clearedOnPicture, repeatedEmptyAnnounced,
        mapBadgeOutsideSurface, chooseButtonWhenUnloaded, chooseButtonWhenLoaded,
      };
    } finally {
      root.unmount();
      timers.restore();
    }
  },

  async testHeadProductFollowRetry() {
    headRetryState.retry = null;
    headRetryState.error = null;
    document.body.innerHTML = '';
    const host = document.createElement('div');
    host.className = 'dash-grid-scroll';
    host.style.width = '900px';
    host.style.height = '400px';
    document.body.appendChild(host);
    const clock = makeClock();
    let followCalls = 0;
    let openCalls = 0;
    let releaseRetryFollow: (() => void) | null = null;
    let notifySecondFollow: (() => void) | null = null;
    const secondFollowStarted = new Promise<void>(resolve => { notifySecondFollow = resolve; });
    const mockClient = {
      available: true,
      setMapViews: async () => {},
      onMapStates: async () => () => {},
      openFollow: async (ports: readonly MessagePort[]) => {
        openCalls++;
        const owned = structuredClone([...ports], { transfer: [...ports] });
        return { session: `page-head-retry:${openCalls}`, close: () => { for (const p of owned) p.close(); } };
      },
      product: async (req: { product: string }) => {
        if (req.product === 'follow') {
          followCalls++;
          if (followCalls === 1) throw new ShownError('Desktop panel channel busy.', 'panel follow 503');
          notifySecondFollow?.();
          await new Promise<void>(resolve => { releaseRetryFollow = resolve; });
          return { data: { available: true, route: 7 } };
        }
        return { data: { available: true } };
      },
    };
    const runtime = makeRuntime(clock, tracks, mockClient);
    const template: DashboardTemplate = {
      version: 1, name: 'Test', alignment: 'calendar',
      roles: [{ id: 'primary', label: 'Primary', selectedTag: null }],
      tracks: [{ id: 't1', roleId: 'primary', label: 'Primary', branchIndex: 0, anchorOffset: null }],
      widgets: [{
        id: 'hr1', kind: 'head-retry-test', configVersion: 1, title: 'Head retry', campaign: 'default',
        trackIds: ['t1'], time: { mode: 'follow' }, countries: { kind: 'all' }, options: {},
      }],
      layouts: { lg: [{ i: 'hr1', x: 0, y: 0, w: 6, h: 8 }] },
    };
    const root = createRoot(host);
    try {
      root.render(createElement(DashboardGrid, { template, runtime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
      await waitFor(() => host.querySelector('[data-state="error"]'), host);
      await settle();
      const firstError = classifyFailure(headRetryState.error as never);
      const callsBeforeRetry = followCalls;
      (headRetryState.retry as (() => void) | null)?.();
      await secondFollowStarted;
      await settle();
      const errorWhileInFlight = !!host.querySelector('[data-state="error"]');
      const loadingWhileInFlight = !!host.querySelector('[data-state="loading"]');
      (releaseRetryFollow as (() => void) | null)?.();
      await settle();
      const callsAfterRetry = followCalls;
      return { firstError, callsBeforeRetry, callsAfterRetry, openCalls, errorWhileInFlight, loadingWhileInFlight };
    } finally {
      (releaseRetryFollow as (() => void) | null)?.();
      root.unmount();
    }
  },

  async testActiveResearchPopupOnEmpty() {
    document.body.innerHTML = '';
    const host = document.createElement('div');
    host.className = 'dash-grid-scroll';
    host.style.width = '900px';
    host.style.height = '400px';
    document.body.appendChild(host);
    const clock = makeClock();
    let releaseHour150: (() => void) | null = null;
    let notifyHour150Started: (() => void) | null = null;
    const hour150Started = new Promise<void>(resolve => { notifyHour150Started = resolve; });
    const mockClient = {
      available: true,
      setMapViews: async () => {},
      onMapStates: async () => () => {},
      product: async (req: { sourceId: string; revision: string; product: string; params: Record<string, unknown> }) => {
        if (req.product === 'research') {
          return {
            sourceId: req.sourceId, revision: req.revision, product: req.product,
            data: {
              history: {
                coverage: [{ firstHour: 43800000, lastHour: 43800200 }],
                slots: [{
                  index: 0,
                  availability: [{ firstHour: 43800000, lastHour: 43800110 }],
                  entries: [{
                    token: 'tech_a', level: 0, technologyHandle: 1, databaseIndex: 1,
                    continuityStartHour: 43800000, firstObservedHour: 43800000, lastObservedHour: 43800110,
                    startedAfterHour: null, endedByHour: 43800110, outcomeObservedHour: 43800110, outcome: 'completed',
                  }],
                }],
              },
              definitions: [{ token: 'tech_a', databaseIndex: 1, name: 'Synthetic Tech', description: null }],
              artwork: [],
              folders: [],
              technologyFolders: [],
              warnings: [],
            },
          };
        }
        if (req.product === 'research-slots') {
          const hour = Number(req.params.hour ?? 43800100);
          if (hour === 43800150) {
            notifyHour150Started?.();
            await new Promise<void>(resolve => { releaseHour150 = resolve; });
          }
          if (hour === 43800160) {
            throw new ShownError('Research slot read failed.', 'slots 503');
          }
          const slots = hour <= 43800110
            ? [{
                index: 0,
                state: {
                  firstHour: 43800000, lastHour: 43800110,
                  token: 'tech_a', databaseIndex: 1, entryFirstObservedHour: 43800000,
                  speedMultiplierRaw: '100000', bankedDaysRaw: '0', appliedDaysRaw: '0',
                  aheadReductionYearsRaw: null, details: null,
                },
                progress: null,
                definition: null,
              }]
            : [];
          return {
            sourceId: req.sourceId, revision: req.revision, product: req.product,
            data: { tag: 'GER', readHour: hour, slots, missing: null, warnings: [], grants: { nextId: null, unavailable: null, grants: [] } },
          };
        }
        if (req.product === 'technology-effects') {
          return { sourceId: req.sourceId, revision: req.revision, product: req.product, data: { hour: Number(req.params.hour ?? 43800100), runs: [], refusals: {}, unavailable: null, warnings: [] } };
        }
        if (req.product === 'research-breakdown') {
          return { sourceId: req.sourceId, revision: req.revision, product: req.product, data: { parts: [] } };
        }
        return { sourceId: req.sourceId, revision: req.revision, product: req.product, data: { artwork: [], warnings: [] } };
      },
    };
    const runtime = makeRuntime(clock, tracks, mockClient);
    const makeWidget = (offset: number): WidgetSpec => ({
      id: 'ar1', kind: 'active-research', configVersion: 1, title: 'Active research', campaign: 'default',
      trackIds: ['t1'], time: { mode: 'pin', windows: { t1: { firstOffset: offset, lastOffset: offset } } },
      countries: { kind: 'tags', tags: ['GER'] }, options: {},
    });
    const root = createRoot(host);
    try {
      root.render(createElement(ActiveResearchWidget, { widget: makeWidget(100), runtime, active: true, mapOptions: { fog: true, country: null }, headerSlot: null, onDiagnostics() {} }));
      const rowName = await waitFor(() => host.querySelector<HTMLElement>('.active-row-name'), host);
      rowName.click();
      const panelBefore = await waitFor(() => document.querySelector<HTMLElement>('.research-window'), host);
      await settle();
      const openBeforeEmpty = !!panelBefore;
      panelBefore.style.left = '120px';
      panelBefore.style.top = '48px';
      const closeBtn = panelBefore.querySelector<HTMLButtonElement>('.floating-panel-close')!;
      closeBtn.focus();
      const leftBefore = panelBefore.offsetLeft;
      const topBefore = panelBefore.offsetTop;
      root.render(createElement(ActiveResearchWidget, { widget: makeWidget(150), runtime, active: true, mapOptions: { fog: true, country: null }, headerSlot: null, onDiagnostics() {} }));
      await hour150Started;
      await settle();
      const pendingShowsLoading = !!panelBefore.querySelector('.dossier > [data-state="loading"]');
      const pendingShowsGoneVerdict = !!panelBefore.querySelector('.dossier > .dossier-missing');
      (releaseHour150 as (() => void) | null)?.();
      await waitFor(() => host.querySelector('[data-state="empty"]'), host);
      await settle();
      const panelAfter = document.querySelector<HTMLElement>('.research-window');
      const openAfterEmpty = !!panelAfter;
      const samePanelNode = panelAfter === panelBefore;
      const positionBefore = { left: leftBefore, top: topBefore };
      const positionAfter = panelAfter ? { left: panelAfter.offsetLeft, top: panelAfter.offsetTop } : null;
      const focusKept = document.activeElement === closeBtn;
      root.render(createElement(ActiveResearchWidget, { widget: makeWidget(160), runtime, active: true, mapOptions: { fog: true, country: null }, headerSlot: null, onDiagnostics() {} }));
      await waitFor(() => panelBefore.querySelector('.dossier > [data-state="error"]'), host);
      await settle();
      const failedShowsError = !!panelBefore.querySelector('.dossier > [data-state="error"]');
      const failedShowsGoneVerdict = !!panelBefore.querySelector('.dossier > .dossier-missing');
      return { openBeforeEmpty, openAfterEmpty, samePanelNode, positionBefore, positionAfter, focusKept, pendingShowsLoading, pendingShowsGoneVerdict, failedShowsError, failedShowsGoneVerdict };
    } finally {
      (releaseHour150 as (() => void) | null)?.();
      root.unmount();
    }
  },

  async testDashboardRestartRemount() {
    nextRemountToken = 0;
    remountProbeState.token = 0;
    remountProbeState.retry = null;
    document.body.innerHTML = '';
    const host = document.createElement('div');
    host.className = 'dash-grid-scroll';
    host.style.width = '1280px';
    host.style.height = '800px';
    document.body.appendChild(host);
    const clock = makeClock();
    let openCalls = 0;
    let notifySecondOpen: (() => void) | null = null;
    const secondOpen = new Promise<void>(resolve => { notifySecondOpen = resolve; });
    let releaseAfterRestart: (() => void) | null = null;
    const afterRestartGate = new Promise<void>(resolve => { releaseAfterRestart = resolve; });
    const mockClient = {
      available: true,
      setMapViews: async () => {},
      onMapStates: async () => () => {},
      openFollow: (ports: readonly MessagePort[]) => {
        openCalls++;
        const seq = openCalls;
        const owned = structuredClone([...ports], { transfer: [...ports] });
        if (seq === 2) notifySecondOpen?.();
        return {
          ready: Promise.resolve(`page-remount:${seq}`),
          close: () => { for (const p of owned) p.close(); },
        };
      },
      product: async (req: { sourceId: string; revision: string; product: string; params: Record<string, unknown> }) => {
        const reply = (data: unknown) => ({ sourceId: req.sourceId, revision: req.revision, product: req.product, data });
        if (req.product === 'scoreboard-columns') {
          return reply({
            branchId: 'b1',
            columns: [{
              key: 'factories', label: 'Factories', unit: '', decimals: 0, available: true, reason: null,
              family: { kind: 'country-factories', measure: 'total' }, partial: null, noneAtZero: false,
            }],
            defaultColumns: ['factories'],
            defaultSort: 'factories',
            warnings: [],
          });
        }
        if (req.product === 'country-groups') {
          return reply({
            branchId: 'b1',
            warnings: [],
            humans: [],
            factions: [{
              id: 'axis', firstHour: 43800000, lastHour: 43800200,
              name: 'Axis', nameReason: null,
              names: [{ firstHour: 43800000, lastHour: 43800200, name: 'Axis', reason: null }],
              recordedName: 'Axis',
              members: [{ tag: 'GER', firstHour: 43800000, lastHour: 43800200 }],
              leaders: [{ tag: 'GER', firstHour: 43800000, lastHour: 43800200 }],
              incomplete: [],
            }],
            wars: [],
            warStarts: [],
            capitulations: [],
            identityChanges: [],
            countryIdentities: [],
            unobserved: { humans: [], factions: [], wars: [] },
          });
        }
        if (req.product === 'follow') {
          const query = req.params.query as { panel?: string } | undefined;
          if (openCalls >= 2) {
            await afterRestartGate;
            return reply({ available: true, route: 1 });
          }
          if (query?.panel === 'research-slots') {
            throw new ShownError('Follow stream interrupted.', 'remount 503');
          }
          return reply({ available: true, route: 1 });
        }
        return reply({ available: true, warnings: [], factions: [], countries: [], labels: [] });
      },
    };
    const runtime = makeRuntime(clock, tracks, mockClient);
    const template: DashboardTemplate = {
      version: 1, name: 'Test', alignment: 'calendar',
      roles: [{ id: 'primary', label: 'Primary', selectedTag: null }],
      tracks: [{ id: 't1', roleId: 'primary', label: 'Primary', branchIndex: 0, anchorOffset: null }],
      widgets: [
        { id: 'rm1', kind: 'remount-probe-test', configVersion: 1, title: 'Remount probe', campaign: 'default', trackIds: ['t1'], time: { mode: 'follow' }, countries: { kind: 'all' }, options: {} },
        { id: 'sb1', kind: 'scoreboard-restart-test', configVersion: 1, title: 'Scoreboard', campaign: 'default', trackIds: ['t1'], time: { mode: 'follow' }, countries: { kind: 'tags', tags: ['GER'] }, options: {} },
        { id: 'ft1', kind: 'faction-restart-test', configVersion: 1, title: 'Factions', campaign: 'default', trackIds: ['t1'], time: { mode: 'follow' }, countries: { kind: 'factions', count: 5, membership: 'end' }, options: { view: 'race' } },
      ],
      layouts: {
        lg: [
          { i: 'rm1', x: 0, y: 0, w: 8, h: 6 },
          { i: 'sb1', x: 8, y: 0, w: 8, h: 6 },
          { i: 'ft1', x: 16, y: 0, w: 8, h: 6 },
        ],
      },
    };
    const root = createRoot(host);
    try {
      root.render(createElement(DashboardGrid, { template, runtime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
      await waitFor(() => host.querySelector('[data-widget="rm1"] [data-state="error"]'), host);
      await settle();
      const tokenBefore = remountProbeState.token;
      const articleBefore = host.querySelector('[data-widget="rm1"]');
      (remountProbeState.retry as (() => void) | null)?.();
      await secondOpen;
      await settle();
      const tokenAfter = remountProbeState.token;
      const articleSame = host.querySelector('[data-widget="rm1"]') === articleBefore;
      const sbArticle = host.querySelector('[data-widget="sb1"]')!;
      const ftArticle = host.querySelector('[data-widget="ft1"]')!;
      const scoreboardLoading = !!sbArticle.querySelector('[data-state="loading"]');
      const scoreboardHasTable = !!sbArticle.querySelector('.table-frame, canvas');
      const factionLoading = !!ftArticle.querySelector('[data-state="loading"]');
      const factionText = ftArticle.textContent ?? '';
      return { tokenBefore, tokenAfter, articleSame, scoreboardLoading, scoreboardHasTable, factionLoading, factionHasEmptyCopy: /No faction totals known/i.test(factionText) };
    } finally {
      (releaseAfterRestart as (() => void) | null)?.();
      root.unmount();
    }
  },

  async testHealthyFollowNotRestarted() {
    siblingRetryState.retry = null;
    document.body.innerHTML = '';
    const host = document.createElement('div');
    host.className = 'dash-grid-scroll';
    host.style.width = '900px';
    host.style.height = '400px';
    document.body.appendChild(host);
    const clock = makeClock();
    let openCalls = 0;
    let catalogueCalls = 0;
    let notifySecondCatalogue: (() => void) | null = null;
    const secondCatalogue = new Promise<void>(resolve => { notifySecondCatalogue = resolve; });
    const mockClient = {
      available: true,
      setMapViews: async () => {},
      onMapStates: async () => () => {},
      openFollow: (ports: readonly MessagePort[]) => {
        openCalls++;
        const owned = structuredClone([...ports], { transfer: [...ports] });
        return {
          ready: Promise.resolve(`page-r8:${openCalls}`),
          close: () => { for (const p of owned) p.close(); },
        };
      },
      product: async (req: { sourceId: string; revision: string; product: string }) => {
        const reply = (data: unknown) => ({ sourceId: req.sourceId, revision: req.revision, product: req.product, data });
        if (req.product === 'equipment-stockpile-catalogue') {
          catalogueCalls++;
          if (catalogueCalls === 1) throw new ShownError('Catalogue read failed.', 'cat 503');
          notifySecondCatalogue?.();
          return reply({ ok: true });
        }
        if (req.product === 'follow') return reply({ available: true, route: 1 });
        return reply({ available: true });
      },
    };
    const runtime = makeRuntime(clock, tracks, mockClient);
    const template: DashboardTemplate = {
      version: 1, name: 'Test', alignment: 'calendar',
      roles: [{ id: 'primary', label: 'Primary', selectedTag: null }],
      tracks: [{ id: 't1', roleId: 'primary', label: 'Primary', branchIndex: 0, anchorOffset: null }],
      widgets: [{ id: 'sr1', kind: 'sibling-retry-test', configVersion: 1, title: 'Sibling retry', campaign: 'default', trackIds: ['t1'], time: { mode: 'follow' }, countries: { kind: 'all' }, options: {} }],
      layouts: { lg: [{ i: 'sr1', x: 0, y: 0, w: 8, h: 6 }] },
    };
    const root = createRoot(host);
    try {
      root.render(createElement(DashboardGrid, { template, runtime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
      await waitFor(() => host.querySelector('[data-state="error"]'), host);
      await settle();
      (siblingRetryState.retry as (() => void) | null)?.();
      await secondCatalogue;
      await settle();
      return { openCalls, catalogueCalls };
    } finally {
      root.unmount();
    }
  },
};
