import assert from 'node:assert/strict';
import test from 'node:test';
import { act, createElement, useCallback, useState } from 'react';
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
  scrollTop = 0; scrollHeight = 0; clientHeight = 600; clientWidth = 800;
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
  get isConnected() { return true; }
  get textContent(): string { return this.nodeType === 3 ? this.nodeValue ?? '' : this.childNodes.map(child => child.textContent).join(''); }
  set textContent(text: string) { this.childNodes = []; if (text) this.appendChild(Object.assign(new FakeNode(3, '#text', this.ownerDocument), { nodeValue: text })); }
  appendChild(child: FakeNode) { return this.insertBefore(child, null); }
  insertBefore(child: FakeNode, before: FakeNode | null) {
    child.parentNode?.removeChild(child);
    const index = before ? this.childNodes.indexOf(before) : -1;
    this.childNodes.splice(index < 0 ? this.childNodes.length : index, 0, child);
    child.parentNode = this;
    return child;
  }
  removeChild(child: FakeNode) { this.childNodes.splice(this.childNodes.indexOf(child), 1); child.parentNode = null; return child; }
  setAttribute(name: string, value: string) { this.attributes.set(name, String(value)); }
  removeAttribute(name: string) { this.attributes.delete(name); }
  getAttribute(name: string) { return this.attributes.get(name) ?? null; }
  hasAttribute(name: string) { return this.attributes.has(name); }
  addEventListener() {}
  removeEventListener() {}
  querySelectorAll() { return []; }
  contains(node: FakeNode | null): boolean { return node === this || this.childNodes.some(child => child.contains(node)); }
  getRootNode() { return this.ownerDocument ?? this; }
  focus() {}
}

const CSS: Record<string, string> = { '--table-comfortable-height': '32', '--table-compact-height': '26', '--table-header-min-height': '24',
  '--type-sm': '12', width: '40px', paddingLeft: '0', paddingRight: '0' };

test('a parent that rebuilds its column accessors on every render does not loop through onVisibleRows', async () => {
  const document = new FakeNode(9, '#document');
  const html = new FakeNode(1, 'HTML', document), body = new FakeNode(1, 'BODY', document);
  html.appendChild(body);
  Object.assign(document, { documentElement: html, body, activeElement: body,
    createElement: (name: string) => new FakeNode(1, name.toUpperCase(), document),
    createElementNS: (_namespace: string, name: string) => new FakeNode(1, name, document),
    createTextNode: (text: string) => Object.assign(new FakeNode(3, '#text', document), { nodeValue: text }) });
  const replaced = ['document', 'window', 'getComputedStyle', 'ResizeObserver', 'cancelAnimationFrame', 'HTMLIFrameElement', 'IS_REACT_ACT_ENVIRONMENT'] as const;
  const host = globalThis as unknown as Record<string, unknown>;
  const before = new Map(replaced.map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
  const define = (key: string, value: unknown) => Object.defineProperty(globalThis, key, { configurable: true, writable: true, value });
  define('document', document);
  define('window', globalThis);
  define('getComputedStyle', () => ({ ...CSS, getPropertyValue: (name: string) => CSS[name] ?? '' }));
  define('cancelAnimationFrame', () => {});
  define('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} });
  define('HTMLIFrameElement', class {});
  define('IS_REACT_ACT_ENVIRONMENT', true);
  const tokens = tokensFromValues(() => '#777', 'sans-serif');
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'table-visible-rows-fixture', enforce: 'pre',
      transform(_source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/charts/useChartTokens.ts')) return `export const useChartTokens = () => (${JSON.stringify(tokens)});`;
        if (path.endsWith('/components/ContextMenu.tsx')) return 'export const useContextMenu = () => ({ open() {}, menu: null });';
      },
    }] });
  const container = document.appendChild(new FakeNode(1, 'DIV', document));
  const root = createRoot(container as unknown as HTMLElement);
  try {
    const { default: Table } = await vite.ssrLoadModule('/ui/src/charts/table/Table.tsx') as typeof import('./Table.tsx');
    type Row = { key: string; score: number };
    const rows: readonly Row[] = Array.from({ length: 12 }, (_, index) => ({ key: `row:${index}`, score: index }));
    const rowKey = (row: Row) => row.key;
    let reports = 0;
    // Like the Generals leaderboard: visible rows feed per-row art, and the columns are
    // rebuilt (with new accessors) whenever that art changes. Stop after a bounded
    // number of reports so a loop fails the assertion instead of hanging the test.
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
  } finally {
    await act(async () => { root.unmount(); });
    await vite.close();
    for (const [key, descriptor] of before) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor); else delete host[key];
    }
  }
});
