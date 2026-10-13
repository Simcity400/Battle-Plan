import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import type { ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { TableProps } from '../charts/table/Table';
import type { WidgetProps } from './definitions';
import type { SpendingTableRow } from './politicsSpending';
import type { PoliticsLedgerReply } from '../generated/dashboard';

const DAY = 24;
const start = 43_800_000 + DAY * 365 * 1944;

for (const currency of ['Political', 'Command'] as const) test(`${currency} spending renders a dense table of payments with folded daily costs and a plain summary`, async () => {
  const payment = (id: string, hour: number, category: string, itemId: string, chargeRaw: string) => ({ id, hour, readHour: hour, sequence: '1', ordinal: 0,
    countryTag: 'native actor', countryName: 'Recorded actor', currency, category, itemId, chargeRaw, realizedDeltaRaw: `-${chargeRaw}`, covered: true, occurrences: 1,
    changes: [{ readHour: hour, chargeRaw, realizedDeltaRaw: `-${chargeRaw}`, covered: true, occurrences: 1 }] });
  const data: PoliticsLedgerReply = { branchId: 'branch', tag: 'native actor', decimals: 5, warnings: [], ideaTooltips: null, commandHolds: [
    { readHour: start, throughHour: start + 100 * DAY, totalRaw: '-7000000', holders: [{ id: 'notIdentified', kind: 'Not identified', name: null, amountRaw: '-7000000' }], reason: 'private' }],
  spending: [
    payment('raid', start + 5 * DAY, 'raids', 'raid', '2000000'),
    ...Array.from({ length: 40 }, (_, day) => payment(`cost ${day}`, start + day * DAY, 'dailyCosts', 'cost', '100001')),
  ],
  spendItems: [
    { id: 'raid', groupId: 'raid', name: null, action: 'Raids: Not identified', icon: null, tooltipToken: null, character: null, targetTag: null, targetName: null, unknown: 'private' },
    { id: 'cost', groupId: 'cost', name: 'Recorded cost modifier', action: 'Daily costs: Recorded cost modifier', icon: null, tooltipToken: null, character: null, targetTag: null, targetName: null, unknown: null },
  ],
  spendCategories: [{ id: 'raids', name: 'Raids', identified: true }, { id: 'dailyCosts', name: 'Daily costs', identified: true }],
  daily: Array.from({ length: 40 }, (_, day) => ({ country: 1, instance: 1, epoch: '1', dayHour: start + day * DAY, readHour: start + day * DAY,
    ppGrossRaw: '250000', ppCostRaw: '100000', ppIncomeRaw: '150000', ppRealizedRaw: '150000', ppSequence: '1',
    cpIncomeRaw: '83000', cpRealizedRaw: '83000', cpSequence: '1' })),
  spendingCoverage: [{ readHour: start, sinceHour: start, throughHour: start + 1, political: false, command: false, reason: 'private' },
    { readHour: start + 2, sinceHour: start + 2, throughHour: start + 2, political: false, command: false, reason: null },
    { readHour: start + 3, sinceHour: start + 3, throughHour: start + 100 * DAY, political: true, command: true, reason: null }] };
  const state = { table: undefined as TableProps<SpendingTableRow> | undefined, clock: start + 45 * DAY, tags: ['native actor'] };
  const host = globalThis as typeof globalThis & { __spendingTable?: typeof state };
  host.__spendingTable = state;
  const replacements: Record<string, string> = {
    '/dashboard/usePoliticsSpending.ts': `export const usePoliticsSpending = (_widget, runtime) => ({track: runtime.tracks[0], tags: globalThis.__spendingTable.tags,
      data: [${JSON.stringify(data)}], ready: true, groupWarnings: [], selection: {matches: () => true, membership: null, unobserved: []}});
      export const useSpendingChoices = () => ({}); export const useSpendingIdeaTooltips = () => undefined;`,
    '/dashboard/countryFlagsAt.ts': `export const useCountryLabelsAt = () => ({labelAt: hour => tag => ({name: 'Dated ' + tag + ' at ' + hour, known: true})});`,
    '/charts/table/useTableHour.ts': 'export const useTableHour = () => globalThis.__spendingTable.clock;',
    '/dashboard/playback.ts': 'export const usePlaybackSpeed = () => 1; export const usePlaybackHour = () => globalThis.__spendingTable.clock;',
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({categories: ['#123456', '#654321'], muted: '#888888'}); export const useViewportHeight = () => 800;`,
    '/charts/table/Table.tsx': 'export default props => {globalThis.__spendingTable.table = props; return null;}; export const CountryCell = () => null;',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'spending-table-adapter', enforce: 'pre',
      transform(_source, id) { const path = id.replaceAll('\\', '/'); return Object.entries(replacements).find(([suffix]) => path.endsWith(suffix))?.[1]; } }] });
  try {
    const kind = currency === 'Political' ? 'political-power-spending' : 'command-power-spending';
    const { default: registered } = await vite.ssrLoadModule(`/ui/src/widgets/${kind}/index.ts`);
    const Widget = registered.render as ComponentType<WidgetProps>;
    const track = { id: 'track', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [], branches: [] } },
      branch: { id: 'branch', firstHour: start, lastHour: start + 100 * DAY, gaps: [] } };
    const props = { widget: { id: 'spending', kind, configVersion: 1, trackIds: ['track'], countries: { kind: 'tags', tags: ['native actor'] },
      time: { mode: 'follow' }, options: {} }, runtime: { client: {}, tracks: [track], clock: { getHour: () => state.clock, getSnapshot: () => ({ hoursPerSecond: 1 }) } },
    active: true, onDiagnostics() {}, setOptions() {}, setOption() {} } as unknown as WidgetProps;
    const markup = renderToStaticMarkup(createElement(Widget, props)).replace(/<[^>]*>/g, ' ').replace(/\s+/g, ' ');
    const unit = currency === 'Political' ? 'PP' : 'CP';
    const table = state.table!;
    assert.deepEqual(table.columns.map(column => column.key), ['date', 'item', 'category', 'amount']);
    assert.deepEqual(table.sort, { column: 'date', direction: 'descending' });
    // 40 daily costs fold into two month rows; the raid stays its own row; no gap rows.
    assert.equal(table.rows.length, 3);
    const folded = table.rows.filter(row => row.month !== null);
    assert.deepEqual(folded.map(row => [row.days, row.amount]), [[31, 3100031n], [9, 900009n]]);
    const raid = table.rows.find(row => row.category === 'raids')!;
    assert.equal(raid.label, 'Raids');
    const amount = table.columns.find(column => column.key === 'amount')!;
    assert.equal(renderToStaticMarkup(createElement('span', null, amount.render!(raid))), '<span>−20</span>', 'whole amounts by default');
    const item = renderToStaticMarkup(createElement('span', null, table.columns.find(column => column.key === 'item')!.render!(folded[0])));
    assert.match(item, /Daily costs.*31 days/);
    const tip = table.tooltip!(folded[0]);
    assert.equal(tip.title, 'Daily costs');
    assert.ok(tip.rows.some(row => row.label === 'Spent' && row.value === `31 ${unit}`));
    assert.doesNotMatch(JSON.stringify(table.tooltip!(raid)), /Not identified/);
    assert.match(markup, new RegExp(`Spent 60 ${unit}`));
    assert.match(markup, new RegExp(`Gained ${currency === 'Political' ? '100' : '33'} ${unit}`));
    assert.match(markup, /Spending wasn’t recorded for 3 hours \(/, 'unrecorded hours are one note, not rows');
    assert.match(markup, /All categories/);
    assert.doesNotMatch(markup, /\bNone\b|held|Not known|Not identified|Total spending unavailable/);

    state.tags = ['native actor', 'native target'];
    renderToStaticMarkup(createElement(Widget, props));
    assert.deepEqual(state.table!.columns.map(column => column.key), ['date', 'country', 'item', 'category', 'amount'], 'several countries add a country column');
    state.tags = ['native actor'];
    props.widget.options = { decimals: 5, order: 'cost' };
    renderToStaticMarkup(createElement(Widget, props));
    assert.deepEqual(state.table!.sort, { column: 'amount', direction: 'descending' }, 'a saved largest-first log keeps its order');
    const precise = state.table!.rows.find(row => row.month !== null && row.days === 9)!;
    assert.equal(renderToStaticMarkup(createElement('span', null, state.table!.columns.find(column => column.key === 'amount')!.render!(precise))), '<span>−9.00009</span>', 'the Decimals setting raises the digits');
  } finally { await vite.close(); delete host.__spendingTable; }
});
