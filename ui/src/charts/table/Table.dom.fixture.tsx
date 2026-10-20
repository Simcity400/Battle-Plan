/** A synthetic table in the production kit and styles, at a chosen width. */
import { createRoot } from 'react-dom/client';
import Table from './Table';
import TooltipLayer from '../TooltipLayer';
import type { TableColumn } from './columns';
import type { NarrowTable } from './options';
import { tooltipStore } from '../tooltip';
import '../../styles.css';

interface Row { key: string; name: string; first: number; second: number; third: number | null }
const rows: Row[] = Array.from({ length: 40 }, (_, i) => ({ key: `r${i}`, name: `Synthetic country ${i}`,
  first: 1000 + i, second: 200000 + i, third: i % 2 ? null : 30000000 + i }));
const columnsFor = (fourth: string, firstPriority?: number): TableColumn<Row>[] => [
  { key: 'name', header: 'Country', label: 'Country', width: 'minmax(var(--table-name-min), 2fr)', priority: firstPriority, value: row => row.name },
  { key: 'first', header: 'First measure', label: 'First measure', numeric: true, width: 'max-content', priority: 1, value: row => row.first },
  { key: 'second', header: 'Second measure', label: 'Second measure', numeric: true, width: 'max-content', priority: 2, value: row => row.second },
  { key: 'third', header: 'Third measure', label: 'Third measure', numeric: true, width: 'max-content', priority: 3, value: row => row.third },
  { key: 'fourth', header: 'Fourth measure', label: 'Fourth measure', width: fourth, priority: 3, inTooltip: true, value: row => row.key },
];
const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
async function settle() { for (let i = 0; i < 8; i++) await frame(); }
const host = document.createElement('div'); document.body.replaceChildren(host);
document.documentElement.dataset.motion = 'reduced';
const root = createRoot(host);
let mounts = 0;
let shown = { narrow: 'hide' as NarrowTable, tooltip: true, sort: 'first', fourth: '120px', firstPriority: undefined as number | undefined, tall: 0, rank: false };
const draw = () => root.render(<><TooltipLayer /><Table key={mounts} label="Synthetic table" rows={rows} columns={columnsFor(shown.fourth, shown.firstPriority)} rowKey={row => row.key} narrow={shown.narrow} rank={shown.rank}
  sort={{ column: shown.sort, direction: 'descending' }} tooltip={shown.tooltip ? row => ({ title: row.name,
    rows: [{ label: 'Fourth measure', value: row.key }, ...(shown.tall ? Array.from({ length: shown.tall }, (_, i) => ({ label: `Fitted row ${i}`, value: String(i) })) : [])] }) : undefined} /></>);
export const fixture = {
  async mount(width: number, narrow: NarrowTable = 'hide', tooltip = true, sort = 'first', firstPriority?: number, rank = false) {
    host.style.cssText = `width:${width}px;height:400px;display:flex;flex-direction:column`;
    mounts++; shown = { narrow, tooltip, sort, fourth: '120px', firstPriority, tall: 0, rank };
    draw(); await settle();
  },
  async setFourth(width: string) { shown = { ...shown, fourth: width }; draw(); await settle(); },
  async setTall(rows = 60) { shown = { ...shown, tall: rows }; draw(); await settle(); },
  settle,
  async setRank(rank: boolean) { shown = { ...shown, rank }; draw(); await settle(); },
  async rowKey(index: number, key: string) {
    host.querySelector(`tr[aria-rowindex="${index + 2}"]`)!.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true }));
    await settle();
  },
  async wheel(deltaY: number) {
    const event = new WheelEvent('wheel', { deltaY, bubbles: true, cancelable: true });
    host.querySelector('tbody tr')!.dispatchEvent(event); await settle();
    return event.defaultPrevented;
  },
  layer() {
    const layer = document.querySelector<HTMLElement>('.chart-tooltip');
    const rows = [...(layer?.querySelectorAll('.chart-tooltip-row') ?? [])].map(row => row.textContent ?? '');
    return { open: !!layer?.matches(':popover-open'), scrollTop: layer?.scrollTop ?? 0, clientHeight: layer?.clientHeight ?? 0,
      scrollHeight: layer?.scrollHeight ?? 0, rows };
  },
  async resize(width: number) { host.style.width = `${width}px`; await settle(); },
  async focusRow(index: number) { host.querySelector<HTMLElement>(`tr[aria-rowindex="${index + 2}"]`)!.focus(); await settle(); },
  snapshot() {
    const scroller = host.querySelector<HTMLElement>('.chart-table')!;
    return { headers: [...host.querySelectorAll('thead th')].map(cell => cell.getAttribute('aria-label')),
      cells: [...host.querySelector('tbody tr')!.querySelectorAll('td')].length,
      clientWidth: scroller.clientWidth, scrollWidth: scroller.scrollWidth,
      description: host.querySelector('.sr-only')?.textContent ?? null,
      // Each shown header's left edge beside the first row's cell under it.
      alignment: [...host.querySelectorAll('thead th')].map((cell, index) => [cell.getBoundingClientRect().left,
        host.querySelectorAll('tbody tr')[0]?.querySelectorAll('td')[index]?.getBoundingClientRect().left ?? null]),
      scrollable: tooltipStore.get()?.content.scrollable ?? false };
  },
};
declare global { interface Window { tableFixture: typeof fixture } }
window.tableFixture = fixture;
