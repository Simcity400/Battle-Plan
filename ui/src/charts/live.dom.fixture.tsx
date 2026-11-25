import { Profiler } from 'react';
import type { ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import type { Root } from 'react-dom/client';
import { LiveCell, liveKey } from './live';
import { valuesOf } from '../test-support/liveValues';
import type { LiveValues } from './live';
import Table from './table/Table';
import type { TableColumn } from './table/Table';
import type { SortValue } from './table/logic';
import Timeline from './timeline/Timeline';
import DrillParts from './drill/DrillParts';
import type { DrillPart, LiveDrillParts } from './drill/DrillParts';
import { canvasElement } from '../test-support/canvasElement';
import '../styles.css';

document.documentElement.dataset.motion = 'reduced';
const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
async function settle() { for (let i = 0; i < 8; i++) await frame(); }

interface Drawn { texts: string[]; fills: string[]; heads: number[] }
const drawn = new WeakMap<HTMLCanvasElement, Drawn>();
type Context = CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;
const pageCanvas = (context: Context) => canvasElement(context.canvas)!;
const log = (context: Context) => {
  const canvas = pageCanvas(context);
  let held = drawn.get(canvas);
  if (!held) drawn.set(canvas, held = { texts: [], fills: [], heads: [] });
  return held;
};
const armed = new WeakSet<Context>();
for (const proto of [CanvasRenderingContext2D.prototype, OffscreenCanvasRenderingContext2D.prototype] as Context[]) {
  const { clearRect, fillText, fillRect, moveTo } = proto;
  proto.clearRect = function (this: Context, x: number, y: number, width: number, height: number) {
    if (x === 0 && y === 0 && width >= pageCanvas(this).clientWidth - 1) drawn.set(pageCanvas(this), { texts: [], fills: [], heads: [] });
    return clearRect.call(this, x, y, width, height);
  };
  proto.fillText = function (this: Context, text: string, ...rest: [number, number, number?]) { log(this).texts.push(String(text)); return fillText.call(this, text, ...rest); };
  proto.fillRect = function (this: Context, ...rest: [number, number, number, number]) { log(this).fills.push(String(this.fillStyle)); return fillRect.apply(this, rest); };
  const lineWidth = Object.getOwnPropertyDescriptor(proto, 'lineWidth')!;
  Object.defineProperty(proto, 'lineWidth', { ...lineWidth, set(this: Context, value: number) {
    if (value === 1.5) armed.add(this); else armed.delete(this);
    lineWidth.set!.call(this, value);
  } });
  proto.moveTo = function (this: Context, x: number, y: number) {
    if (armed.has(this)) { armed.delete(this); log(this).heads.push(x); }
    return moveTo.call(this, x, y);
  };
}

const barWidth = (bar: HTMLElement | null) => {
  const scale = bar && /scaleX\(([^)]+)\)/.exec(bar.style.transform)?.[1];
  return scale == null ? null : `${Number(scale) * 100}%`;
};
const commits: Record<string, number> = {};
const counted = (id: string, node: ReactNode) => <Profiler id={id} onRender={() => { commits[id] = (commits[id] ?? 0) + 1; }}>{node}</Profiler>;

const roots = new Map<string, { host: HTMLDivElement; root: Root }>();
function host(id: string, width: number, height: number) {
  let held = roots.get(id);
  if (!held) {
    const element = document.body.appendChild(document.createElement('div'));
    element.dataset.chart = id;
    held = { host: element, root: createRoot(element) };
    roots.set(id, held);
  }
  held.host.style.cssText = `width:${width}px;height:${height}px;display:flex;flex-direction:column;position:relative`;
  return held;
}
const canvasOf = (id: string) => roots.get(id)!.host.querySelector('canvas')!;
const lastFrame = (id: string): Drawn => drawn.get(canvasOf(id)) ?? { texts: [], fills: [], heads: [] };

interface Row { key: string; name: string }
const tableRows: Row[] = Array.from({ length: 200 }, (_, i) => ({ key: `r${i}`, name: `Country ${String(i).padStart(3, '0')}` }));
const tableColumns: TableColumn<Row>[] = [
  { key: 'name', header: 'Country', label: 'Country', width: '160px', value: row => row.name },
  { key: 'score', header: 'Score', label: 'Score', width: '120px', numeric: true, bar: 'visible', value: row => Number(row.key.slice(1)), paint: value => `${value} pts` },
];
const tableLive = new LiveCell<LiveValues<SortValue>>(valuesOf(new Map(), 'Not read yet'));
const rowKey = (row: Row) => row.key;


const lanes = [{ id: 'slot', label: 'Slot', spans: [{ id: 'holder', firstHour: 0, lastHour: 1000, label: 'Holder' }] }];
const timelineHead = new LiveCell(100);

const drillParts = (a: number | null, b: number | null): DrillPart[] => [
  { id: 'a', label: 'Alpha', value: a, text: a == null ? 'Not known' : `${a} units` },
  { id: 'b', label: 'Beta', value: b, text: b == null ? 'Not known' : `${b} units` },
];
const drillLive = new LiveCell<ReturnType<LiveDrillParts['read']>>({ parts: drillParts(30, 10) });

export const fixture = {
  settle,
  commits: () => ({ ...commits }),

  async mountTable(live: boolean, sort: 'name' | 'score') {
    const { root } = host('table', 420, 400);
    root.render(counted('table', <Table key={`${live}:${sort}`} label="Synthetic table" rows={tableRows} columns={tableColumns} rowKey={rowKey}
      sort={{ column: sort, direction: sort === 'score' ? 'descending' : 'ascending' }} live={live ? tableLive : undefined} />));
    await settle();
  },
  async pushTable(scores: Record<string, number>) {
    tableLive.set(valuesOf(new Map(Object.entries(scores).map(([key, value]) => [liveKey(key, 'score'), value])), key => `no score for ${key.split('\u0000')[0]}`));
    await settle();
  },
  tableRows(count: number) {
    return [...roots.get('table')!.host.querySelectorAll<HTMLElement>('tbody tr:not([aria-hidden="true"])')]
      .sort((a, b) => Number(a.ariaRowIndex) - Number(b.ariaRowIndex)).slice(0, count).map(row => {
        const cell = row.querySelectorAll('td')[1]!, text = cell.querySelector<HTMLElement>('.table-live');
        return { key: row.dataset.tableRow, text: (text ?? cell).textContent, title: text?.title ?? '', bar: barWidth(cell.querySelector<HTMLElement>('.table-cellbar')) };
      });
  },
  tableMounted: () => roots.get('table')!.host.querySelectorAll('tbody tr').length,

  async mountTimeline(live: boolean) {
    const { root } = host('timeline', 600, 200);
    root.render(counted('timeline', <Timeline key={String(live)} label="Synthetic timeline" extent={[0, 1000]} playhead={100} lanes={lanes} showIcons={false} showNames
      active revision={0} liveHead={live ? timelineHead : undefined} />));
    await settle();
  },
  async pushHead(hour: number) { timelineHead.set(hour); await settle(); },
  playheadX: () => lastFrame('timeline').heads[0] ?? null,

  async mountDrill(live: boolean) {
    const { root } = host('drill', 400, 200);
    root.render(counted('drill', <DrillParts key={String(live)} parts={drillParts(30, 10)} format={String} live={live ? drillLive : undefined} />));
    await settle();
  },
  async pushDrill(a: number | null, b: number | null) { drillLive.set({ parts: drillParts(a, b) }); await settle(); },
  drill: () => [...roots.get('drill')!.host.querySelectorAll('.drill-parts > li')].map(row => [row.querySelector('.drill-part-label')?.textContent, row.querySelector('b')?.textContent]),


  picture: (id: string) => canvasOf(id).toDataURL(),
  unmount(id: string) { roots.get(id)?.root.render(<></>); },
};
declare global { interface Window { liveCharts: typeof fixture } }
window.liveCharts = fixture;
