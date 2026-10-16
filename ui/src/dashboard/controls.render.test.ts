import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement, type ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

// Before any recording binds a reference track, both the coverage owner key and
// the unset coverage state are undefined; the strip must still draw.
test('the playback strip draws without a reference track', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { PlaybackStrip } = await vite.ssrLoadModule('/ui/src/dashboard/controls.tsx') as { PlaybackStrip: ComponentType<Record<string, unknown>> };
    const snapshot = { hour: 0, playing: false, hoursPerSecond: 1, bounds: null };
    const clock = { getSnapshot: () => snapshot, subscribe: () => () => {}, getHour: () => 0 };
    const markup = renderToStaticMarkup(createElement(PlaybackStrip, { clock, tracks: [], alignment: 'date', defaultRoleId: null, client: {} }));
    assert.ok(markup.length > 0);
  } finally {
    await vite.close();
  }
});
