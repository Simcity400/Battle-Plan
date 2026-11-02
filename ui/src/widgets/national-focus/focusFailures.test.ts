import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { Artwork } from '../../generated/dashboard';

test('current and completed focuses retain successful titles and show every partial artwork failure', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { FocusCompletion, CurrentFocus } = await vite.ssrLoadModule('/ui/src/widgets/national-focus/NationalFocusWidget.tsx') as typeof import('./NationalFocusWidget');
    const { RefusedError } = await vite.ssrLoadModule('/ui/src/dashboard/queryError.ts');
    const art: Artwork = { token: 'synthetic', name: 'Recorded focus title', description: null, icon: null, baseDurationDays: null };
    const refused = new RefusedError('Synthetic focus descriptions were not recorded'), failure = new Error('Synthetic focus image decoder failed');
    for (const errors of [[failure], [refused], [failure, refused]]) {
      const html = [renderToStaticMarkup(createElement(FocusCompletion, {
        entry: { ordinal: 0, token: art.token, completedByHour: null }, art, failures: errors, open: false, onToggle() {},
      })), renderToStaticMarkup(createElement(CurrentFocus, {
        order: { current: art.token, continuous: null, paused: false, completed: [] }, art, state: null, live: false, failures: errors,
      }))];
      for (const markup of html) {
        assert.match(markup, />Recorded focus title</);
        for (const error of errors) {
          assert.ok(markup.includes(`data-detail="${error.message}"`));
          assert.ok(markup.includes(error instanceof RefusedError ? `>${error.message}<` : '>Couldn’t load focus details.<'));
        }
        assert.doesNotMatch(markup, /Loading name|Focus name unavailable/);
      }
    }
    const missing = renderToStaticMarkup(createElement(FocusCompletion, {
      entry: { ordinal: 0, token: art.token, completedByHour: null }, art: { ...art, name: null }, open: false, onToggle() {},
    }));
    assert.match(missing, />Focus name unavailable</);
    assert.doesNotMatch(missing, /Loading name/);
  } finally { await vite.close(); }
});
