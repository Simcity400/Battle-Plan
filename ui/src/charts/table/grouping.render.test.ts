import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { tokensFromValues } from '../tokens.ts';
import type { TableGroup } from './logic';
import type { TableColumn } from './columns';

const tokens = tokensFromValues(() => '#777', 'sans-serif');
const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
  server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'table-grouping-fixture', enforce: 'pre',
    transform(_source, id) {
      const path = id.replaceAll('\\', '/');
      if (path.endsWith('/charts/useChartTokens.ts')) return `export const useChartTokens = () => (${JSON.stringify(tokens)}); export const useViewportHeight = () => 800;`;
      if (path.endsWith('/components/ContextMenu.tsx')) return 'export const useContextMenu = () => ({ open() {}, menu: null });';
    },
  }] });
const { default: Table } = await vite.ssrLoadModule('/ui/src/charts/table/Table.tsx').finally(() => vite.close()) as typeof import('./Table.tsx');

type Row = { key: string; group: TableGroup; value: bigint | null };
const columns: readonly TableColumn<Row>[] = [
  { key: 'country', header: 'Country', label: 'Country', value: row => row.key,
    render: row => createElement('span', { 'data-table-key': row.key }, row.key) },
  { key: 'score', header: 'Score', label: 'Score', numeric: true, bar: 'visible', value: row => row.value },
];
const rowKey = (row: Row) => row.key;
const groupBy = (row: Row) => row.group;
const render = (rows: readonly Row[], grouped: boolean, direction: 'ascending' | 'descending' = 'descending') =>
  renderToStaticMarkup(createElement(Table<Row>, { label: 'Native score groups', rows, columns, rowKey,
    groupBy: grouped ? groupBy : undefined, sort: { column: 'score', direction }, rank: true, onRowClick() {} }));
const keys = (html: string) => [...html.matchAll(/data-table-key="([^"]+)"/g)].map(match => match[1]);

test('the actual table keeps factions together across column direction changes and preserves its ungrouped default', () => {
  const rows: Row[] = [
    { key: 'small', group: { key: 'faction:2', label: 'Same name', orderKey: 2n }, value: -1n },
    { key: 'large', group: { key: 'faction:2', label: 'Same name', orderKey: 2n }, value: 9007199254740993n },
    { key: 'other', group: { key: 'faction:10', label: 'Same name', orderKey: 10n }, value: 9007199254740994n },
    { key: 'unknown', group: { key: 'faction:2', label: 'Same name', orderKey: 2n }, value: null },
  ];
  const descending = render(rows, true);
  assert.deepEqual(keys(descending), ['large', 'small', 'unknown', 'other']);
  assert.deepEqual(keys(render(rows, true, 'ascending')), ['small', 'large', 'unknown', 'other']);
  assert.deepEqual(keys(render(rows, false)), ['other', 'large', 'small', 'unknown']);
  assert.match(descending, /aria-rowcount="5"/);
  assert.equal((descending.match(/aria-rowindex=/g) ?? []).length, rows.length);
  assert.equal((descending.match(/tabindex="0"/g) ?? []).length, rows.length);
  assert.equal((descending.match(/is-activatable/g) ?? []).length, rows.length);
  assert.match(descending, /9007199254740993/);
});

test('five thousand distinct faction identities retain one windowed table and native row slots', () => {
  const rows: Row[] = Array.from({ length: 5000 }, (_, index) => ({ key: `row:${index}`,
    group: { key: `faction:${4999 - index}`, label: 'Same name', orderKey: BigInt(4999 - index) },
    value: BigInt(5000 - index) }));
  const html = render(rows, true);
  const mounted = keys(html).map(key => Number(key.slice(4)));
  assert.ok(mounted.length > 0 && mounted.length <= 32);
  assert.deepEqual(mounted, [...mounted].sort((a, b) => b - a), 'the mounted window follows faction identity, not the global score sort');
  assert.match(html, /aria-rowcount="5001"/);
  assert.equal((html.match(/<table /g) ?? []).length, 1);
  assert.equal((html.match(/aria-rowindex=/g) ?? []).length, mounted.length);
  assert.equal((html.match(/is-activatable/g) ?? []).length, mounted.length);
});
