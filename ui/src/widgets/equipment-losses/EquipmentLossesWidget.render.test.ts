import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { LossesReply } from '../../generated/dashboard';
import type { SunburstProps } from '../../charts/sunburst/Sunburst';
import { groupTree, sliceTooltip } from '../../charts/sunburst/layout.ts';

const reply: LossesReply = { branchId: '1', tag: 'AAA', amountScale: 100000, landSource: 'destruction',
  landLowerBound: null, destructionCountedFrom: 100, destructionCoverage: [{ firstHour: 100, lastHour: 109, reason: null }],
  retainedMonths: null, warnings: [], coverage: [], designs: [], causes: [{
    source: { kind: 'destruction', reason: 'land_combat' }, domain: 'land', loss: true, group: null,
    name: { key: null, template: null, text: 'Combat' }, unknown: null, unknownSpans: [],
    leaves: [{ design: null, points: [{ hour: 100, totalRaw: '200000' }], lowerBoundFrom: null }],
  }] };

test('the loss overview combines data availability with amount completeness and hides read metadata', async () => {
  const host = globalThis as typeof globalThis & { __lossReply?: LossesReply; __lossSunburst?: SunburstProps };
  host.__lossReply = reply;
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': 'export const useProduct = () => ({ data: globalThis.__lossReply, retry() {} });',
    '/dashboard/useCountryGroups.ts': `export const useCountryGroups = () => ({ data: { countryIdentities: [
      { tag: 'AAA', identities: [{ firstHour: 100, lastHour: 109, slot: 7, countryId: 77 }] }] } });`,
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour(); export const usePlaybackSpeed = () => 24;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({ categories: ['#123'], positive: '#080', warning: '#880',
      error: '#800', surface: '#000', text: '#fff' }); export const useViewportHeight = () => 1000;`,
    '/charts/sunburst/Sunburst.tsx': `export default props => { globalThis.__lossSunburst = props; return props.legendExtra; };`,
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'loss-overview-fixture', enforce: 'pre',
      transform: (_source, id) => { const key = Object.keys(replacements).find(key => id.endsWith(key)); return key ? replacements[key] : undefined; },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/equipment-losses/EquipmentLossesWidget.tsx');
    const props = { widget: { id: 'losses', kind: 'equipment-losses', trackIds: ['main'], countries: { kind: 'tags', tags: ['AAA'] },
      time: { mode: 'campaign' }, options: { legend: false } }, runtime: { client: {}, clock: { getHour: () => 109 }, tracks: [{
        id: 'main', axisOffset: 0, branch: { id: '1', firstHour: 100, lastHour: 109 },
        source: { id: 'synthetic', revision: '1', metadata: { campaignKey: 'synthetic' } },
      }] }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = (data: LossesReply) => {
      host.__lossReply = data;
      return renderToStaticMarkup(createElement(Widget, props));
    };
    assert.match(render(reply), />Complete</);
    assert.match(render(reply), /View all losses/);
    const chart = host.__lossSunburst!;
    const tree = groupTree(chart.items, chart.root, chart.levels, chart.authoritativeTotal);
    const tip = sliceTooltip(tree.children[0], tree, chart.text)[0];
    assert.ok(!tip.rows.some(row => /read|Game text|Cause label|Last recorded change/i.test(row.label)));
    const gap = { ...reply, destructionCoverage: [{ firstHour: 100, lastHour: 109, reason: 'Native drain unavailable' }] };
    assert.match(render(gap), />Incomplete</, 'missing acquisition intervals cannot be labelled Complete');
    assert.doesNotMatch(render(gap), /Native drain unavailable/);
    const missing = { ...reply, causes: [{ ...reply.causes[0], leaves: [], unknown: 'Raw acquisition refusal' }] };
    assert.match(render(missing), />Unavailable</);
    assert.doesNotMatch(render(missing), /Raw acquisition refusal/);
  } finally {
    delete host.__lossReply; delete host.__lossSunburst;
    await vite.close();
  }
});
