import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { tokensFromValues } from '../../charts/tokens.ts';
import { countryBalances, decodeTransfers } from './territoryData.ts';
import type { TransferLabels } from './territoryTooltip';
import { deal, holder, reply } from './transfers.fixture.ts';
import type { Move } from './transfers.fixture.ts';

const tokens = tokensFromValues(() => '#777', 'sans-serif');
const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
  server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'territory-tables-fixture', enforce: 'pre',
    transform(_source, id) {
      const path = id.replaceAll('\\', '/');
      if (path.endsWith('/charts/useChartTokens.ts')) return `export const useChartTokens = () => (${JSON.stringify(tokens)}); export const useViewportHeight = () => 800;`;
      if (path.endsWith('/components/ContextMenu.tsx')) return 'export const useContextMenu = () => ({ open() {}, menu: null });';
    },
  }] });
const { CountryTable, TransferTable } = await vite.ssrLoadModule('/ui/src/widgets/territory-transfers/TransferTables.tsx').finally(() => vite.close()) as typeof import('./TransferTables.tsx');

const names: Record<string, string> = { LOS: 'Loserland', WIN: 'Winnerland', NEW: 'Freedonia' };
const labels: TransferLabels = {
  country: side => ({ name: names[side.tag] ?? 'Country name unavailable' }),
  tag: tag => ({ name: names[tag] ?? 'Country name unavailable' }),
  state: occurrence => `State name ${occurrence.state}`,
};
const move = (state: number, from: Move['from'], to: Move['to'], extra: Partial<Move> = {}): Move => ({ state, from, to, before: 100, after: 101, ...extra });
const los = holder(1, 'LOS', 100), win = holder(2, 'WIN', 101), free = holder(3, 'NEW', 101);
const data = decodeTransfers(reply([deal('peace', [move(1, los, win, { vp: '5' }), move(2, los, win, { vp: '4' }), move(3, los, free, { vp: null, released: true })])]));
const shared = { labels, cellBars: true, density: 'comfortable' as const, narrow: 'hide' as const, hoursPerSecond: 1, menu: () => [], onVisibleRows() {} };
const cells = (html: string) => [...html.matchAll(/<td[^>]*>(.*?)<\/td>/g)].map(match => match[1].replace(/<[^>]+>/g, '').trim());

test('the actual Countries table nets each holder, colours signed values and leaves an unknown VP blank', () => {
  const html = renderToStaticMarkup(createElement(CountryTable, { ...shared, rows: countryBalances(data.all), sort: { column: 'net', direction: 'descending' },
    onPick() {}, tooltip: () => ({ title: '', rows: [] }) }));
  assert.deepEqual(cells(html), ['Winnerland', '2', '0', '+2', '+9', 'Freedonia', '1', '0', '+1', '—', 'Loserland', '0', '3', '−3', '—']);
  assert.match(html, /class="transfer-gain">\+2</);
  assert.match(html, /class="transfer-loss">−3</);
});

test('the actual Transfers table marks a release and, in a country drill, signs VP from its side', () => {
  const all = renderToStaticMarkup(createElement(TransferTable, { ...shared, rows: data.all, sort: { column: 'state', direction: 'ascending' },
    tooltip: () => ({ title: '', rows: [] }), label: 'Territory transfers' }));
  const rows = cells(all);
  assert.deepEqual(rows.slice(10, 15).map(cell => cell.replace(/\s+/g, ' ')), ['State name 3', 'Loserland', 'Freedonia · released', rows[13], '—']);
  assert.doesNotMatch(all, /\bBy\b/);
  const drill = renderToStaticMarkup(createElement(TransferTable, { ...shared, rows: data.all.filter(item => item.from?.key === '[1,1001]'), focus: '[1,1001]',
    sort: { column: 'state', direction: 'ascending' }, tooltip: () => ({ title: '', rows: [] }), label: 'Country territory transfers' }));
  assert.deepEqual(cells(drill).filter((_, index) => index % 5 === 4), ['−5', '−4', '—']);
  assert.match(drill, /transfer-country is-quiet/);
});
