import { createRoot } from 'react-dom/client';
import HeatMatrix from './HeatMatrix';
import type { MatrixAxisItem, MatrixCell } from './matrix';
import { tooltipStore } from '../tooltip';
import '../../styles.css';

const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
async function settle() { for (let i = 0; i < 8; i++) await frame(); }
const host = document.createElement('div'); document.body.replaceChildren(host);
document.documentElement.dataset.motion = 'reduced';
const root = createRoot(host);
let visible: { rows: readonly string[]; columns: readonly string[] } = { rows: [], columns: [] };
let selected: [number, number] | null = null;
let mounts = 0;
const items = (count: number): MatrixAxisItem[] => Array.from({ length: count }, (_, i) => ({ id: `c${i}`, label: `Synthetic country ${i}` }));
const cell = (row: number, column: number): MatrixCell => row === column ? null : (row + column) % 7 === 0 ? { unknown: true } : { color: '#3366aa' };
export const fixture = {
  async mount(width: number, height: number, count: number) {
    host.style.cssText = `width:${width}px;height:${height}px;display:flex;flex-direction:column`;
    visible = { rows: [], columns: [] }; selected = null;
    const axis = items(count);
    root.render(<HeatMatrix key={++mounts} label="Synthetic opinions" rows={axis} columns={axis} cell={cell} revision={count} active
      tooltip={(row, column) => ({ title: `${axis[row].label} of ${axis[column].label}`, rows: [] })}
      onSelect={(row, column) => { selected = [row, column]; }} onVisible={(rows, columns) => { visible = { rows, columns }; }} />);
    await settle();
  },
  async replace(count: number) {
    const axis = items(count).map(item => ({ ...item, id: `other ${item.id}` }));
    root.render(<HeatMatrix key={mounts} label="Synthetic opinions" rows={axis} columns={axis} cell={cell} revision={`other:${count}`} active
      tooltip={() => null} onVisible={(rows, columns) => { visible = { rows, columns }; }} />);
    await settle();
  },
  async key(key: string) {
    host.querySelector('canvas')!.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true }));
    await settle();
  },
  async focus() { host.querySelector('canvas')!.focus(); await settle(); },
  snapshot() {
    const scroller = host.querySelector<HTMLElement>('.heat-matrix-surface')!;
    return { clientWidth: scroller.clientWidth, clientHeight: scroller.clientHeight, scrollWidth: scroller.scrollWidth, scrollHeight: scroller.scrollHeight,
      scrollLeft: scroller.scrollLeft, scrollTop: scroller.scrollTop, hostScrollWidth: host.scrollWidth, hostWidth: host.clientWidth,
      visibleRows: visible.rows.length, visibleColumns: visible.columns.length, firstRow: visible.rows[0] ?? null,
      tooltip: tooltipStore.get()?.content.title ?? null, selected,
      spoken: host.querySelector('[aria-live]')?.textContent ?? '' };
  },
};
declare global { interface Window { matrixFixture: typeof fixture } }
window.matrixFixture = fixture;
