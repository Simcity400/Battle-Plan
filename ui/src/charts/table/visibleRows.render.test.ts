import assert from 'node:assert/strict';
import test from 'node:test';
import { act, createElement, createRef, useCallback, useLayoutEffect, useState } from 'react';
import type { CSSProperties, ReactNode } from 'react';
import type { Root } from 'react-dom/client';
import { APPEARANCE_EVENT } from '../../shell/appearance/events.ts';
import { createRoot } from 'react-dom/client';
import { createServer } from 'vite';
import { tokensFromValues } from '../tokens.ts';
import type { TableColumn } from './columns';

/** Just enough of a document for react-dom's client renderer and the table's layout reads. */
class FakeNode {
  childNodes: FakeNode[] = [];
  parentNode: FakeNode | null = null;
  attributes = new Map<string, string>();
  style: Record<string, unknown> & { setProperty(name: string, value: string): void; removeProperty(name: string): void };
  dataset: Record<string, string> = {};
  nodeValue: string | null = null;
  scrollTop = 0; width = 800; height = 600; requiredWidth: number | (() => number) = 800;
  get scrollHeight() { reads.layout++; return 600; }
  get clientHeight() { reads.layout++; return this.height; }
  get clientWidth() { reads.layout++; return this.width; }
  get scrollWidth() { reads.layout++; return typeof this.requiredWidth === 'function' ? this.requiredWidth() : this.requiredWidth; }
  get offsetHeight() { reads.layout++; return 32; }
  get offsetWidth() { reads.layout++; return this.width; }
  get offsetTop() { reads.layout++; return 0; }
  get offsetLeft() { reads.layout++; return 0; }
  getBoundingClientRect() { reads.layout++; return { x: 0, y: 0, top: 0, left: 0, width: this.width, height: this.height }; }
  readonly nodeType: number; readonly nodeName: string; ownerDocument: FakeNode | null;
  constructor(nodeType: number, nodeName: string, ownerDocument: FakeNode | null = null) {
    this.nodeType = nodeType; this.nodeName = nodeName; this.ownerDocument = ownerDocument;
    const style: Record<string, unknown> = {};
    this.style = Object.assign(style, { setProperty: (name: string, value: string) => { style[name] = value; },
      removeProperty: (name: string) => { delete style[name]; } });
  }
  get namespaceURI() { return 'http://www.w3.org/1999/xhtml'; }
  get tagName() { return this.nodeName; }
  get firstChild(): FakeNode | null { return this.childNodes[0] ?? null; }
  get lastChild(): FakeNode | null { return this.childNodes.at(-1) ?? null; }
  get nextSibling(): FakeNode | null { const siblings = this.parentNode?.childNodes ?? []; return siblings[siblings.indexOf(this) + 1] ?? null; }
  get isConnected(): boolean { return this.nodeType === 9 || (this.parentNode?.isConnected ?? false); }
  get textContent(): string { return this.nodeType === 3 ? this.nodeValue ?? '' : this.childNodes.map(child => child.textContent).join(''); }
  set textContent(text: string) {
    for (const child of this.childNodes) child.parentNode = null;
    this.childNodes = [];
    if (text) this.appendChild(Object.assign(new FakeNode(3, '#text', this.ownerDocument), { nodeValue: text }));
  }
  appendChild(child: FakeNode) { return this.insertBefore(child, null); }
  insertBefore(child: FakeNode, before: FakeNode | null) {
    child.parentNode?.removeChild(child);
    const index = before ? this.childNodes.indexOf(before) : -1;
    this.childNodes.splice(index < 0 ? this.childNodes.length : index, 0, child);
    child.parentNode = this;
    return child;
  }
  removeChild(child: FakeNode) { this.childNodes.splice(this.childNodes.indexOf(child), 1); child.parentNode = null; return child; }
  setAttribute(name: string, value: string) {
    this.attributes.set(name, String(value));
    if (name.startsWith('data-')) this.dataset[name.slice(5).replace(/-([a-z])/g, (_, letter: string) => letter.toUpperCase())] = String(value);
  }
  removeAttribute(name: string) { this.attributes.delete(name); }
  getAttribute(name: string) { return this.attributes.get(name) ?? null; }
  hasAttribute(name: string) { return this.attributes.has(name); }
  addEventListener() {}
  removeEventListener() {}
  matches(selector: string): boolean {
    return selector === '[data-table-natural]' ? this.hasAttribute('data-table-natural')
      : selector === 'tbody tr' ? this.nodeName === 'TR' && this.parentNode?.nodeName === 'TBODY'
      : this.nodeName.toLowerCase() === selector;
  }
  querySelectorAll(selector: string): FakeNode[] {
    return this.childNodes.flatMap(child => [...(child.matches(selector) ? [child] : []), ...child.querySelectorAll(selector)]);
  }
  closest(selector: string): FakeNode | null { return selector.split(', ').some(part => this.matches(part)) ? this : this.parentNode?.closest(selector) ?? null; }
  contains(node: FakeNode | null): boolean { return node === this || this.childNodes.some(child => child.contains(node)); }
  getRootNode() { return this.ownerDocument ?? this; }
  getContext() { return { font: '', measureText: () => ({ width: 8 }) }; }
  focus() {}
}

const CSS: Record<string, string> = { '--table-comfortable-height': '32', '--table-compact-height': '26', '--table-header-min-height': '24',
  '--type-sm': '12', width: '40px', paddingLeft: '0', paddingRight: '0', font: '12px sans-serif', letterSpacing: '0' };

const reads = { layout: 0, styles: 0, properties: 0, typography: 0, metrics: 0, state: 0, setFit: 0, setView: 0, setNatural: 0, setRowMeasurements: 0, setDescription: 0, observed: [] as string[] };
function resetReads() { for (const key of Object.keys(reads) as (keyof typeof reads)[]) { if (key === 'observed') reads.observed = []; else reads[key] = 0; } }

async function withTable(run: (fixture: { Table: typeof import('./Table.tsx').default; CellBar: typeof import('./Table.tsx').CellBar;
  ProgressMeter: typeof import('../meter/ProgressMeter.tsx').default; WindowedRows: typeof import('../WindowedRows.tsx').default;
  loadModule: <T>(path: string) => Promise<T>;
  root: Root; container: FakeNode; css: Record<string, string>; events: EventTarget; settle: () => Promise<void>; resize: (element: FakeNode) => void }) => Promise<void>) {
  resetReads();
  const document = new FakeNode(9, '#document');
  const html = new FakeNode(1, 'HTML', document), body = new FakeNode(1, 'BODY', document);
  document.appendChild(html);
  html.appendChild(body);
  Object.assign(document, { documentElement: html, body, activeElement: body,
    createElement: (name: string) => new FakeNode(1, name.toUpperCase(), document),
    createElementNS: (_namespace: string, name: string) => new FakeNode(1, name, document),
    createTextNode: (text: string) => Object.assign(new FakeNode(3, '#text', document), { nodeValue: text }) });
  const replaced = ['document', 'window', 'getComputedStyle', 'ResizeObserver', 'cancelAnimationFrame', 'requestAnimationFrame', '__tableReads', 'HTMLIFrameElement', 'IS_REACT_ACT_ENVIRONMENT'] as const;
  const host = globalThis as unknown as Record<string, unknown>;
  const before = new Map(replaced.map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
  const define = (key: string, value: unknown) => Object.defineProperty(globalThis, key, { configurable: true, writable: true, value });
  define('document', document);
  const events = new EventTarget();
  define('window', Object.assign(Object.create(globalThis), { addEventListener: events.addEventListener.bind(events), removeEventListener: events.removeEventListener.bind(events),
    setTimeout, clearTimeout }));
  define('__tableReads', reads);
  const css = { ...CSS };
  define('getComputedStyle', (element: FakeNode) => {
    reads.styles++;
    if (element === html) reads.typography++;
    if (element.getAttribute('class')?.split(' ').includes('chart-table')) reads.metrics++;
    return { ...css, getPropertyValue: (name: string) => { reads.properties++; return css[name] ?? ''; } };
  });
  const frames = new Map<number, FrameRequestCallback>();
  let frame = 0;
  define('requestAnimationFrame', (callback: FrameRequestCallback) => { frames.set(++frame, callback); return frame; });
  define('cancelAnimationFrame', (id: number) => frames.delete(id));
  const observers = new Set<Observer>();
  class Observer {
    targets = new Set<FakeNode>(); pending = new Set<FakeNode>();
    constructor(readonly callback: (entries: unknown[]) => void) { observers.add(this); }
    observe(target: FakeNode) { reads.observed.push(target.dataset.tableNatural ?? 'row/view'); this.targets.add(target); this.pending.add(target); }
    unobserve(target: FakeNode) { this.targets.delete(target); this.pending.delete(target); }
    disconnect() { this.targets.clear(); this.pending.clear(); observers.delete(this); }
  }
  define('ResizeObserver', Observer);
  const settle = async () => {
    for (let pass = 0; pass < 30; pass++) {
      if (!frames.size && ![...observers].some(observer => observer.pending.size)) return;
      await act(async () => {
        for (const observer of [...observers]) {
          const targets = [...observer.pending]; observer.pending.clear();
          if (targets.length) observer.callback(targets.map(target => ({ target, contentRect: { width: target.textContent.length * 8 }, borderBoxSize: [{ blockSize: 32 }] })));
        }
        const callbacks = [...frames.values()]; frames.clear();
        for (const callback of callbacks) callback(0);
      });
    }
    assert.fail('the synthetic table layout did not settle');
  };
  define('HTMLIFrameElement', class {});
  define('IS_REACT_ACT_ENVIRONMENT', true);
  const tokens = tokensFromValues(() => '#777', 'sans-serif');
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'table-visible-rows-fixture', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/charts/table/Table.tsx')) return source.replace(/\b(setFit|setView|setNatural|setRowMeasurements|setDescription)\(/g,
          '(globalThis.__tableReads.state++, globalThis.__tableReads.$1++, $1)(');
        if (path.endsWith('/charts/useChartTokens.ts')) return `export const useChartTokens = () => (${JSON.stringify(tokens)}); export const useViewportHeight = () => 800;`;
        if (path.endsWith('/components/ContextMenu.tsx')) return 'export const useContextMenu = () => ({ open() {}, menu: null });';
      },
    }] });
  const container = body.appendChild(new FakeNode(1, 'DIV', document));
  const root = createRoot(container as unknown as HTMLElement);
  try {
    const { default: Table, CellBar } = await vite.ssrLoadModule('/ui/src/charts/table/Table.tsx') as typeof import('./Table.tsx');
    const { default: WindowedRows } = await vite.ssrLoadModule('/ui/src/charts/WindowedRows.tsx') as typeof import('../WindowedRows.tsx');
    const { default: ProgressMeter } = await vite.ssrLoadModule('/ui/src/charts/meter/ProgressMeter.tsx') as typeof import('../meter/ProgressMeter.tsx');
    await run({ Table, CellBar, ProgressMeter, WindowedRows, root, container, css, events, settle,
      loadModule: async <T>(path: string) => await vite.ssrLoadModule(path) as T,
      resize(element) { for (const observer of observers) if (observer.targets.has(element)) observer.pending.add(element); } });

  } finally {
    await act(async () => { root.unmount(); });
    await vite.close();
    for (const [key, descriptor] of before) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor); else delete host[key];
    }
  }
}

test('a parent that rebuilds its column accessors on every render does not loop through onVisibleRows', async () => {
  await withTable(async ({ Table, root, container }) => {
    type Row = { key: string; score: number };
    const rows: readonly Row[] = Array.from({ length: 12 }, (_, index) => ({ key: `row:${index}`, score: index }));
    const rowKey = (row: Row) => row.key;
    let reports = 0;
    // Bound reports so a render loop fails instead of hanging the daily run.
    function Parent() {
      const [, setVisible] = useState<readonly Row[]>([]);
      const onVisibleRows = useCallback((shown: readonly Row[]) => { if (++reports <= 40) setVisible(shown); }, []);
      const columns: readonly TableColumn<Row>[] = [
        { key: 'name', header: 'Name', label: 'Name', sortable: false, value: row => row.key },
        { key: 'score', header: 'Score', label: 'Score', numeric: true, value: row => row.score },
      ];
      return createElement(Table<Row>, { label: 'Leaders', rows, columns, rowKey, rank: true, sort: { column: 'score', direction: 'descending' },
        onVisibleRows });
    }
    await act(async () => { root.render(createElement(Parent)); });
    assert.ok(reports <= 2, `the table reported unchanged visible rows ${reports} times`);
    assert.match(container.textContent, /row:11/);
  });
});

test('equal playback rows do no layout, style or state work, while changed layout inputs still measure', async context => {
  for (const scenario of ['equal', 'rank', 'natural', 'resize', 'columns', 'readd', 'appearance', 'colors'] as const) await context.test(scenario, async () => withTable(async ({ Table, root, container, css, events, settle, resize }) => {
    type Row = { key: string; name: string; score: number; hour: number };
    let cellRenders = 0, clicked: Row | undefined;
    function Cell({ name }: { name: string }) { cellRenders++; return createElement('span', null, name); }
    const columnsFor = (width = 'max-content'): readonly TableColumn<Row>[] => [
      { key: 'name', label: 'Name', header: 'Name', value: row => row.name, render: row => createElement(Cell, { name: row.name }), width: 'max-content' },
      { key: 'score', label: 'Score', header: 'Score', value: row => row.score, numeric: true, priority: 1, width },
    ];
    const rows: Row[] = [{ key: 'a', name: 'Alpha', score: 1, hour: 0 }, { key: 'b', name: 'Beta', score: 2, hour: 0 }];
    const draw = async (items: Row[], columns = columnsFor(), density: 'comfortable' | 'compact' = 'comfortable', rowHeight?: number, rank = false) => {
      await act(async () => root.render(createElement(Table<Row>, { rows: items, columns, label: 'Playback', rowKey: row => row.key,
        sort: { column: 'name', direction: 'ascending' }, density, rowHeight, rank, onRowClick: row => { clicked = row; } })));
      await settle();
    };
    const table = () => container.querySelectorAll('table')[0];
    const scroller = () => table().parentNode!;
    const visual = () => ({ text: container.textContent, grid: scroller().style['--table-columns'], height: scroller().style['--table-row-height'] });

    await draw(rows, columnsFor(), 'comfortable', undefined, scenario === 'rank');
    if (scenario === 'equal' || scenario === 'rank') {
      const before = visual(), renders = cellRenders;
      resetReads();
      const next = rows.map(row => ({ ...row, hour: 1 }));
      await draw(next, columnsFor(), 'comfortable', undefined, scenario === 'rank');
      assert.deepEqual(visual(), before);
      assert.equal(reads.layout, 0);
      assert.equal(reads.styles, 0);
      assert.equal(reads.properties, 0);
      assert.equal(reads.setFit, 0);
      assert.equal(reads.state, 0);
      assert.deepEqual(reads.observed, []);
      assert.equal(cellRenders, renders, 'equal cell props do not render the row or its children');
      const row = container.querySelectorAll('tbody tr')[0];
      const reactProps = Object.entries(row).find(([key]) => key.startsWith('__reactProps$'))?.[1] as { onClick(): void };
      reactProps.onClick();
      assert.equal(clicked, next[0], 'a skipped row dispatches its newest row object');
    }
    if (scenario === 'natural') {
      resetReads();
      await draw(rows.map(row => ({ ...row, score: row.score + 100 })));
      assert.ok(reads.observed.includes('score'));
      assert.ok(!reads.observed.includes('name'), 'unchanged custom-rendered cells keep their cached natural measurements');
      assert.ok(reads.layout > 0, 'changed cell content refits');
      const before = cellRenders;
      resetReads();
      await draw(rows.map(row => ({ ...row, score: row.score + 100, name: row.key === 'a' ? 'Alpha extended' : row.name })));
      assert.equal(cellRenders, before + 1, 'a changed custom cell renders only its own row');
      assert.ok(reads.observed.includes('name'));
      assert.ok(!reads.observed.includes('score'));
    }
    if (scenario === 'resize') {
      resetReads();
      table().width = 300; table().requiredWidth = 900;
      scroller().width = 300;
      await act(async () => resize(scroller())); await settle();
      assert.equal(table().querySelectorAll('th').length, 1);
      assert.ok(reads.setFit > 0);
      table().width = 1200; table().requiredWidth = 800;
      scroller().width = 1200;
      await act(async () => resize(scroller())); await settle();
      assert.equal(table().querySelectorAll('th').length, 2);
    }
    if (scenario === 'columns') {
      const extra: TableColumn<Row> = { key: 'extra', label: 'Extra', header: 'Extra', value: row => row.score, width: '60px', priority: 2 };
      const full = [...columnsFor('180px'), extra];
      for (const columns of [columnsFor('180px'), full, [full[0], full[2], full[1]]]) {
        resetReads(); await draw(rows, columns);
        assert.ok(reads.layout > 0);
        assert.equal(reads.typography, 0, 'column changes reuse typography');
      }
      resetReads(); await draw(rows, columnsFor(), 'compact', 40);
      assert.ok(reads.layout > 0);
      assert.equal(reads.typography, 0, 'density and preferred height reuse the stylesheet heights');
      assert.equal(scroller().style['--table-row-height'], '32.5px');
    }
    if (scenario === 'readd') {
      const scoreWidth = () => parseFloat(String(scroller().style['--table-columns']).split(' ').at(-1)!);
      await draw(rows.map(row => ({ ...row, score: 1234567890123 })));
      const wide = scoreWidth();
      await draw(rows, columnsFor().slice(0, 1));
      await draw(rows);
      assert.ok(scoreWidth() < wide, 'a restored column measures its current content without a removed column\'s old maximum');
    }
    if (scenario === 'appearance') {
      await draw(rows, columnsFor(), 'compact', 40);
      resetReads();
      css['--type-sm'] = '18'; css['--font-tables'] = 'Synthetic Font';
      css['--table-compact-height'] = '24';
      document.documentElement.dataset.wordWrap = 'false';
      await act(async () => { events.dispatchEvent(new Event(APPEARANCE_EVENT)); }); await settle();
      assert.equal(reads.typography, 1);
      assert.equal(reads.metrics, 1);
      assert.equal(scroller().style['--table-row-height'], '45px');
      assert.ok(!scroller().getAttribute('class')?.split(' ').includes('is-wrapped'));
      resetReads();
      await draw(rows.map(row => ({ ...row })), columnsFor(), 'compact', 40);
      assert.equal(reads.styles, 0);
      assert.equal(reads.layout, 0);
      assert.equal(reads.state, 0);
    }
    if (scenario === 'colors') {
      await draw(rows.map(row => ({ ...row, score: 1234567890123 })));
      await draw(rows);
      const before = visual();
      resetReads();
      await act(async () => { events.dispatchEvent(new Event(APPEARANCE_EVENT)); }); await settle();
      assert.equal(reads.typography, 1);
      assert.equal(reads.metrics, 1);
      assert.equal(reads.layout, 0);
      assert.equal(reads.setFit, 0);
      assert.equal(reads.state, 0);
      assert.deepEqual(reads.observed, []);
      assert.deepEqual(visual(), before, 'unchanged layout metrics preserve the historical natural grid');
    }
  }));
});

test('windowed rows reuse equal displayed content and keep their latest nested callbacks', async () => {
  await withTable(async ({ WindowedRows, root, container, settle }) => {
    let renders = 0, clicked = 0;
    function Cell({ value }: { value: string }) { renders++; return createElement('span', null, value); }
    const draw = async (hour: number, value = 'Alpha') => {
      await act(async () => root.render(createElement(WindowedRows<{ value: string }>, {
        items: [{ key: 'a', item: { value } }], rowHeight: 24, label: 'Windowed playback',
        children: row => createElement('button', { onClick: () => { clicked = hour; } }, createElement(Cell, { value: row.item.value })),
      })));
      await settle();
    };
    await draw(0);
    resetReads();
    const equalRenders = renders;
    await draw(1);
    assert.equal(renders, equalRenders, 'equal cells skip their row even when its event closures change');
    assert.equal(reads.styles, 0);
    assert.equal(reads.layout, 0);
    const button = container.querySelectorAll('button')[0];
    const props = Object.entries(button).find(([key]) => key.startsWith('__reactProps$'))?.[1] as { onClick(): void };
    props.onClick();
    assert.equal(clicked, 1, 'a skipped nested control dispatches the latest handler');
    const before = renders;
    await draw(2, 'Renamed');
    assert.ok(renders > before);
    assert.match(container.textContent, /Renamed/);
  });
});

test('memoized row controls preserve object refs and attach a replacement ref', async () => {
  await withTable(async ({ WindowedRows, root, container, settle }) => {
    const first = createRef<HTMLButtonElement>(), next = createRef<HTMLButtonElement>();
    const draw = async (ref: typeof first) => {
      await act(async () => root.render(createElement(WindowedRows<string>, {
        items: [{ key: 'a', item: 'Alpha' }], rowHeight: 24, label: 'Focusable row',
        children: row => createElement('button', { ref }, row.item),
      })));
      await settle();
    };
    await draw(first);
    const button = container.querySelectorAll('button')[0];
    assert.equal(first.current, button);
    await draw(first);
    assert.equal(first.current, button);
    await draw(next);
    assert.equal(first.current, null);
    assert.equal(next.current, button);
  });
});

test('bar values, maxima and colours update paint without reading table layout', async context => {
  for (const mode of ['column', 'nested', 'meter', 'key'] as const) await context.test(mode, async () => withTable(async ({ Table, CellBar, ProgressMeter, root, container, settle }) => {
    type Row = { key: string; raw: bigint; maximum: bigint; color: string };
    const text = (row: Row) => `${Math.round(Number(row.raw) / 100000)}%`;
    function ComposedCell({ row }: { row: Row }) {
      return createElement(CellBar, { value: row.raw, maximum: row.maximum,
        children: createElement('span', { style: { color: row.color } }, text(row)) });
    }
    const draw = async (row: Row) => {
      const columns: readonly TableColumn<Row>[] = [
        { key: 'name', label: 'Name', header: 'Name', width: '100px', value: item => item.key },
        { key: 'resistance', label: 'Resistance', header: 'Resistance', width: mode === 'column' ? '150px' : 'max-content', priority: 1,
          value: item => item.raw, bar: mode === 'column' ? row.maximum : undefined, widthKey: mode === 'key' ? text : undefined, render: item => {
            const content = createElement('span', { style: { color: item.color } }, text(item));
            return mode === 'key' ? createElement(ComposedCell, { row: item }) : mode === 'nested'
              ? createElement(CellBar, { value: item.raw, maximum: item.maximum, children: content }) : mode === 'meter'
                ? createElement('span', null, createElement(ProgressMeter, { value: Number(item.raw) / Number(item.maximum), state: 'live' }), content) : content;
          } },
      ];
      await act(async () => root.render(createElement(Table<Row>, { rows: [row], columns, label: 'Resistance', rowKey: item => item.key,
        sort: { column: 'name', direction: 'ascending' } })));
      await settle();
    };
    await draw({ key: 'a', raw: 4133540n, maximum: 10000000n, color: 'red' });
    const before = container.textContent, width = container.querySelectorAll('i')[0].style.width;
    resetReads();
    await draw({ key: 'a', raw: 4149999n, maximum: 20000000n, color: 'blue' });
    assert.equal(container.textContent, before);
    assert.notEqual(container.querySelectorAll('i')[0].style.width, width, 'the bar still paints its exact new fraction');
    assert.ok(container.querySelectorAll('span').some(span => span.style.color === 'blue'));
    assert.equal(reads.layout, 0);
    assert.equal(reads.styles, 0);
    assert.equal(reads.properties, 0);
    assert.equal(reads.setFit, 0);
    assert.equal(reads.state, 0);
    assert.deepEqual(reads.observed, []);
  }));
});

test('a column addition, removal or reorder resets every historical natural maximum', async context => {
  for (const change of ['add', 'remove', 'reorder'] as const) await context.test(change, async () => withTable(async ({ Table, root, container, settle, resize }) => {
    type Row = { key: string; name: string };
    const name: TableColumn<Row> = { key: 'name', label: 'Name', header: 'Name', width: 'max-content', value: row => row.name };
    const first: TableColumn<Row> = { key: 'first', label: 'First', header: 'First', width: '100px', priority: 1, value: () => 1 };
    const second: TableColumn<Row> = { key: 'second', label: 'Second', header: 'Second', width: '70px', priority: 2, value: () => 2 };
    const extra: TableColumn<Row> = { key: 'extra', label: 'Extra', header: 'Extra', width: '50px', priority: 3, value: () => 3 };
    const original = [name, first, second];
    const draw = async (text: string, columns: readonly TableColumn<Row>[]) => {
      await act(async () => root.render(createElement(Table<Row>, { rows: [{ key: 'a', name: text }], columns, label: 'Natural reset', rowKey: row => row.key,
        sort: { column: 'name', direction: 'ascending' } })));
      await settle();
    };
    await draw('A synthetic country name with a long historic maximum', original);
    const table = container.querySelectorAll('table')[0], scroller = table.parentNode!;
    const nameWidth = () => parseFloat(String(scroller.style['--table-columns']).split(' ')[0]);
    table.width = 300; scroller.width = 300;
    table.requiredWidth = () => String(scroller.style['--table-columns']).split(' ').reduce((sum, track) => sum + (track === 'max-content'
      ? Math.max(0, ...table.querySelectorAll('[data-table-natural]').map(cell => cell.textContent.length * 8)) : parseFloat(track) || 0), 0);
    await act(async () => resize(scroller)); await settle();
    const wide = nameWidth();
    await draw('Alpha', original);
    assert.equal(nameWidth(), wide, 'the unchanged columns retain the historical maximum');
    assert.ok(table.querySelectorAll('th').length < original.length, 'the oversized name keeps a measure hidden');
    resetReads();
    const next = change === 'add' ? [...original, extra] : change === 'remove' ? [name, first] : [name, second, first];
    await draw('Alpha', next);
    assert.ok(nameWidth() < wide, 'the unchanged name column is measured again at its current width');
    assert.equal(table.querySelectorAll('th').length, next.length, 'the narrower current contents let every column fit');
    assert.ok(reads.observed.includes('name'));
  }));
});

test('windowed chart memoization leaves histories opaque and preserves callback consumer handoffs', async () => {
  await withTable(async ({ WindowedRows, root, container, settle }) => {
    let dataReads = 0, picked = -1;
    const opaque = { get() { dataReads++; throw new Error('chart history was traversed'); },
      ownKeys() { dataReads++; throw new Error('chart history was enumerated'); } };
    const runs = new Proxy(new Array<unknown>(100000), opaque), summaries = new Proxy({ runs }, opaque), pickOwner = new Proxy({ runs, summaries }, opaque);
    type Props = { playhead: number; runs: readonly unknown[]; summaries: object; pickOwner: object; onSummaryView: (hour: number) => void };
    const rendered: Props[] = [], first: number[] = [], next: number[] = [];
    const consumeFirst = (hour: number) => { first.push(hour); }, consumeNext = (hour: number) => { next.push(hour); };
    function Chart(props: Props) {
      assert.equal(props.runs, runs); assert.equal(props.summaries, summaries); assert.equal(props.pickOwner, pickOwner);
      rendered.push(props);
      useLayoutEffect(() => { props.onSummaryView(props.playhead); }, [props.playhead, props.onSummaryView]);
      return createElement('span', null, props.playhead);
    }
    const draw = async (hour: number, consumer = consumeFirst) => {
      await act(async () => root.render(createElement(WindowedRows<string>, { items: [{ key: 'a', item: 'a' }], rowHeight: 24, label: 'History chart',
        children: () => createElement('section', { onClick: () => { picked = hour; } },
          createElement(Chart, { playhead: hour, runs, summaries, pickOwner, onSummaryView: consumer })) })));
      await settle();
    };
    const nativeHandler = () => (Object.entries(container.querySelectorAll('section')[0]).find(([key]) => key.startsWith('__reactProps$'))?.[1] as { onClick(): void }).onClick;
    await draw(0);
    const click = nativeHandler();
    await draw(1);
    assert.equal(dataReads, 0);
    assert.equal(rendered.length, 2, 'changed playheads still render their chart');
    assert.equal(click, nativeHandler(), 'a native element keeps one stable wrapper per handler');
    click();
    assert.equal(picked, 1, 'the stable native handler dispatches the latest committed callback');
    await draw(1);
    assert.equal(rendered.length, 2, 'equal content and data identities skip the row');
    await draw(1, consumeNext);
    assert.equal(rendered.at(-1)!.onSummaryView, consumeNext, 'custom component callbacks retain their original identity');
    assert.deepEqual(first, [0, 1]);
    assert.deepEqual(next, [1], 'a new viewport consumer receives the unchanged viewport');
    assert.equal(dataReads, 0);
  });
});

test('the actual Missing cell keys formatted text and font weight rather than raw operands', async () => {
  await withTable(async ({ Table, root, container, settle, loadModule }) => {
    const { missingCell } = await loadModule<typeof import('../../widgets/division-equipment/DivisionEquipmentWidget.tsx')>('/ui/src/widgets/division-equipment/DivisionEquipmentWidget.tsx');
    type Row = { key: string; raw: string };
    const columns: readonly TableColumn<Row>[] = [
      { key: 'name', label: 'Name', header: 'Name', width: '100px', value: row => row.key },
      { key: 'missing', label: 'Missing', header: 'Missing', width: 'max-content', priority: 1, numeric: true,
        value: row => BigInt(row.raw), render: row => missingCell(row.raw) },
    ];
    const draw = async (raw: string) => {
      await act(async () => root.render(createElement(Table<Row>, { rows: [{ key: 'a', raw }], columns, label: 'Missing equipment', rowKey: row => row.key,
        sort: { column: 'name', direction: 'ascending' } })));
      await settle();
    };
    await draw('4133540');
    const before = container.textContent;
    assert.equal(container.querySelectorAll('b')[0].textContent, '41');
    resetReads();
    await draw('4149999');
    assert.equal(container.textContent, before);
    assert.equal(reads.layout, 0);
    assert.equal(reads.styles, 0);
    assert.equal(reads.setFit, 0);
    assert.equal(reads.state, 0);
    assert.deepEqual(reads.observed, []);
    await draw('40000');
    const zero = container.textContent;
    resetReads();
    await draw('0');
    assert.equal(container.textContent, zero);
    assert.equal(container.querySelectorAll('b').length, 0);
    assert.ok(reads.layout > 0, 'bold zero and plain zero have distinct rendered widths');
  });
});

test('unchanged custom-cell styles and colour-only changes do no layout or style reads', async () => {
  await withTable(async ({ Table, root, container, settle }) => {
    let style: CSSProperties = { color: 'red', fontWeight: 500 };
    function CustomCell({ style, children }: { style: CSSProperties; children: ReactNode }) { return createElement('span', { style }, children); }
    type Row = { key: string; hour: number };
    const columns: readonly TableColumn<Row>[] = [
      { key: 'name', label: 'Name', header: 'Name', width: 'max-content', value: () => 'Alpha',
        render: () => createElement(CustomCell, { style, children: 'Alpha' }) },
      { key: 'value', label: 'Value', header: 'Value', width: '80px', priority: 1, value: () => 1 },
    ];
    const draw = async (hour: number) => {
      await act(async () => root.render(createElement(Table<Row>, { rows: [{ key: 'a', hour }], columns, label: 'Styled cell', rowKey: row => row.key,
        sort: { column: 'name', direction: 'ascending' } })));
      await settle();
    };
    await draw(0);
    for (const next of [style, { ...style }, { ...style, color: 'blue' }]) {
      style = next;
      resetReads();
      await draw(1);
      assert.equal(reads.layout, 0);
      assert.equal(reads.styles, 0);
      assert.equal(reads.setFit, 0);
      assert.equal(reads.state, 0);
      assert.deepEqual(reads.observed, []);
    }
    assert.ok(container.querySelectorAll('span').some(span => span.style.color === 'blue'));
    style = { ...style, fontWeight: 700 };
    resetReads();
    await draw(2);
    assert.ok(reads.layout > 0, 'a changed font weight still invalidates the width key');
  });
});

test('Generals identity artwork, flags and hidden skill operands are paint-only width inputs', async () => {
  await withTable(async ({ Table, root, container, settle, loadModule }) => {
    const { LeaderIdentity } = await loadModule<typeof import('../../widgets/generals/LeaderIdentity.tsx')>('/ui/src/widgets/generals/LeaderIdentity.tsx');
    let identity: Parameters<typeof LeaderIdentity>[0] = { name: 'Synthetic leader', rank: 'General', hasOwner: true, level: 1, maximum: 5,
      countryName: 'First owner', nameFailure: undefined, flag: undefined, flagReason: undefined, flagDetail: undefined, flagPending: true,
      image: undefined, portraitFailure: undefined, portraitMissing: undefined };
    type Row = { key: string; hour: number };
    const columns: readonly TableColumn<Row>[] = [
      { key: 'name', label: 'Leader', header: 'Leader', width: 'max-content', value: () => identity.name,
        render: () => createElement(LeaderIdentity, identity) },
      { key: 'value', label: 'Value', header: 'Value', width: '80px', priority: 1, value: () => 1 },
    ];
    const draw = async (hour: number) => {
      await act(async () => root.render(createElement(Table<Row>, { rows: [{ key: 'a', hour }], columns, label: 'Identity width', rowKey: row => row.key,
        sort: { column: 'name', direction: 'ascending' } })));
      await settle();
    };
    await draw(0);
    const before = container.textContent;
    identity = { ...identity, image: 'synthetic-portrait', flag: 'synthetic-flag', flagPending: false, flagReason: 'Changed metadata',
      countryName: 'Second owner', level: 9, maximum: 10 };
    resetReads();
    await draw(1);
    assert.equal(container.textContent, before);
    assert.equal(reads.layout, 0);
    assert.equal(reads.styles, 0);
    assert.equal(reads.setFit, 0);
    assert.equal(reads.state, 0);
    assert.deepEqual(reads.observed, []);
    assert.ok(container.querySelectorAll('img').some(img => img.getAttribute('src') === 'synthetic-portrait'));
    identity = { ...identity, rank: 'Field marshal' };
    resetReads();
    await draw(2);
    assert.ok(reads.layout > 0, 'a changed displayed rank still refits');
  });
});

test('a full star row changes only its coloured split without measuring again', async () => {
  await withTable(async ({ Table, root, container, settle, loadModule }) => {
    const { SkillStars } = await loadModule<typeof import('../../widgets/generals/LeaderIdentity.tsx')>('/ui/src/widgets/generals/LeaderIdentity.tsx');
    type Row = { key: string; level: number };
    const columns: readonly TableColumn<Row>[] = [
      { key: 'name', label: 'Name', header: 'Name', width: '100px', value: row => row.key },
      { key: 'skill', label: 'Skill', header: 'Skill', width: '92px', priority: 1, value: row => row.level,
        render: row => createElement(SkillStars, { level: row.level, maximum: 5 }) },
    ];
    const draw = async (level: number) => {
      await act(async () => root.render(createElement(Table<Row>, { rows: [{ key: 'a', level }], columns, label: 'Star paint', rowKey: row => row.key,
        sort: { column: 'name', direction: 'ascending' } })));
      await settle();
    };
    await draw(2);
    const before = container.textContent;
    resetReads();
    await draw(3);
    assert.equal(container.textContent, before);
    assert.equal(container.querySelectorAll('em')[0].textContent, '★★');
    assert.equal(reads.layout, 0);
    assert.equal(reads.styles, 0);
    assert.equal(reads.setFit, 0);
    assert.equal(reads.state, 0);
  });
});

test('compact battle counts use their displayed format without full-mode measurement', async () => {
  await withTable(async ({ Table, root, container, settle, loadModule }) => {
    const { default: BattleSides } = await loadModule<typeof import('../../widgets/battles/BattleSides.tsx')>('/ui/src/widgets/battles/BattleSides.tsx');
    type Row = import('../../widgets/battles/battleRows').BattleRow;
    const summary: import('../../widgets/battles/battleRows').BattleSideSummary = { label: 'Units', countLabel: 'Units', separator: '/', unknown: 'Unknown',
      formatCount: (value, compact) => value.value == null ? 'Unknown' : String(compact ? Math.floor(value.value / 100000) : value.value) };
    const columns: readonly TableColumn<Row>[] = [
      { key: 'name', label: 'Name', header: 'Name', width: '100px', value: row => row.id },
      { key: 'sides', label: 'Sides', header: 'Sides', width: '160px', priority: 1, value: row => row.id,
        render: row => createElement(BattleSides, { row, compact: true, summary, country: () => ({ name: 'Synthetic country' }) }) },
    ];
    const draw = async (value: number) => {
      const row = { id: 'a', sides: [{ label: 'Side', countries: [], counts: [{ label: 'Units', value }], losses: [] }] } as unknown as Row;
      await act(async () => root.render(createElement(Table<Row>, { rows: [row], columns, label: 'Compact sides', rowKey: item => item.id,
        sort: { column: 'name', direction: 'ascending' } })));
      await settle();
    };
    const displayed = () => container.querySelectorAll('span').filter(node => node.getAttribute('class') === 'battle-count').map(node => node.textContent);
    await draw(4133540);
    const before = displayed();
    resetReads();
    await draw(4149999);
    assert.deepEqual(displayed(), before);
    assert.equal(reads.layout, 0);
    assert.equal(reads.styles, 0);
    assert.equal(reads.setFit, 0);
    assert.equal(reads.state, 0);
  });
});
