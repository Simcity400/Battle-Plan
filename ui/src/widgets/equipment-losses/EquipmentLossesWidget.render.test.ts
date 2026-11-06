import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { LossesReply } from '../../generated/dashboard';
import type { SunburstProps } from '../../charts/sunburst/Sunburst';
import type { TimeChartProps } from '../../charts/TimeChart';
import type { PlotFrame } from '../../charts/frame';
import type { ChartTokens } from '../../charts/tokens';
import { groupTree, sliceTooltip } from '../../charts/sunburst/layout.ts';

const reply: LossesReply = { branchId: '1', tag: 'AAA', amountScale: 100000, landSource: 'destruction',
  landLowerBound: null, destructionCountedFrom: 100, destructionCoverage: [{ firstHour: 100, lastHour: 109, reason: null }],
  retainedMonths: null, warnings: [], coverage: [], designs: [], causes: [{
    source: { kind: 'destruction', reason: 'land_combat' }, domain: 'land', loss: true, group: null,
    name: { key: null, template: null, text: 'Combat' }, unknown: null, unknownSpans: [],
    leaves: [{ design: null, points: [{ hour: 100, totalRaw: '200000' }], lowerBoundFrom: null }],
  }] };

test('the loss overview combines data availability with amount completeness and hides read metadata', async () => {
  const host = globalThis as typeof globalThis & { __lossReply?: LossesReply; __lossSunburst?: SunburstProps; __lossChart?: TimeChartProps };
  host.__lossReply = reply;
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': `export const useProduct = () => ({ data: globalThis.__lossReply, retry() {} });
      export const loadProduct = (_client, _request, _signal, _validate, _source, _restart, publish) => {
        if (globalThis.__lossReply !== undefined) publish?.(globalThis.__lossReply); return new Promise(() => {}); };
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    // Effects never run here: the view read lands within the render, as the reply it is given.
    '/dashboard/viewRead.ts': `export const useViewRead = (lane, view, _viewKey, active, read) => { let data;
      if (active && lane != null && view != null) void read(view, new AbortController().signal, value => { data = value; });
      return { data, retry() {} }; };`,
    '/dashboard/useCountryGroups.ts': `export const useCountryGroups = () => ({ data: { countryIdentities: [
      { tag: 'AAA', identities: [{ firstHour: 100, lastHour: 109, slot: 7, countryId: 77 }] }] } });`,
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour(); export const usePlaybackMotion = () => 24;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({ categories: ['#123'], positive: '#080', warning: '#880',
      error: '#800', surface: '#000', text: '#fff' }); export const useViewportHeight = () => 1000;`,
    '/charts/sunburst/Sunburst.tsx': `export default props => { globalThis.__lossSunburst = props; return props.legendExtra; };`,
    '/charts/TimeChart.tsx': 'export default props => { globalThis.__lossChart = props; return null; };',
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
    const render = (data?: LossesReply) => {
      host.__lossReply = data && { ...data, preparedReads: [100, 109], preparedHeadHour: 109, preparedDetailHour: 109,
        preparedFrom: 100, preparedTo: 109, preparedWidth: 1, restarts: [] } as LossesReply | undefined;
      return renderToStaticMarkup(createElement(Widget, props));
    };
    const pending = render();
    assert.equal((pending.match(/This range is still being prepared\./g) ?? []).length, 1);
    assert.doesNotMatch(pending, /No equipment losses|View all losses|>Complete</);
    assert.match(render(reply), />Complete</);
    assert.doesNotMatch(render(reply), /still being prepared/);
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
    props.widget.options = { show: 'loss-details' };
    render(reply);
    const history = host.__lossChart!;
    assert.deepEqual(history.extent, [100, 110], 'the last recorded tick retains its full chart width');
    let fills = 0;
    const context = { beginPath() {}, moveTo() {}, lineTo() {}, closePath() {}, stroke() {}, fill() { fills++; }, fillRect() { fills++; } } as unknown as CanvasRenderingContext2D;
    const frame = { x: [100, 110], plotWidth: 1, toX: (hour: number) => hour, toY: (value: number) => value } as unknown as PlotFrame;
    for (const layer of history.layers) layer.draw(context, frame, { surface: '#000' } as ChartTokens, { playhead: null });
    assert.ok(fills > 0, 'prepared data through 109 draws against the chart extent ending at 110');
    fills = 0;
    for (const layer of history.layers) layer.draw(context, { ...frame, x: [100, 108] }, { surface: '#000' } as ChartTokens, { playhead: null });
    assert.equal(fills, 0, 'a landed summary never paints past the hours a zoomed view shows');
    assert.equal(history.tooltip?.(105, frame, []), null, 'sampled chart geometry does not establish exact hover values');
  } finally {
    delete host.__lossReply; delete host.__lossSunburst; delete host.__lossChart;
    await vite.close();
  }
});
