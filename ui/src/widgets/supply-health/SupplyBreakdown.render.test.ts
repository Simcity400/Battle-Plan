import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { ComponentProps } from 'react';
import type SupplyBreakdown from './SupplyBreakdown';
import type { SupplyHealthReply } from '../../generated/dashboard';

test('pinned supply omits its country heading only when the frame names it and distinguishes zero from missing', async () => {
  const data: SupplyHealthReply = { branchId: '1', available: true, reason: null, missingInputs: [], warnings: [],
    series: [{ tag: 'X', identities: [{ firstHour: 10, lastHour: 20, slot: 1, countryId: 2 }],
      runs: [{ firstHour: 10, lastHour: 10, startsSegment: true, inSupply: 1, strained: 0, noSupply: 0 }] }] };
  const state = { data };
  const host = globalThis as typeof globalThis & { __supplyPresentation?: typeof state }; host.__supplyPresentation = state;
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': 'export const useProduct = () => ({ data: globalThis.__supplyPresentation.data }); export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
    '/dashboard/playback.ts': 'export const usePlaybackSpeed = () => 0;',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: "Country " + tag }) });',
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 1000; export const useChartTokens = () => ({ error: "bad", warning: "warn", positive: "good" });',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ menu: null });',
    '/charts/drill/DrillParts.tsx': 'export default props => props.totalRow.value;',
    '/charts/StackedShareChart.tsx': 'export default () => null;',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'supply-breakdown-presentation', enforce: 'pre', transform(code, id) {
      const path = id.replaceAll('\\', '/');
      const replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix))?.[1];
      if (replacement) return replacement;
      if (path.endsWith('/supply-health/SupplyBreakdown.tsx')) return code.replace("import { createPortal } from 'react-dom';", 'const createPortal = () => null;');
    } }] });
  try {
    const { default: Component } = await vite.ssrLoadModule('/ui/src/widgets/supply-health/SupplyBreakdown.tsx');
    const track = { id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: { id: '1', firstHour: 10, lastHour: 20 } };
    const props = { runtime: { tracks: [track], client: {}, clock: {} }, track, active: true, options: {}, countrySlot: {} as HTMLElement,
      headerSelection: { tag: 'X', hour: 10 }, extent: [10, 20], playhead: null, tag: 'X', hour: 10, originHour: 10,
      following: false, pinned: true, onPick() {}, onFollow() {}, onWarnings() {} } as unknown as ComponentProps<typeof SupplyBreakdown>;
    const render = () => renderToStaticMarkup(createElement(Component, props));
    assert.doesNotMatch(render(), /supply-health-country|drill-empty/);
    props.headerSelection = null;
    assert.match(render(), /supply-health-country[^>]*>Country X/);
    props.headerSelection = { tag: 'X', hour: 10 }; props.countrySlot = null;
    assert.match(render(), /supply-health-country[^>]*>Country X/);
    state.data = { ...data, series: [{ ...data.series[0], runs: [{ ...data.series[0].runs[0], inSupply: 0 }] }] };
    assert.match(render(), /No divisions\./);
    props.hour = 11;
    assert.match(render(), /Supply was not recorded at this time\./);
    assert.doesNotMatch(render(), /No divisions/);
    state.data = { ...data, available: false, reason: 'DivisionLandInputs is not recorded' };
    assert.match(render(), /Supply data is not in this recording\./);
    assert.doesNotMatch(render(), /DivisionLandInputs/);
  } finally { delete host.__supplyPresentation; await vite.close(); }
});
