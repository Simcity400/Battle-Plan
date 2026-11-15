/** The shared time charts following a playing head in a real browser, sized as the dashboard's heaviest pages draw them:
 * what each one's painting costs a frame (timed as the measured run times it, `paint` of the frame record), and what a
 * frame of playback asks of the page besides drawing: layout read back, text measured, colours parsed. */
import { Profiler } from 'react';
import { createRoot } from 'react-dom/client';
import type { Root } from 'react-dom/client';
import TimeChart from './TimeChart';
import type { ChartLayer } from './TimeChart';
import { tiledStackLayer } from './tiledStack';
import type { ChartSummary } from './chartSummary';
import Timeline from './timeline/Timeline';
import type { TimelineLane } from './timeline/layout';
import type { EventBadge } from './badges';
import { LiveCell, liveKey, valuesOf } from './live';
import type { LiveValues } from './live';
import Table from './table/Table';
import type { TableColumn } from './table/Table';
import type { SortValue } from './table/logic';
import LogList from './log/LogList';
import { VirtualList } from '../components/VirtualList';
import { useRef } from 'react';
import type { LogRow } from './log/logData';
import type { RevealClock } from './reveal';
import { measureWith } from '../shell/perf.ts';
import { PlaybackController } from '../dashboard/playback';
import { WidgetFrames } from '../dashboard/widgetClock';
import type { PerfRecorder } from '../shell/perf.ts';
import { canvasElement } from './surface';
import '../styles.css';

const native = window.requestAnimationFrame.bind(window);
const frame = () => new Promise<number>(resolve => native(now => resolve(now)));

// Each chart's painting by frame, as the measured run records it; every other measurement is dropped.
const painted = new Map<number, Map<string, number>>();
measureWith(new Proxy({ paint: (widget: string, ms: number, at: number) => {
  let widgets = painted.get(at);
  if (!widgets) painted.set(at, widgets = new Map());
  widgets.set(widget, (widgets.get(widget) ?? 0) + ms);
} }, { get: (target, name) => (target as Record<string | symbol, unknown>)[name] ?? (() => {}) }) as unknown as PerfRecorder);

// What a frame asks of the page: layout read back, text measured, colours parsed into a context.
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
// Charts draw into their canvas's offscreen canvas, so both kinds of context are counted.
type Context = CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;
const contexts = [CanvasRenderingContext2D.prototype, OffscreenCanvasRenderingContext2D.prototype] as Context[];
for (const proto of contexts) {
  const measureText = proto.measureText;
  proto.measureText = function (this: Context, text: string) { if (counting) asked.measure++; return measureText.call(this, text); };
}

const HOURS = 114_000;
const COLOURS = ['#c0392b', '#2980b9', '#27ae60', '#8e44ad', '#d35400', '#16a085', '#7f8c8d', '#f39c12'];
const head = new LiveCell(20_000);

/** A country measure's lines as tiles draw them: a vertex per bin in view, eight lines, each to the head. */
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
/** A stack of `bands` lines as tiles give them (World tension's countries, Ruling party's parties): `columns` bins over the
 * campaign, each band its own wave. */
function stackSummaries(bands: number, columns: number): ChartSummary[] {
  const width = HOURS / columns;
  return Array.from({ length: bands }, (_, band) => {
    const value = (at: number) => 10 + 8 * Math.sin(at / (17 + band * 5) + band);
    return { from: 0, to: HOURS - 1, width: columns, buckets: Array.from({ length: columns }, (_, column) => {
      const first = Math.floor(column * width), last = Math.floor((column + 1) * width) - 1;
      const [a, b, c] = [value(column), value(column + 0.5), value(column + 1)];
      return { firstHour: first, lastHour: last, firstValue: a, lastValue: c, lastValueHour: last, minHour: first + 1, minValue: Math.min(a, b, c),
        maxHour: last - 1, maxValue: Math.max(a, b, c), firstBreak: false, hasBreak: false };
    }) };
  });
}
// Fills and strokes made on one canvas while counted.
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

/** A diplomacy timeline: factions and their members, each lane a run of coloured bars with names. */
function timelineLanes(count: number): TimelineLane[] {
  return Array.from({ length: count }, (_, lane) => ({
    id: `lane${lane}`, label: `Country ${lane} of a long name`, heading: lane % 6 === 0,
    spans: Array.from({ length: 6 }, (_, span) => ({ id: `s${lane}-${span}`, firstHour: span * 18_000 + lane * 97, lastHour: span * 18_000 + lane * 97 + 15_000,
      label: `Holder ${lane}.${span} with a name`, color: COLOURS[(lane + span) % COLOURS.length], glyph: span === 0 ? '★' : undefined })),
  }));
}

/** An event feed already scrolling, holding rows ahead of where it was drawn, newest first, revealed by the clock. */
const feedRows: LogRow<string>[] = Array.from({ length: 600 }, (_, at) => ({ key: `e${at}`, hour: 18_000 + at * 11, item: `Event ${at} at a place with a name` }))
  .sort((a, b) => b.hour - a.hour);
const feedClock: RevealClock = { hour: () => Math.floor(head.read()), playing: () => true, subscribe: changed => head.subscribe(changed) };

/** The feed's commits while played: a block of rows it had not drawn, which measures its view as it commits. */
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
  /** Three country measures, an event feed and four diplomacy timelines, as Politics, Overview and Diplomacy show them,
   * following the head. */
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
  /** `frames` frames of playback at `hours` a frame, after the charts settle: each chart's painting by frame, and what the
   * played frames asked of the page. */
  async play(frames: number, hours: number, marks?: { from: string; to: string }) {
    // Past the view's first measures, which a resize observer defers to a later frame.
    for (let at = 0; at < 60; at++) await frame();
    if (marks) performance.mark(marks.from);
    painted.clear();
    asked.layout = 0; asked.measure = 0; feedCommits = 0;
    const times: number[] = [];
    counting = true;
    // A painter writing the page's text each frame, as the dashboard's live values do, leaves its style to bring up to date.
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
  /** A stacked chart of `bands` bands over 700 bins following the head, as World tension and Ruling party draw theirs:
   * the fills and strokes it makes a frame, and its painting by frame. */
  async stack(bands: number, frames: number, hours: number) {
    const summaries = stackSummaries(bands, 700);
    const axes = { xFrom: null, xTo: null, yMin: null, yMax: null, log: false };
    const host = place('stack', 700, 260);
    host.render(<TimeChart label="stack" extent={[0, HOURS - 1]} playhead={head.read()} liveHead={head} playback="draw-in" hoursPerSecond={1440}
      axes={axes} branchStart={0} layers={[tiledStackLayer(() => summaries, () => COLOURS.slice(0, bands), () => [0, 20 * bands])]} active revision={0} />);
    for (let at = 0; at < 60; at++) await frame();
    counted = document.querySelector<HTMLCanvasElement>('[data-widget="stack"] canvas');
    drawn.fill = 0; drawn.stroke = 0; painted.clear();
    const times: number[] = [];
    for (let at = 0; at < frames; at++) {
      head.set(head.read() + hours);
      times.push(await frame());
    }
    await frame();
    counted = null;
    const paints = times.slice(0, -1).flatMap(time => painted.get(time)?.get('stack') ?? []);
    return { frames: times.length, fills: drawn.fill, strokes: drawn.stroke, paints };
  },
  /** The event feed alone, revealing a row or two each frame as its clock plays `hours` a frame from halfway through its
   * rows: the transform transitions started on rows, on the list's glide and on anything else, the frames that revealed
   * a row, and the scroll's height against the list's. Rows keep their slide; their entry (fade and flash) is off unless
   * `entries`, so a trace of the window holds what moving the rows costs. */
  async glide(frames: number, hours: number, marks?: { from: string; to: string }, entries = false) {
    const clock = new LiveCell(18_000 + 300 * 11);
    const reveal: RevealClock = { hour: () => Math.floor(clock.read()), playing: () => true, subscribe: changed => clock.subscribe(changed) };
    const still = entries ? null : document.head.appendChild(document.createElement('style'));
    if (still) still.textContent = `[data-widget="glide"] .moving-row:is(.is-entering, [data-reveal='in']) { animation: none; }
      [data-widget="glide"] .moving-row:is(.is-entering, [data-reveal='in'])::after { content: none; }`;
    const root = place('glide', 400, 320);
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
    for (let at = 0; at < 60; at++) await frame();
    const shown = () => host.querySelectorAll('.log-row:not([data-reveal="ahead"])').length;
    const scroller = host.querySelector<HTMLElement>('.log-scroll')!, body = host.querySelector<HTMLElement>('.log-body')!;
    let revealed = 0, before = shown();
    const reveals: { at: number; rows: number }[] = [];
    if (marks) performance.mark(marks.from);
    listening = true;
    for (let at = 0; at < frames; at++) {
      clock.set(clock.read() + hours);
      await frame();
      const now = shown();
      if (now !== before) { revealed++; reveals.push({ at: performance.now(), rows: Math.abs(now - before) }); }
      before = now;
    }
    listening = false;
    if (marks) performance.mark(marks.to);
    // Rows inside their 700 ms entry at the end, and whether the shared entry layer is drawing them.
    const end = performance.now();
    const entering = reveals.filter(reveal => end - reveal.at < 700).reduce((sum, reveal) => sum + reveal.rows, 0);
    const layer = host.querySelectorAll('canvas.log-entries:not([hidden])').length;
    const animated = host.getAnimations({ subtree: true }).filter(animation => animation.playState === 'running').length;
    const scroll = { height: scroller.scrollHeight, list: body.offsetHeight, view: scroller.clientHeight };
    root.unmount();
    host.remove();
    still?.remove();
    return { frames, revealed, started, scroll, entering, layer, animated };
  },
  /** A log committed `count` times with rows newer than it drew, as pages landing at the head commit it: the layout read
   * back while committing and in the frames after, and the commits. */
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
  /** A virtual list (National focus, a ruling party's people, Research) committed `count` times with rows it had not
   * shown, as a structure change while playing commits it: the layout read back while committing and in the frames after. */
  async virtualCommits(count: number) {
    const root = place('virtual', 400, 320);
    function Rows({ at }: { at: number }) {
      const scroll = useRef<HTMLDivElement>(null);
      const rows = feedRows.slice(at * 5, at * 5 + 300);
      return <div ref={scroll} style={{ overflow: 'auto', height: '100%' }}>
        <VirtualList items={rows} itemKey={row => row.key} estimate={28} scrollRef={scroll} label="Virtual"
          style={{ display: 'flex', flexDirection: 'column', rowGap: '2px', padding: '4px' }}>{row => <div style={{ height: 28 }}>{row.item}</div>}</VirtualList>
      </div>;
    }
    root.render(<Rows at={0} />);
    for (let at = 0; at < 30; at++) await frame();
    asked.layout = 0;
    counting = true;
    for (let at = 1; at <= count; at++) {
      root.render(<Rows at={at} />);
      await frame();
      await frame();
    }
    counting = false;
    const result = { layout: asked.layout, shown: document.querySelectorAll('[data-widget="virtual"] [data-virtual-key]').length };
    root.unmount();
    document.querySelector('[data-widget="virtual"]')?.remove();
    return result;
  },
  /** A ranked table of 40 countries whose live scores, and so their bars, move every frame (Game score, Scoreboard):
   * played `frames` frames between the marks. */
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
  /** Two widgets on one playing clock, one off screen until the end: the frames each widget's listener heard, and the
   * painting each was timed for. */
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
