import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

test('current and completed research reserve a flask fallback without artwork reasons', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'research-art-fixture', enforce: 'pre',
      transform(source, id) {
        if (id.replaceAll('\\', '/').endsWith('/research/ResearchWidget.tsx')) return `${source}\nexport { ResearchArt, ResearchCard };`;
      },
    }] });
  try {
    const { ResearchArt, ResearchCard } = await vite.ssrLoadModule('/ui/src/widgets/research/ResearchWidget.tsx');
    const art = { name: 'Mod technology', icon: null, missing: 'Private artwork reason' };
    for (const element of [createElement(ResearchArt, { art }), createElement(ResearchCard, {
      art, history: {}, slots: [], slot: { index: 0 }, entry: { token: 'mod_technology' }, jump: {},
    })]) {
      const markup = renderToStaticMarkup(element);
      assert.match(markup, /art-fallback/);
      assert.doesNotMatch(markup, /Private artwork reason|artwork unavailable|\?>/i);
    }
    assert.match(renderToStaticMarkup(createElement(ResearchCard, {
      art, history: {}, slots: [], slot: { index: 0 }, entry: { token: 'mod_technology' }, jump: {},
    })), /Mod technology, slot 1, show details/);
    const pictured = renderToStaticMarkup(createElement(ResearchArt, { art: { ...art, icon: 'data:image/png;base64,YQ==' } }));
    assert.match(pictured, /<img/);
    assert.doesNotMatch(pictured, /art-fallback/);
  } finally { await vite.close(); }
});
