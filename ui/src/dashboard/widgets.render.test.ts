import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { BoundTrack, WidgetSpec } from './model';
import type { WidgetRuntime } from './definitions';

const replacements: Record<string, string> = {
  '/dashboard/definitions.ts': `export const widgetUnavailable = () => null;
    export const widgetDefinition = kind => ({ data: true,
      playsBackDated: kind === 'synthetic-dated',
      render: () => 'RENDERED' });`,
};

test('a default-campaign widget is gated by the campaign it renders, not its saved tracks', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'widget-scope', enforce: 'pre',
      transform(_source, id) { return Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1]; } }] });
  try {
    const { WidgetBody } = await vite.ssrLoadModule('/ui/src/dashboard/widgets.tsx') as typeof import('./widgets');
    const tracks = (['full', 'map'] as const).map(mode => ({ id: `track-${mode}`, roleId: mode, label: mode,
      branch: { id: 'branch', firstHour: 0, lastHour: 100 },
      source: { path: `${mode}.battleplan`, metadata: { countries: [], branches: [], captureMode: mode } } })) as unknown as BoundTrack[];
    const render = (defaultRoleId: string, saved: string, playing = false, kind = 'synthetic') => renderToStaticMarkup(createElement(WidgetBody, {
      widget: { id: 'scoped', kind, configVersion: 1, title: 'Synthetic', campaign: 'default', trackIds: [saved],
        time: { mode: 'follow' }, countries: { kind: 'tags', tags: [] } } as WidgetSpec,
      runtime: { tracks, campaignTracks: tracks, defaultRoleId, maps: {}, client: {}, clock: { getHour: () => 0, getSnapshot: () => ({ playing }) } } as unknown as WidgetRuntime }));
    assert.match(render('full', 'track-map'), /RENDERED/, 'Map to Full: the Full default renders');
    const gated = render('map', 'track-full');
    assert.match(gated, /This is a Map recording, which holds the campaign map and battle windows only\./, 'Full to Map: the Map default is gated');
    assert.match(gated, /data-state="unavailable"/);
    assert.doesNotMatch(gated, /is-error|role="alert"/);
    const placeholder = render('full', 'track-full', true, 'synthetic');
    assert.match(placeholder, /Synthetic isn’t playing back yet\./);
    assert.match(placeholder, /data-state="placeholder"/);
    assert.match(placeholder, /data-playback-placeholder/);
    assert.doesNotMatch(placeholder, /is-error|role="alert"/);
    assert.match(render('full', 'track-full', true, 'synthetic-dated'), /RENDERED/, 'a playsBackDated widget renders during playback');
    assert.match(render('full', 'track-full', true, 'campaign-clock'), /RENDERED/, 'a family view plays back live with no placeholder and no playsBackDated flag');
  } finally { await vite.close(); }
});
