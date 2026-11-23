import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { DashboardMapState } from '../../dashboard/client';

test('map load errors hide diagnostics and retain Retry; an hour not available yet says so; the displayed map keeps its mode and date; a hidden map is not loading; a frame of another hour stays labelled, never covered by a loading placeholder', async () => {
  const host = globalThis as typeof globalThis & { __mapReview?: DashboardMapState; __mapsHidden?: boolean };
  host.__mapReview = { id: 'map', clockId: 'page-1', sourceId: 's', revision: 'r', branchId: 'b', hour: 105, fraction: 0,
    sequence: 0, requestedSequence: 0, loading: false, error: 'Synthetic native failure' };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'map-state', enforce: 'pre',
      transform(_source, id) {
        const path = id.replaceAll('\\', '/');
        if (path.endsWith('/dashboard/geometry.ts')) return 'export const useMapState = () => globalThis.__mapReview; export const useMapsHidden = () => !!globalThis.__mapsHidden;';
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
    host.__mapReview.loading = true;
    host.__mapReview.hour = null;
    host.__mapsHidden = true;
    const hidden = render();
    assert.match(hidden, /Map hidden for layout controls/);
    assert.doesNotMatch(hidden, /spinner|Preparing this map/);
    host.__mapsHidden = false;
    (props.runtime.clock as { getSnapshot: () => unknown }).getSnapshot = () => ({ position: 106, hour: 106, playing: false, sequence: 3, seekSequence: 1, fraction: 0, hoursPerSecond: 4 });
    host.__mapReview = { ...host.__mapReview, hour: 105, sequence: 2, requestedSequence: 3, loading: true };
    const stale = render();
    assert.doesNotMatch(stale, /spinner|Preparing this map/);
    assert.match(stale, /Updating…/);
    assert.match(stale, /data-map-current="false"/);
    host.__mapReview = { ...host.__mapReview, hour: null, branchId: null };
    const first = render();
    assert.match(first, /spinner/);
    assert.match(first, /Preparing this map…/);
  } finally { await vite.close(); delete host.__mapReview; delete host.__mapsHidden; }
});
