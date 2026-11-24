import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

test('named ArtFallback renders initials, accessible label, and tooltip while unnamed decorative fallback stays hidden', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const mod = await vite.ssrLoadModule('/ui/src/components/ArtFallback.tsx') as {
      default: (props: Record<string, unknown>) => ReturnType<typeof createElement>;
      artInitials: (name: string) => string;
      artFallbackText: (input: { name?: string | null; failure?: Error; reason?: string | null }) => string | undefined;
    };
    assert.equal(mod.artInitials('Synthetic building'), 'SB');
    assert.equal(mod.artInitials('Infantry Equipment I'), 'IEI');
    assert.equal(mod.artInitials('Chromium'), 'CH');
    assert.equal(mod.artInitials('e\u0301clair'), 'E\u0301C');
    assert.equal(mod.artInitials('   '), '');
    assert.equal(mod.artFallbackText({ name: 'Mod spirit' }), 'Mod spirit — Art not in the current installation.');
    assert.equal(mod.artFallbackText({ name: 'Mod spirit', reason: 'Missing mod icon' }), 'Mod spirit: Missing mod icon');
    assert.equal(mod.artFallbackText({}), undefined);

    const named = renderToStaticMarkup(createElement(mod.default, { icon: 'factory', name: 'Synthetic building' }));
    assert.match(named, /^<span class="art-fallback" data-has-tag=""/);
    assert.match(named, /role="img"/);
    assert.match(named, /aria-label="Synthetic building — Art not in the current installation\."/);
    assert.match(named, /data-tooltip="Synthetic building — Art not in the current installation\."/);
    assert.match(named, /<span class="art-fallback-tag" aria-hidden="true">SB<\/span>/);

    const decorative = renderToStaticMarkup(createElement(mod.default, { icon: 'flask' }));
    assert.match(decorative, /aria-hidden="true"/);
    assert.doesNotMatch(decorative, /role="img"|data-has-tag|art-fallback-tag/);
  } finally { await vite.close(); }
});
