import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';

// The race owner verifies dates/native binding; these widgets must supply its header slot.
test('Factories and Equipment produced register a breakdown slot and forward it to the shared body', async () => {
  const globals = globalThis as typeof globalThis & { __economyPartsProps?: { countrySlot?: HTMLElement | null } };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'economy-parts-slot', enforce: 'pre',
      transform(source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/charts/race/CountryPartsDrill.tsx')) return 'export default props => { globalThis.__economyPartsProps = props; return null; };';
        if (path.endsWith('/dashboard/CountryChip.tsx')) return source.slice(source.indexOf('export function PartsCountryChip'), source.indexOf('/** The widget header'))
          + "\nfunction CountryChip() { return 'Generic country'; }";
        if (path.endsWith('/dashboard/products.ts')) return 'export const useProduct = () => ({ retry() {} }); export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });';
        if (path.endsWith('/dashboard/useCountryGroups.ts')) return 'export const useCountryGroups = () => ({ retry() {} });';
        if (path.endsWith('/dashboard/playback.ts')) return 'export const usePlaybackHour = () => 0;';
        if (path.endsWith('/charts/useChartTokens.ts')) return "export const useChartTokens = () => ({ categories: [], muted: '#888' }); export const useViewportHeight = () => 800;";
      },
    }] });
  try {
    for (const kind of ['factories', 'equipment-produced']) {
      const renderer = (await vite.ssrLoadModule(`/ui/src/widgets/${kind}/index.ts`)).default;
      const props = { widget: { id: kind, kind, trackIds: [], countries: { kind: 'tags', tags: ['X'] }, time: { mode: 'campaign' },
        options: { show: 'breakdown', drillOriginHour: 105 } },
        runtime: { tracks: [], client: {}, clock: { getHour: () => 112 } }, active: true,
        countrySlot: {} as HTMLElement, onDiagnostics() {} } as unknown as WidgetProps;
      assert.match(renderToStaticMarkup(createElement(renderer.countryChip, { ...props, onSlot() {} })), /dash-widget-country-slot/);
      renderToStaticMarkup(createElement(renderer.render, props));
      assert.equal(globals.__economyPartsProps!.countrySlot, props.countrySlot);
      props.widget.options = { show: 'main' };
      assert.equal(renderToStaticMarkup(createElement(renderer.countryChip, { ...props, onSlot() {} })), 'Generic country');
    }
  } finally { delete globals.__economyPartsProps; await vite.close(); }
});
