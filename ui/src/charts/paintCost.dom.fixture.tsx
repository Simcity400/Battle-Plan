import { Profiler } from 'react';
import { createRoot } from 'react-dom/client';
import type { Root } from 'react-dom/client';
import TimeChart from './TimeChart';
import type { ChartLayer } from './TimeChart';
import Timeline from './timeline/Timeline';
import type { TimelineLane } from './timeline/layout';
import type { EventBadge } from './badges';
import { LiveCell, liveKey, valuesOf } from './live';
import type { LiveValues } from './live';
import Table from './table/Table';
import type { TableColumn } from './table/Table';
import type { SortValue } from './table/logic';
import LogList from './log/LogList';
import { ENTRY_MS, entryTime } from './log/entryLayer.ts';
import { VirtualList } from '../components/VirtualList';
import { useRef } from 'react';
import type { LogRow } from './log/logData';
import type { RevealClock } from './reveal';
import { measureWith } from '../shell/perf.ts';
import { PlaybackController } from '../dashboard/playback';
import { WidgetFrames } from '../dashboard/widgetClock';
import type { PerfRecorder } from '../shell/perf.ts';
import { canvasElement } from '../test-support/canvasElement';
import '../styles.css';

const native = window.requestAnimationFrame.bind(window);
const frame = () => new Promise<number>(resolve => native(now => resolve(now)));

const painted = new Map<number, Map<string, number>>();
measureWith(new Proxy({ paint: (widget: string, ms: number, at: number) => {
  let widgets = painted.get(at);
  if (!widgets) painted.set(at, widgets = new Map());
  widgets.set(widget, (widgets.get(widget) ?? 0) + ms);
} }, { get: (target, name) => (target as Record<string | symbol, unknown>)[name] ?? (() => {}) }) as unknown as PerfRecorder);

const asked = { layout: 0, measure: 0 };
let counting = false;
for (const [owner, names] of [[Element.prototype, ['clientWidth', 'clientHeight', 'scrollTop', 'scrollHeight']], [HTMLElement.prototype, ['offsetWidth', 'offsetHeight']]] as const) {
  for (const name of names) {
    const property = Object.getOwnPropertyDescriptor(owner, name)!;
    Object.defineProperty(owner, name, { ...property, get(this: Element) { if (counting) asked.layout++; return property.get!.call(this); } });
  }
}
const computed = window.getComputedStyle;
window.getComputedStyle = ((...args: Parameters<typeof computed>) => { if (counting) asked.layout++; return computed.apply(window, args); }) as typeof computed;
const rect = Element.prototype.getBoundingClientRect;
Element.prototype.getBoundingClientRect = function () { if (counting) asked.layout++; return rect.call(this); };
type Context = CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;
const contexts = [CanvasRenderingContext2D.prototype, OffscreenCanvasRenderingContext2D.prototype] as Context[];
for (const proto of contexts) {
  const measureText = proto.measureText;
  proto.measureText = function (this: Context, text: string) { if (counting) asked.measure++; return measureText.call(this, text); };
}

const HOURS = 114_000;
const COLOURS = ['#c0392b', '#2980b9', '#27ae60', '#8e44ad', '#d35400', '#16a085', '#7f8c8d', '#f39c12'];
const head = new LiveCell(20_000);

function lines(count: number): ChartLayer[] {
  const bins = 2_000, binHours = HOURS / bins;
  return Array.from({ length: count }, (_, line) => {
    const values = Float64Array.from({ length: bins }, (_, bin) => 100 + 60 * Math.sin(bin / (30 + line * 7) + line) + bin * 0.05 * (line + 1));
    return {
      draw: (ctx, plot, tokens, state) => {
        const until = Math.min(plot.x[1], state.playhead ?? plot.x[1]);
        const first = Math.max(0, Math.floor(plot.x[0] / binHours)), last = Math.min(bins - 1, Math.floor(until / binHours));
        ctx.save(); ctx.strokeStyle = COLOURS[line % COLOURS.length]; ctx.lineWidth = tokens.lineWidth.series; ctx.lineJoin = 'round';
        ctx.beginPath();
        for (let bin = first; bin <= last; bin++) ctx[bin === first ? 'moveTo' : 'lineTo'](plot.toX(bin * binHours), plot.toY(values[bin]));
        ctx.stroke(); ctx.restore();
      },
      extent: x => {
        const first = Math.max(0, Math.floor(x[0] / binHours)), last = Math.min(bins - 1, Math.floor(Math.min(x[1], head.read()) / binHours));
        let low = Infinity, high = -Infinity;
        for (let bin = first; bin <= last; bin++) { low = Math.min(low, values[bin]); high = Math.max(high, values[bin]); }
        return low <= high ? [low, high] : null;
      },
    };
  });
}
let counted: HTMLCanvasElement | null = null;
const drawn = { fill: 0, stroke: 0 };
for (const proto of contexts) for (const name of ['fill', 'stroke'] as const) {
  const original = proto[name] as (...args: unknown[]) => void;
  (proto as unknown as Record<string, unknown>)[name] = function (this: Context, ...args: unknown[]) {
    if (counted && canvasElement(this.canvas) === counted) drawn[name]++;
    return original.apply(this, args);
  };
}

const badges: EventBadge[] = Array.from({ length: 60 }, (_, at) => ({ hour: 400 + at * 1_800, glyph: at % 2 ? '⚔' : '★', label: `Event ${at}` }));

function timelineLanes(count: number): TimelineLane[] {
  return Array.from({ length: count }, (_, lane) => ({
    id: `lane${lane}`, label: `Country ${lane} of a long name`, heading: lane % 6 === 0,
    spans: Array.from({ length: 6 }, (_, span) => ({ id: `s${lane}-${span}`, firstHour: span * 18_000 + lane * 97, lastHour: span * 18_000 + lane * 97 + 15_000,
      label: `Holder ${lane}.${span} with a name`, color: COLOURS[(lane + span) % COLOURS.length], glyph: span === 0 ? '★' : undefined })),
  }));
}

const feedRows: LogRow<string>[] = Array.from({ length: 600 }, (_, at) => ({ key: `e${at}`, hour: 18_000 + at * 11, item: `Event ${at} at a place with a name` }))
  .sort((a, b) => b.hour - a.hour);
const feedClock: RevealClock = { hour: () => Math.floor(head.read()), playing: () => true, subscribe: changed => head.subscribe(changed) };

let feedCommits = 0;

const roots: Root[] = [];
function place(id: string, width: number, height: number): Root {
  const host = document.body.appendChild(document.createElement('article'));
  host.dataset.widget = id;
  host.style.cssText = `width:${width}px;height:${height}px;display:flex;flex-direction:column;position:relative;overflow:hidden`;
  const root = createRoot(host);
  roots.push(root);
  return root;
}

declare global { interface Window { paintCost: typeof api } }
const api = {
  mount() {
    const axes = { xFrom: null, xTo: null, yMin: null, yMax: null, log: false };
    for (const id of ['measure-a', 'measure-b', 'measure-c']) {
      place(id, 700, 260).render(<TimeChart label={id} extent={[0, HOURS - 1]} playhead={head.read()} liveHead={head} playback="draw-in" hoursPerSecond={1440}
        axes={axes} branchStart={0} layers={lines(8)} badges={badges} active revision={0} />);
    }
    place('feed', 400, 320).render(<Profiler id="feed" onRender={() => { if (counting) feedCommits++; }}>
      <LogList rows={feedRows} label="Events" density="comfortable" months animate playhead={head.read()} reveal={feedClock}
        columns="minmax(0, 1fr)" renderRow={row => <span>{row.item}</span>} describe={row => row.item} /></Profiler>);
    for (const id of ['timeline-a', 'timeline-b', 'timeline-c', 'timeline-d']) {
      place(id, 700, 320).render(<Timeline label={id} extent={[0, HOURS - 1]} playhead={head.read()} liveHead={head} lanes={timelineLanes(24)} showIcons={false} showNames
        hoursPerSecond={1440} active revision={0} />);
    }
  },
  async play(frames: number, hours: number, marks?: { from: string; to: string }) {
    for (let at = 0; at < 60; at++) await frame();
    if (marks) performance.mark(marks.from);
    painted.clear();
    asked.layout = 0; asked.measure = 0; feedCommits = 0;
    const times: number[] = [];
    counting = true;
    const stamp = document.body.appendChild(document.createElement('output'));
    for (let at = 0; at < frames; at++) {
      head.set(head.read() + hours);
      times.push(await frame());
      stamp.textContent = String(at);
    }
    stamp.remove();
    await frame();
    counting = false;
    if (marks) performance.mark(marks.to);
    const charts = new Map<string, number[]>();
    for (const time of times.slice(0, -1)) for (const [widget, ms] of painted.get(time) ?? []) charts.set(widget, [...charts.get(widget) ?? [], ms]);
    return { asked: { ...asked }, frames: times.length - 1, charts: Object.fromEntries(charts), feedCommits };
  },
  async glide(frames: number, hours: number, marks?: { from: string; to: string }, entries = false) {
    const clock = new LiveCell(18_000 + 300 * 11);
    const reveal: RevealClock = { hour: () => Math.floor(clock.read()), playing: () => true, subscribe: changed => clock.subscribe(changed) };
    const still = entries ? null : document.head.appendChild(document.createElement('style'));
    if (still) still.textContent = `[data-widget="glide"] .moving-row:is(.is-entering, [data-reveal='in']) { animation: none; }
      [data-widget="glide"] .moving-row:is(.is-entering, [data-reveal='in'])::after { content: none; }`;
    const root = place('glide', 400, 320);
    let time = 0;
    const realTime = entryTime.now;
    entryTime.now = () => time;
    root.render(<LogList rows={feedRows} label="Glide" density="comfortable" months animate playhead={clock.read()} reveal={reveal}
      columns="minmax(0, 1fr)" renderRow={row => <span>{row.item}</span>} describe={row => row.item} />);
    const host = document.querySelector<HTMLElement>('[data-widget="glide"]')!;
    let listening = false;
    const started = { rows: 0, glide: 0, other: 0 };
    host.addEventListener('transitionrun', event => {
      if (!listening || event.propertyName !== 'transform') return;
      const target = event.target as Element;
      started[target.classList.contains('log-row') ? 'rows' : target.classList.contains('log-glide') ? 'glide' : 'other']++;
    });
    for (let at = 0; at < 60; at++) { time += 1000 / 60; await frame(); }
    const shown = () => host.querySelectorAll('.log-row:not([data-reveal="ahead"])').length;
    const scroller = host.querySelector<HTMLElement>('.log-scroll')!, body = host.querySelector<HTMLElement>('.log-body')!;
    let revealed = 0, before = shown();
    const reveals: { at: number; rows: number }[] = [];
    if (marks) performance.mark(marks.from);
    listening = true;
    for (let at = 0; at < frames; at++) {
      clock.set(clock.read() + hours);
      time += 1000 / 60;
      await frame();
      const now = shown();
      if (now !== before) { revealed++; reveals.push({ at: time, rows: Math.abs(now - before) }); }
      before = now;
    }
    listening = false;
    if (marks) performance.mark(marks.to);
    const entering = reveals.filter(reveal => time - reveal.at < ENTRY_MS).reduce((sum, reveal) => sum + reveal.rows, 0);
    const layer = host.querySelectorAll('canvas.log-entries:not([hidden])').length;
    const animated = host.getAnimations({ subtree: true }).filter(animation => animation.playState === 'running').length;
    const scroll = { height: scroller.scrollHeight, list: body.offsetHeight, view: scroller.clientHeight };
    root.unmount();
    entryTime.now = realTime;
    host.remove();
    still?.remove();
    return { frames, revealed, started, scroll, entering, layer, animated };
  },
  async commits(count: number) {
    const root = place('commits', 400, 320);
    const rowsAt = (at: number) => feedRows.slice(Math.max(0, feedRows.length - 200 - at * 12), feedRows.length - at * 12 + 200);
    const render = (at: number) => root.render(<Profiler id="commits" onRender={() => { if (counting) commits++; }}>
      <LogList rows={rowsAt(at)} label="Commits" density="comfortable" months animate playhead={rowsAt(at)[0]?.hour ?? 0}
        columns="minmax(0, 1fr)" renderRow={row => <span>{row.item}</span>} describe={row => row.item} /></Profiler>);
    let commits = 0;
    render(0);
    for (let at = 0; at < 30; at++) await frame();
    asked.layout = 0;
    counting = true;
    for (let at = 1; at <= count; at++) {
      render(at);
      await frame();
      await frame();
    }
    counting = false;
    const result = { layout: asked.layout, commits };
    root.unmount();
    document.querySelector('[data-widget="commits"]')?.remove();
    return result;
  },
  async virtualCommits(count: number) {
    const root = place('virtual', 400, 320);
    let commits = 0;
    function Rows({ at }: { at: number }) {
      const scroll = useRef<HTMLDivElement>(null);
      const rows = feedRows.slice(at * 5, at * 5 + 300);
      return <div ref={scroll} data-case="virtual-scroll" style={{ overflow: 'auto', height: '100%' }}>
        <Profiler id="virtual" onRender={() => { commits++; }}>
          <VirtualList items={rows} itemKey={row => row.key} estimate={28} scrollRef={scroll} label="Virtual"
            style={{ display: 'flex', flexDirection: 'column', rowGap: '2px', padding: '4px' }}>{row => <div style={{ height: 28 }}>{row.item}</div>}</VirtualList>
        </Profiler>
      </div>;
    }
    root.render(<Rows at={0} />);
    for (let at = 0; at < 30; at++) await frame();
    asked.layout = 0;
    commits = 0;
    counting = true;
    for (let at = 1; at <= count; at++) {
      root.render(<Rows at={at} />);
      await frame();
      await frame();
      counting = false;
      document.querySelector<HTMLElement>('[data-case="virtual-scroll"]')!.scrollTop += 30;
      await frame();
      counting = true;
    }
    counting = false;
    const result = { layout: asked.layout, commits, shown: document.querySelectorAll('[data-widget="virtual"] [data-virtual-key]').length };
    root.unmount();
    document.querySelector('[data-widget="virtual"]')?.remove();
    return result;
  },
  async tableBars(frames: number, marks: { from: string; to: string }) {
    type Row = { key: string; name: string };
    const rows: Row[] = Array.from({ length: 40 }, (_, at) => ({ key: `r${at}`, name: `Country ${String(at).padStart(2, '0')}` }));
    const columns: TableColumn<Row>[] = [
      { key: 'name', header: 'Country', label: 'Country', width: '160px', value: row => row.name },
      { key: 'score', header: 'Score', label: 'Score', width: '160px', numeric: true, bar: 'visible', value: row => Number(row.key.slice(1)), paint: value => `${Math.round(Number(value))}` },
    ];
    const live = new LiveCell<LiveValues<SortValue>>(valuesOf(new Map(), 'Not read yet'));
    const scores = (at: number) => valuesOf(new Map(rows.map((row, index) => [liveKey(row.key, 'score'), 1000 + 400 * Math.sin(at / 9 + index)])), 'not recorded');
    live.set(scores(0));
    place('bars', 420, 600).render(<Table label="Bars" rows={rows} columns={columns} rowKey={row => row.key} sort={{ column: 'name', direction: 'ascending' }}
      hoursPerSecond={1440} live={live} />);
    for (let at = 0; at < 60; at++) await frame();
    performance.mark(marks.from);
    for (let at = 1; at <= frames; at++) { live.set(scores(at)); await frame(); }
    performance.mark(marks.to);
    return { bars: document.querySelectorAll('[data-widget="bars"] .table-cellbar').length };
  },
  async barGlide() {
    type Row = { key: string; name: string };
    const rows: Row[] = Array.from({ length: 6 }, (_, at) => ({ key: `r${at}`, name: `Country ${at}` }));
    const columns: TableColumn<Row>[] = [
      { key: 'name', header: 'Country', label: 'Country', width: '160px', value: row => row.name },
      { key: 'score', header: 'Score', label: 'Score', width: '160px', numeric: true, bar: 1000, value: () => 0, paint: value => `${Math.round(Number(value))}` },
    ];
    const live = new LiveCell<LiveValues<SortValue>>(valuesOf(new Map(rows.map(row => [liveKey(row.key, 'score'), 200])), 'not recorded'));
    place('glide', 420, 400).render(<Table label="Glide" rows={rows} columns={columns} rowKey={row => row.key} sort={{ column: 'name', direction: 'ascending' }}
      hoursPerSecond={1440} live={live} />);
    for (let at = 0; at < 30; at++) await frame();
    const bar = document.querySelector<HTMLElement>('[data-widget="glide"] tr[data-table-row="r0"] .table-cellbar')!;
    const scaleOf = () => Number(/scaleX\(([^)]+)\)/.exec(bar.style.transform)?.[1] ?? NaN);
    const before = scaleOf();
    live.set(valuesOf(new Map(rows.map((row, index) => [liveKey(row.key, 'score'), index === 0 ? 800 : 200])), 'not recorded'));
    const samples: number[] = [];
    for (let at = 0; at < 600 && samples.at(-1) !== 0.8; at++) { await frame(); samples.push(scaleOf()); }
    const style = getComputedStyle(bar), durations = style.transitionDuration.split(',').map(value => parseFloat(value));
    const transition = style.transitionProperty.split(',').map(name => name.trim()).filter((_, at) => (durations[at % durations.length] ?? 0) > 0).join(', ');
    return { before, samples, transition };
  },
  async gate() {
    const page = new PlaybackController();
    page.setBounds({ firstHour: 0, lastHour: 1_000_000 });
    const shown = new WidgetFrames(page, 'shown'), hidden = new WidgetFrames(page, 'hidden');
    hidden.setShown(false);
    const heard = { shown: 0, hidden: 0 };
    const stops = [shown.clock.subscribeFrame(() => heard.shown++), hidden.clock.subscribeFrame(() => heard.hidden++)];
    painted.clear();
    page.setSpeed(1440);
    page.setPlaying(true);
    for (let at = 0; at < 30; at++) await frame();
    const offScreen = { ...heard, painted: [...new Set([...painted.values()].flatMap(widgets => [...widgets.keys()]))] };
    hidden.setShown(true);
    const shownAgain = heard.hidden;
    for (let at = 0; at < 5; at++) await frame();
    page.setPlaying(false);
    for (const stop of stops) stop();
    return { offScreen, shownAgain, after: { ...heard }, sameMethod: shown.clock.getSnapshot === shown.clock.getSnapshot, hour: shown.clock.getHour() === page.getHour() };
  },
};
window.paintCost = api;
