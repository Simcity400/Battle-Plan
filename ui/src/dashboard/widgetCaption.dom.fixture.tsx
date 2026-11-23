import { createElement, useLayoutEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import '../styles.css';
import './dashboard.css';
import '../charts/charts.css';
import '../charts/drill/drill.css';
import '../charts/table/table.css';
import '../widgets/army-losses-by-month/armyLosses.css';
import DashboardGrid from './DashboardGrid.tsx';
import { registerWidgets } from './definitions.ts';
import type { WidgetProps, WidgetRuntime } from './definitions.ts';
import { MapViewsStore } from './geometry.ts';
import type { BoundTrack, DashboardTemplate, WidgetSpec } from './model.ts';
import { PlaybackController } from './playback.ts';
import type { FrameEngine } from '../engine/frame.ts';
import { WidgetCaption } from './widgetFrame.tsx';
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
}]);

const branch = { id: 'b1', sessionId: 'b1', firstHour: 100, lastHour: 200, main: true, fork: null, joins: [], sampleBoundary: '', gaps: [] };
const tracks: BoundTrack[] = [{
  id: 't1', roleId: 'primary', label: 'Primary', branchIndex: 0, anchorOffset: null, axisOffset: 0,
  selectedTag: null, selectedSlot: null,
  source: {
    id: 's1', path: 'synthetic', revision: '1',
    metadata: {
      path: 'synthetic', name: 'Test', format: 145, captureMode: 'full', campaignKey: 'test',
      branches: [branch], countries: [], playerCountryId: null, contentVerified: true, partial: null, warnings: [], buildWarnings: [], revision: '1',
    },
  },
  branch,
} as unknown as BoundTrack];

const clock = new PlaybackController({ every: () => ({ wake() {}, stop() {} }) } as unknown as FrameEngine);
clock.setBounds({ firstHour: 100, lastHour: 200 });
clock.seek(150);

const client = { available: true, setMapViews: async () => {}, onMapStates: async () => () => {} };

const runtime = {
  tracks,
  campaignTracks: tracks,
  defaultRoleId: 'primary',
  alignment: 'calendar',
  campaigns: [],
  recordings: [],
  maps: new MapViewsStore(client as never, clock),
  client,
  clock,
} as unknown as WidgetRuntime;

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
};
