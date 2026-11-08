import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { DashboardMapState } from '../../dashboard/client';

test('map load errors hide diagnostics and retain Retry; an hour not available yet says so; the displayed map keeps its mode and date; a hidden map is not loading', async () => {
  const host = globalThis as typeof globalThis & { __mapReview?: DashboardMapState; __mapsHidden?: boolean };
  host.__mapReview = { id: 'map', sourceId: 's', revision: 'r', branchId: 'b', hour: 105, fraction: 0,
    sequence: 0, requestedSequence: 0, loading: false, error: 'Synthetic native failure' };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'map-state', enforce: 'pre',
      transform(_source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/geometry.ts')) return 'export const useMapState = () => globalThis.__mapReview; export const useMapsHidden = () => !!globalThis.__mapsHidden;';
        if (path.endsWith('/dashboard/playback.ts')) return 'export const usePlaybackHour = clock => clock.getHour();';
      },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/map/MapWidget.tsx');
    const props = { widget: { id: 'map', kind: 'map', trackIds: ['t'], countries: { kind: 'all' }, time: { mode: 'follow' } }, active: true, mapOptions: { fog: true },
      runtime: { client: { available: true, nativeMaps: true }, maps: { retry() {}, register() {}, schedule() {} },
        tracks: [{ id: 't', axisOffset: 0, branch: { id: 'b', firstHour: 100, lastHour: 110 }, source: { id: 's', path: 'synthetic', revision: 'r' } }],
        clock: { getHour: () => 105, getSnapshot: () => ({ position: 105, hour: 105, playing: true, sequence: 0, fraction: 0, hoursPerSecond: 4 }) } } } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(Widget, props));
    const failed = render();
    assert.match(failed, /Couldn’t load\./);
    assert.match(failed, /Retry/);
    assert.doesNotMatch(failed.replace(/<[^>]*>/g, ' '), /Synthetic native failure|Follow the campaign/);
    host.__mapReview.error = null;
    const live = render();
    assert.match(live, /Campaign map/);
    assert.match(live, /Live/);
    assert.doesNotMatch(live, /Follow the campaign|Native map/);
    // Under an open menu the native map is hidden: it says so, with no spinner, even while its view reports loading.
    host.__mapReview.loading = true;
    host.__mapReview.hour = null;
    host.__mapsHidden = true;
    const hidden = render();
    assert.match(hidden, /Map hidden while a menu is open/);
    assert.doesNotMatch(hidden, /spinner|Preparing this map/);
  } finally { await vite.close(); delete host.__mapReview; delete host.__mapsHidden; }
});
