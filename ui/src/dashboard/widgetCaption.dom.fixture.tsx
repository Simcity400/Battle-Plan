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

declare global { interface Window { captionLayout: unknown } }

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
function DynamicSwitchBody({ widget }: WidgetProps) {
  const version = String(widget.options?.version ?? 'v1');
  useWidgetViewSwitch({
    label: 'Mode',
    value: 'a',
    options: [{ value: 'a', label: 'Alpha' }, { value: 'b', label: 'Beta' }],
    onChange: next => { switchClicks.push(`${version}:${next}`); },
  });
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
        : <div className="frame-probe-body" data-held={held}>Picture</div>}
  </>;
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
  render: FrameProbeBody,
}, {
  kind: 'frame-probe-test',
  version: 1,
  title: 'Frame probe',
  icon: 'grid',
  description: 'Reading badge and announcer test.',
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
  render: FrameProbeBody,
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

const client = { available: true, setMapViews: async () => {}, onMapStates: async () => () => {} };

const makeRuntime = (clock: PlaybackController, loadedTracks: BoundTrack[] = tracks) => ({
  tracks: loadedTracks,
  campaignTracks: loadedTracks,
  defaultRoleId: 'primary',
  alignment: 'calendar',
  campaigns: [],
  recordings: [],
  maps: new MapViewsStore(client as never, clock),
  client,
  clock,
} as unknown as WidgetRuntime);

const settle = () => new Promise<void>(resolve => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
const delay = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms));
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
    await waitFor(() => previewHost.querySelector('[data-view-switch]'), previewHost);
    const previewHasDynamicSwitch = !!previewHost.querySelector('[data-view-switch]');
    previewRoot.unmount();
    root.unmount();
    return { switchClicks: [...switchClicks], previewHasDynamicSwitch };
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

    host.style.width = '900px';
    root.render(createElement(DashboardGrid, { template: makeTemplate(12, { pill: '1 completed' }), runtime: loadedRuntime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
    await waitFor(() => header.querySelector('.dash-widget-summary .pill'), host);
    await settle();
    const stageBeforePlay = Number(header.dataset.headerStage);
    const changesDuringPlay: string[] = [];
    const playMo = new MutationObserver(() => changesDuringPlay.push(header.dataset.headerStage ?? ''));
    playMo.observe(header, { attributes: true, attributeFilter: ['data-header-stage'] });
    clock.setPlaying(true);
    host.style.width = '420px';
    await settle();
    await settle();
    const stageWhilePlaying = Number(header.dataset.headerStage);
    const changesWhilePlaying = changesDuringPlay.length;
    clock.setPlaying(false);
    await settle();
    await settle();
    const stageAfterPause = Number(header.dataset.headerStage);
    playMo.disconnect();
    root.unmount();
    return { unloadedOscillations, triggerWidth, loadedStage, subjectCompact, stageBeforePlay, stageWhilePlaying, changesWhilePlaying, stageAfterPause };
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
    const makeTemplate = (options: Record<string, string>, title = 'Frame probe'): DashboardTemplate => ({
      version: 1, name: 'Test', alignment: 'calendar',
      roles: [{ id: 'primary', label: 'Primary', selectedTag: null }],
      tracks: [{ id: 't1', roleId: 'primary', label: 'Primary', branchIndex: 0, anchorOffset: null }],
      widgets: [{
        id: 'fp1', kind: 'frame-probe-test', configVersion: 1, title, campaign: 'default',
        trackIds: ['t1'], time: { mode: 'follow' }, countries: { kind: 'all' }, options,
      }],
      layouts: { lg: [{ i: 'fp1', x: 0, y: 0, w: 6, h: 8 }] },
    });
    const root = createRoot(host);
    root.render(createElement(DashboardGrid, { template: makeTemplate({ mode: 'loading' }), runtime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
    await waitFor(() => host.querySelector('[data-state="loading"]'), host);
    await settle();
    const article = host.querySelector<HTMLElement>('article.dash-widget')!;
    const loadingRole = article.querySelector('[data-state="loading"]')?.getAttribute('role') ?? null;
    const loadingAnnouncement = article.querySelector('.sr-only[aria-live="polite"]')?.textContent ?? '';

    root.render(createElement(DashboardGrid, { template: makeTemplate({ mode: 'picture', held: 'stale' }), runtime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
    await waitFor(() => article.querySelector('[data-held="stale"]'), host);
    await delay(220);
    root.render(createElement(DashboardGrid, { template: makeTemplate({ mode: 'picture', held: 'stale', tick: '1' }), runtime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
    await delay(240);
    const badgeAfterRerender = article.querySelector('.dash-reading-badge')?.textContent ?? null;
    const badgeRole = article.querySelector('.dash-reading-badge')?.getAttribute('role') ?? null;
    const badgeAnnouncement = article.querySelector('.sr-only[aria-live="polite"]')?.textContent ?? '';

    clock.setPlaying(true);
    root.render(createElement(DashboardGrid, { template: makeTemplate({ mode: 'picture', held: 'behind' }), runtime, editing: false, onApply() {}, onRemove() {}, onLayouts() {} }));
    await settle();
    await delay(460);
    const behindBadge = article.querySelector('.dash-reading-badge')?.textContent ?? null;
    clock.setPlaying(false);

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

    root.unmount();
    return { loadingRole, loadingAnnouncement, badgeAfterRerender, badgeRole, badgeAnnouncement, behindBadge, firstEmptyAnnounced, clearedOnPicture, repeatedEmptyAnnounced };
  },
};
