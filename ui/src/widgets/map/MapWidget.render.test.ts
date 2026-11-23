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
    assert.match(failed, /Couldn’t load the campaign map\./);
    assert.match(failed, /Retry/);
    assert.doesNotMatch(failed.replace(/<[^>]*>/g, ' '), /Synthetic native failure|Follow the campaign/);
    host.__mapReview.error = null;
    const live = render();
    assert.match(live, /Campaign map/);
    assert.match(live, /data-map-current="true"/);
    assert.doesNotMatch(live, /data-held=/);
    assert.doesNotMatch(live, /Follow the campaign|Native map|dash-map-footer/);
    host.__mapReview.loading = true;
    host.__mapReview.hour = null;
    host.__mapsHidden = true;
    const hidden = render();
    assert.match(hidden, /Map hidden while a menu is open/);
    assert.doesNotMatch(hidden, /spinner|Preparing this map/);
    host.__mapsHidden = false;
    (props.runtime.clock as { getSnapshot: () => unknown }).getSnapshot = () => ({ position: 106, hour: 106, playing: false, sequence: 3, seekSequence: 1, fraction: 0, hoursPerSecond: 4 });
    host.__mapReview = { ...host.__mapReview, hour: 105, sequence: 2, requestedSequence: 3, loading: true };
    const stale = render();
    assert.doesNotMatch(stale, /spinner|Preparing this map/);
    assert.match(stale, /data-map-current="false"/);
    assert.match(stale, /data-held="stale"/);
    (props.runtime.clock as { getSnapshot: () => unknown }).getSnapshot = () => ({ position: 106, hour: 106, playing: true, sequence: 3, seekSequence: 1, fraction: 0, hoursPerSecond: 4 });
    const behind = render();
    assert.match(behind, /data-held="behind"/);
    host.__mapReview = { ...host.__mapReview, hour: 105, sequence: 3, requestedSequence: 3, loading: false, behind: 12 };
    const reportedBehind = render();
    assert.match(reportedBehind, /data-held="behind"/, 'reported native behind > 0 marks the map held even when loading is false');
    host.__mapReview = { ...host.__mapReview, behind: 0, hour: null, branchId: null };
    const first = render();
    assert.match(first, /spinner/);
    assert.match(first, /Preparing this map…/);

    const { MapSettingsSection } = await vite.ssrLoadModule('/ui/src/widgets/map/MapSettingsSection.tsx');
    const branched = {
      id: 'b2', firstHour: 43800000, lastHour: 43800240,
      fork: { hour: 43800120 },
      joins: [
        { segmentId: 'seg-1', hour: 43800048, evidence: 'unconfirmed', parentId: 'seg-0', choices: [] },
        { segmentId: 'seg-2', hour: 43800096, evidence: 'unrecorded', parentId: 'seg-1', choices: [] },
      ],
    };
    const settingsHtml = renderToStaticMarkup(createElement(MapSettingsSection, {
      widget: props.widget,
      reference: {
        id: 't',
        branch: branched,
        source: { id: 's', revision: 'r', metadata: { branches: [{ id: 'b1', firstHour: 43800000, lastHour: 43800120, fork: null, joins: [] }, branched] } },
      },
      runtime: { ...props.runtime, onMapBranch() {} },
      mapOptions: { fog: true, country: null },
      onMapOptions() {},
    }));
    assert.match(settingsHtml, /was left behind when the game was loaded from an earlier save\./);
    assert.match(settingsHtml, /the recording can’t tell which save this play was loaded from/);
    assert.match(settingsHtml, /play the recorder didn’t see/);
  } finally { await vite.close(); delete host.__mapReview; delete host.__mapsHidden; }
});
