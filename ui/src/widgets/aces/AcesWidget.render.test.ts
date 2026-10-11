import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { AirAcesManifestReply, AirAcesWindowReply } from '../../generated/dashboard';

test('ace portrait slots use a helmet and refresh failures retain Retry without artwork reasons', async () => {
  const interval = { firstHour: 100, lastHour: 147 };
  const detail: AirAcesWindowReply = { branchId: '1', window: 0, ...interval, warnings: [], texts: ['Synthetic ace'],
    aces: [{ id: 'ace', firstSeen: 100, runs: [{ ...interval, tag: 'mod country', name: 0, callsign: null,
      wing: null, wingName: null, aircraft: [], alive: true, death: null }] }] };
  const manifest: AirAcesManifestReply = { branchId: '1', ...interval, available: true, unavailable: null,
    observed: [interval], readHours: [100], warnings: [], windows: [interval], aces: [{ id: 'ace', firstSeen: 100 }], variants: {} };
  const state = { manifest, detail, pending: false, failure: false, portrait: null as string | null, retries: 0 };
  const globals = globalThis as typeof globalThis & { __acePresentation?: typeof state };
  globals.__acePresentation = state;
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': 'export const useProduct = () => ({ data: globalThis.__acePresentation.manifest, retry() {} });',
    '/dashboard/productWindows.ts': `export const useProductWindows = () => { const state = globalThis.__acePresentation; return {
      data: state.pending ? new Map() : new Map([[0, state.detail]]), errors: state.failure ? new Map([[0, 'Synthetic native refusal']]) : new Map(),
      retry() { state.retries++; } }; }; export const useWindowDisplay = () => ({data: globalThis.__acePresentation.detail, hour: 100});`,
    '/charts/table/useTableHour.ts': 'export const useTableHour = () => 100;',
    '/dashboard/playback.ts': 'export const usePlaybackSpeed = () => 0;',
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({retry() {}});',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({labelAt: () => tag => ({name: tag})});',
    '/dashboard/countryNamesAt.ts': `export const nameKey = (hour, tag) => JSON.stringify([hour, tag]);
      export const useCountryFactsAt = () => new Map([[nameKey(100, 'ace'), {image: globalThis.__acePresentation.portrait, missing: 'Synthetic portrait refusal'}]]);`,
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 1000;',
    '/charts/table/Table.tsx': `import {createElement} from 'react'; export default props => createElement('div', {},
      props.rows.map(row => createElement('div', {key: row.key}, props.columns.map(column => createElement('span', {key: column.key}, column.render?.(row))))));`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'ace-presentation', enforce: 'pre',
      transform: (_source, id) => Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1],
    }] });
  try {
    const Widget = (await vite.ssrLoadModule('/ui/src/widgets/aces/AcesWidget.tsx')).default;
    const props = { widget: { id: 'aces', kind: 'aces', trackIds: ['main'], countries: { kind: 'all' }, time: { mode: 'campaign' } },
      runtime: { client: {}, tracks: [{ id: 'main', source: { id: 'source', revision: '1' }, branch: { id: '1', ...interval } }] },
      active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(Widget, props));
    const missing = render();
    assert.match(missing, /art-fallback aces-portrait/);
    assert.doesNotMatch(missing, /Callsign not recorded|Hall of fame|Portrait not recorded|Synthetic portrait refusal|role="img"[^>]*Portrait/);
    state.portrait = 'data:image/png;base64,synthetic';
    const present = render();
    assert.match(present, /class="aces-portrait" src="data:image\/png;base64,synthetic"/);
    assert.doesNotMatch(present, /art-fallback aces-portrait/);
    state.pending = true; state.failure = true;
    const failed = render();
    assert.match(failed, /Couldn’t load aces\./); assert.match(failed, /Retry/);
    assert.doesNotMatch(failed, /Synthetic native refusal/);
  } finally { await vite.close(); delete globals.__acePresentation; }
});
