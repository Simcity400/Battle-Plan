import { StrictMode, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import { VirtualList } from './VirtualList';
import { createTabFixture } from './VirtualList.tab.fixture';

const items = Array.from({ length: 100 }, (_, id) => ({ id: String(id) }));
const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
async function settle() { for (let i = 0; i < 8; i++) await frame(); }

type RowControls = 'nested' | 'plain' | 'description';
const rowHeights = new Map<string, number>();
let eligibilityCalls = 0;
function Row({ id, column, controls }: { id: string; column: number; controls: RowControls }) {
  const [open, setOpen] = useState(false);
  return <details onToggle={event => setOpen(event.currentTarget.open)}>
    <summary data-control={`${column}:${id}:row`} style={{ height: rowHeights.get(`${column}:${id}`) ?? 56 }}>Completion {column}:{id}</summary>
    {open && (controls === 'plain' ? <div style={{ height: 40 }}>
      <input aria-label="Conditional input" data-control={`${column}:${id}:input`} />
      <button data-control={`${column}:${id}:button`}>Conditional action</button>
    </div> : <details><summary data-control={`${column}:${id}:details`} style={{ height: 24 }}>Description</summary>
      <div style={{ height: 96 }}>{controls === 'description' ? 'Technology description' : <>
        <input aria-label="Nested input" data-control={`${column}:${id}:input`} />
        <button data-control={`${column}:${id}:button`}>Nested action</button></>}</div>
    </details>)}
  </details>;
}

type HistoryProps = { columns: number; deferChildren: boolean; controls: RowControls; noninteractive: readonly number[];
  rows?: typeof items; eligible?: readonly number[] };
function Histories({ columns, deferChildren, controls, noninteractive, rows = items, eligible }: HistoryProps) {
  const scroll = useRef<HTMLDivElement>(null);
  const interactive = useMemo(() => {
    const positions = new Set(eligible ?? noninteractive);
    return (item: { id: string }) => eligible ? positions.has(Number(item.id)) : !positions.has(Number(item.id));
  }, [eligible, noninteractive]);
  const isInteractive = useMemo(() => (item: { id: string }) => { eligibilityCalls++; return interactive(item); }, [interactive]);
  const [ready, setReady] = useState(!deferChildren);
  useLayoutEffect(() => { setReady(true); }, []);
  return <><button data-control="before">Before histories</button>
    <div ref={scroll} data-scroll style={{ height: 300, overflow: 'auto', width: 700 }}>
      <div style={{ display: 'flex', alignItems: 'flex-start' }}>
        {ready && Array.from({ length: columns }, (_, column) => <section key={column} data-column={column} style={{ flex: 1 }}>
          <VirtualList items={rows} itemKey={item => item.id} estimate={56} label={`Column ${column}`} scrollRef={scroll}
            isInteractive={isInteractive}>
            {item => !interactive(item) ? <div style={{ height: 56 }}>Unfinished {column}:{item.id}</div>
              : <Row id={item.id} column={column} controls={controls} />}
          </VirtualList>
        </section>)}
      </div>
    </div><button data-control="after">After histories</button></>;
}

const observations = { observed: [] as Element[], unobserved: [] as Element[] };
const virtualRow = (target: Element) => target instanceof HTMLLIElement && target.dataset.virtualKey !== undefined;
const observe = ResizeObserver.prototype.observe, unobserve = ResizeObserver.prototype.unobserve;
ResizeObserver.prototype.observe = function (target, options) { if (virtualRow(target)) observations.observed.push(target); observe.call(this, target, options); };
ResizeObserver.prototype.unobserve = function (target) { if (virtualRow(target)) observations.unobserved.push(target); unobserve.call(this, target); };

const host = document.createElement('div');
document.body.replaceChildren(host);
document.body.style.margin = '0';
const root = createRoot(host);
let mount = 0;
let mounted: HistoryProps | null = null, strict = false;
const show = (node: ReactNode) => root.render(strict ? <StrictMode>{node}</StrictMode> : node);
function watchedScroller() {
  const node = document.createElement('div');
  node.style.cssText = 'height:300px;overflow:auto;width:700px';
  const listeners = new Set<EventListenerOrEventListenerObject>();
  const add = node.addEventListener.bind(node), remove = node.removeEventListener.bind(node);
  node.addEventListener = ((type: string, listener: EventListenerOrEventListenerObject, options?: boolean | AddEventListenerOptions) => {
    if (type === 'scroll') listeners.add(listener); add(type, listener, options);
  }) as typeof node.addEventListener;
  node.removeEventListener = ((type: string, listener: EventListenerOrEventListenerObject, options?: boolean | EventListenerOptions) => {
    if (type === 'scroll') listeners.delete(listener); remove(type, listener, options);
  }) as typeof node.removeEventListener;
  return { node, listeners };
}
let rebinding: { move(): void; dispose(): void; listeners(): number[]; oldScroll(top: number): void } | null = null;
let sparse: HistoryProps | null = null;
export const fixture = {
  tab: createTabFixture(node => { rebinding?.dispose(); rebinding = null; root.render(node); }),
  async mount(columns = 1, deferChildren = false, controls: RowControls = 'nested', noninteractive: readonly number[] = [], strictly = false) {
    rebinding?.dispose(); rebinding = null;
    rowHeights.clear();
    mounted = { columns, deferChildren, controls, noninteractive }; strict = strictly;
    show(<Histories key={++mount} {...mounted} />);
    await settle();
  },
  async rerender() { show(<Histories key={mount} {...mounted!} rows={[...items]} />); await settle(); },
  observations(reset = false) {
    const kept = (targets: Element[]) => targets.filter(target => target.isConnected).length;
    const seen = { observed: observations.observed.length, unobserved: observations.unobserved.length,
      keptObserved: kept(observations.observed), keptUnobserved: kept(observations.unobserved) };
    if (reset) { observations.observed.length = 0; observations.unobserved.length = 0; }
    return seen;
  },
  async mountSparse(count: number, eligible: readonly number[], controls: RowControls = 'plain') {
    rebinding?.dispose(); rebinding = null; rowHeights.clear(); eligibilityCalls = 0;
    sparse = { columns: 1, deferChildren: false, controls, noninteractive: [], eligible,
      rows: Array.from({ length: count }, (_, id) => ({ id: String(id) })) };
    root.render(<Histories key={++mount} {...sparse} />); await settle();
  },
  async eligibility(eligible: readonly number[]) {
    sparse = { ...sparse!, eligible }; root.render(<Histories key={mount} {...sparse} />); await settle();
  },
  async reverseSparseRows() {
    sparse = { ...sparse!, rows: [...sparse!.rows!].reverse() }; root.render(<Histories key={mount} {...sparse} />); await settle();
  },
  predicateCalls(reset = false) { const calls = eligibilityCalls; if (reset) eligibilityCalls = 0; return calls; },
  settle,
  async rowGap(gap: number) {
    const list = document.querySelector<HTMLOListElement>('ol')!;
    list.style.display = 'flex'; list.style.flexDirection = 'column'; list.style.rowGap = `${gap}px`;
    await settle();
  },
  async fractionalRow(height: number, zoom = 1) {
    const scroller = document.querySelector<HTMLElement>('[data-scroll]')!;
    scroller.style.height = '300.5px'; scroller.style.zoom = String(zoom);
    const row = document.querySelector<HTMLLIElement>('[data-virtual-index="0"]')!;
    rowHeights.set('0:0', height);
    row.querySelector<HTMLElement>('summary')!.style.height = `${height}px`;
    const measured = await new Promise<number>(done => {
      const observer = new ResizeObserver(entries => {
        observer.disconnect(); done(entries[0].borderBoxSize[0].blockSize);
      });
      observer.observe(row);
    });
    await settle(); return measured;
  },
  async scroll(top: number) { document.querySelector<HTMLElement>('[data-scroll]')!.scrollTop = top; await settle(); },
  async focus(control: string) { document.querySelector<HTMLElement>(`[data-control="${control}"]`)!.focus(); await settle(); },
  async expand(column: number, id: number) {
    const outer = document.querySelector<HTMLDetailsElement>(`[data-column="${column}"] [data-virtual-index="${id}"] details`)!;
    outer.open = true; await settle(); outer.querySelector('details')!.open = true; await settle();
  },
  async expandOuter(column: number, id: number) {
    document.querySelector<HTMLDetailsElement>(`[data-column="${column}"] [data-virtual-index="${id}"] details`)!.open = true;
    await settle();
  },
  async mountRebinding() {
    root.render(null); await settle(); rebinding?.dispose();
    const area = document.createElement('div'), first = watchedScroller(), second = watchedScroller();
    first.node.dataset.scroll = '';
    area.append(first.node, second.node); document.body.append(area);
    const container = document.createElement('div'); first.node.append(container);
    const reference = { current: first.node as HTMLElement | null }, listRoot = createRoot(container);
    function RebindingList({ version }: { version: number }) {
      return <div data-version={version}><VirtualList items={items} itemKey={item => item.id} estimate={56} label="Rebound history" scrollRef={reference}>
        {item => <summary style={{ height: 56 }}>Completion {item.id}</summary>}
      </VirtualList></div>;
    }
    listRoot.render(<RebindingList version={1} />);
    rebinding = {
      move() {
        second.node.append(container); delete first.node.dataset.scroll; second.node.dataset.scroll = '';
        const spacer = document.createElement('div'); spacer.style.height = '6000px'; first.node.append(spacer);
        reference.current = second.node; listRoot.render(<RebindingList version={2} />);
      },
      dispose() { listRoot.unmount(); area.remove(); },
      listeners: () => [first.listeners.size, second.listeners.size],
      oldScroll(top) { first.node.scrollTop = top; },
    };
    await settle();
  },
  async rebind() { rebinding!.move(); await settle(); },
  async oldScroll(top: number) { rebinding!.oldScroll(top); await settle(); },
  bindingListeners() { return rebinding!.listeners(); },
  unmountRebinding() { rebinding!.dispose(); return rebinding!.listeners(); },
  snapshot() {
    const lists = [...document.querySelectorAll<HTMLOListElement>('ol')];
    return {
      active: document.activeElement?.getAttribute('data-control'),
      scrollTop: document.querySelector<HTMLElement>('[data-scroll]')!.scrollTop,
      columns: lists.map(list => ({
        totalHeight: parseFloat(list.style.height),
        rows: [...list.querySelectorAll<HTMLLIElement>('[data-virtual-index]')].map(row => ({
          index: Number(row.dataset.virtualIndex), top: parseFloat(row.style.top), height: row.offsetHeight,
          position: Number(row.getAttribute('aria-posinset')), size: Number(row.getAttribute('aria-setsize')),
          opened: [...row.querySelectorAll('details')].map(details => details.open),
        })),
      })),
    };
  },
};

declare global { interface Window { virtualListFixture: typeof fixture } }
window.virtualListFixture = fixture;
