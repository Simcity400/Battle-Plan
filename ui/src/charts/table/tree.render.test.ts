import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { tokensFromValues } from '../tokens.ts';
import { treeRows } from './logic.ts';
import type { TableTree } from './logic';
import type { TableColumn } from './columns';

const tokens = tokensFromValues(() => '#777', 'sans-serif');
const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
  server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'table-tree-fixture', enforce: 'pre',
    transform(_source, id) {
      const path = id.replaceAll('\\', '/');
      if (path.endsWith('/charts/useChartTokens.ts')) return `export const useChartTokens = () => (${JSON.stringify(tokens)}); export const useViewportHeight = () => 800;`;
      if (path.endsWith('/components/ContextMenu.tsx')) return 'export const useContextMenu = () => ({ open() {}, menu: null });';
    },
  }] });
const { default: Table } = await vite.ssrLoadModule('/ui/src/charts/table/Table.tsx').finally(() => vite.close()) as typeof import('./Table.tsx');

type Row = { key: string; value: bigint | null; children?: Row[] };
const columns: readonly TableColumn<Row>[] = [
  { key: 'name', header: 'Name', label: 'Name', value: row => row.key,
    render: row => createElement('span', { 'data-table-key': row.key }, row.key) },
  { key: 'score', header: 'Score', label: 'Score', numeric: true, value: row => row.value },
];
const rowKey = (row: Row) => row.key;
const children = (row: Row) => row.children;
const keys = (html: string) => [...html.matchAll(/data-table-key="([^"]+)"/g)].map(match => match[1]);
const render = (rows: readonly Row[], tree: TableTree<Row>, unsorted = false) => renderToStaticMarkup(createElement(Table<Row>, {
  label: 'Nested rows', rows, columns, rowKey, tree, unsorted, sort: { column: 'score', direction: 'descending' } }));

const queue: Row[] = [
  { key: 'line:1', value: 1n, children: [{ key: 'unit:a', value: 5n }, { key: 'unit:b', value: 9n }] },
  { key: 'line:2', value: 3n, children: [] },
  { key: 'line:3', value: 2n, children: [{ key: 'unit:c', value: 1n }] },
  { key: 'plain', value: 0n },
];

test('group header rows carry their rows beneath them, each level sorted on its own', () => {
  const html = render(queue, { children });
  assert.deepEqual(keys(html), ['line:2', 'line:3', 'unit:c', 'line:1', 'unit:b', 'unit:a', 'plain']);
  assert.match(html, /role="treegrid"/);
  assert.match(html, /aria-rowcount="8"/);
  assert.equal((html.match(/aria-level="2"/g) ?? []).length, 3);
  assert.equal((html.match(/aria-expanded="true"/g) ?? []).length, 2, 'an empty group has nothing to open');
  assert.equal((html.match(/class="table-disclosure"/g) ?? []).length, 3, 'every group header row keeps the disclosure column');
  assert.equal((html.match(/<path /g) ?? []).length, 2, 'only groups with rows draw the disclosure mark');
  assert.equal((html.match(/class="table-tree is-group"/g) ?? []).length, 3);
});

test('the given order holds at every level when the rows are unsorted, and headers offer no sorting', () => {
  const html = render(queue, { children }, true);
  assert.deepEqual(keys(html), ['line:1', 'unit:a', 'unit:b', 'line:2', 'line:3', 'unit:c', 'plain']);
  assert.doesNotMatch(html, /aria-sort=/);
  assert.doesNotMatch(html, /<button/);
});

test('closed groups leave their rows out, whether closed initially or by the widget', () => {
  const initially = render(queue, { children, initiallyOpen: row => row.key !== 'line:1' }, true);
  assert.deepEqual(keys(initially), ['line:1', 'line:2', 'line:3', 'unit:c', 'plain']);
  assert.match(initially, /aria-expanded="false"/);
  let asked = 0;
  const controlled = render(queue, { children, open: row => { asked++; return row.key === 'line:1'; } }, true);
  assert.deepEqual(keys(controlled), ['line:1', 'unit:a', 'unit:b', 'line:2', 'line:3', 'plain']);
  assert.ok(asked > 0);
});

test('tree rows skip repeated keys, so a cycle cannot repeat rows, and can include closed groups', () => {
  const loop: Row = { key: 'loop', value: 1n };
  loop.children = [loop, { key: 'inner', value: 2n }];
  const { rows, nesting } = treeRows([loop], children, () => true, list => list.slice(), rowKey);
  assert.deepEqual(rows.map(rowKey), ['loop', 'inner']);
  assert.deepEqual(nesting.get('inner'), { level: 2, parent: 'loop', group: false, open: false, position: 2, size: 2, count: 0 });
  const closed = treeRows(queue, children, () => false, list => list.slice(), rowKey);
  assert.deepEqual(closed.rows.map(rowKey), ['line:1', 'line:2', 'line:3', 'plain']);
  const every = treeRows(queue, children, () => false, list => list.slice(), rowKey, true);
  assert.equal(every.rows.length, 7);
  assert.equal(every.nesting.get('line:1')?.open, false);
});

test('five thousand groups of nested rows stay one windowed table', () => {
  const rows: Row[] = Array.from({ length: 5000 }, (_, group) => ({ key: `line:${group}`, value: BigInt(group),
    children: Array.from({ length: 3 }, (_, unit) => ({ key: `unit:${group}:${unit}`, value: BigInt(unit) })) }));
  const html = render(rows, { children }, true);
  const mounted = keys(html);
  assert.ok(mounted.length > 0 && mounted.length <= 32);
  const flat = rows.flatMap(row => [row.key, ...row.children!.map(rowKey)]);
  const first = flat.indexOf(mounted[0]);
  assert.deepEqual(mounted, flat.slice(first, first + mounted.length), 'the mounted window is a run of the nested order');
  assert.match(html, /aria-rowcount="20001"/);
});
