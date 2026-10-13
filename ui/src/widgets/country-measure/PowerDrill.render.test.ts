import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { tokensFromValues } from '../../charts/tokens.ts';
import { powerBalanceIndex } from '../../dashboard/powerBalance.ts';
import type { PoliticsLedgerReply } from '../../generated/dashboard';
import type PowerDrill from './PowerDrill';

test('power drills show missing parts and one qualified total, retaining adjustment details and reservations', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'power-drill-chart', enforce: 'pre',
      transform: (_source, id) => id.replaceAll('\\', '/').endsWith('/charts/TimeChart.tsx') ? 'export default () => null;' : undefined,
    }] });
  try {
    const { default: Drill } = await vite.ssrLoadModule('/ui/src/widgets/country-measure/PowerDrill.tsx') as { default: typeof PowerDrill };
    const reply: PoliticsLedgerReply = { branchId: '1', tag: 'mod country', decimals: 5, warnings: [], spending: [], spendItems: [],
      spendCategories: [], spendingCoverage: [], ideaTooltips: null, commandHolds: [],
      daily: [{ country: 1, instance: 1, epoch: '1', dayHour: 24, readHour: 26, ppGrossRaw: null, ppCostRaw: '100000',
        ppIncomeRaw: '500000', ppRealizedRaw: '0', ppSequence: '1', cpIncomeRaw: '-200000', cpRealizedRaw: '0', cpSequence: '2' }],
      balanceChanges: [{ hour: 24, readHour: 26, currency: 'Political', action: 'Synthetic action',
        amountRaw: '900000', realizedDeltaRaw: '400000', covered: true, reason: null }] };
    const props: Parameters<typeof PowerDrill>[0] = { index: powerBalanceIndex(reply, 'Political'), pickOwner: {}, presentationReady: true,
      mode: 'income', hour: 26, visibleThrough: 30, extent: [24, 30], within: null, countries: [{ firstHour: 24, lastHour: 30 }],
      playback: { playhead: null, mode: 'cursor', afterPlayhead: false, windowHours: 24 }, branchStart: 24,
      hoursPerSecond: 0, active: true, tokens: tokensFromValues(() => '#777777', 'sans-serif'), onContextMenu() {} };
    const text = () => renderToStaticMarkup(createElement(Drill, props)).replace(/<[^>]*>/g, ' ');
    const income = text();
    assert.match(income, /Daily gain\s+Not recorded/);
    assert.match(income, /Daily costs\s+−1\.00/);
    assert.match(income, /Daily change\s+\+5\.00 · Income breakdown incomplete\./);
    assert.equal((income.match(/Daily change/g) ?? []).length, 1);
    assert.doesNotMatch(income, /Other|Net daily income|Income sources were not recorded|Shares use|%/, 'no catch-all label and no shares of signed parts');
    assert.doesNotMatch(income, /Synthetic action|Hour 24/, 'recorded changes are not repeated as details');
    assert.match(income, /Lost at the cap\s+−5\.00/);
    props.index = powerBalanceIndex({ ...reply, commandHolds: [{ readHour: 24, throughHour: 30, totalRaw: '300000', reason: null,
      holders: [{ id: 'hold', kind: 'advisor', name: 'Synthetic reservation', amountRaw: '300000' }] }] }, 'Command');
    const command = text();
    assert.match(command, /Daily deficit\s+−2\.00/);
    assert.doesNotMatch(command, /Synthetic reservation|Total held|Reserv/, 'a reserve that did not move that day is not shown');
    props.index = powerBalanceIndex({ ...reply, daily: [] }, 'Political');
    assert.match(text(), /Daily change\s+Not recorded · Income not recorded\./);
  } finally { await vite.close(); }
});
