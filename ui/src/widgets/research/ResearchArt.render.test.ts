import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import { HOUR_EPOCH } from '../../dashboard/calendar.ts';

test('history cells reserve a flask fallback without artwork reasons and say what they open', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'research-art-fixture', enforce: 'pre',
      transform(source, id) {
        if (id.replaceAll('\\', '/').endsWith('/research/ResearchWidget.tsx')) return `${source}\nexport { Art, LaneRow };`;
      },
    }] });
  try {
    const { Art, LaneRow } = await vite.ssrLoadModule('/ui/src/widgets/research/ResearchWidget.tsx');
    const ended = HOUR_EPOCH + 1944 * 8760 + 12;
    const entry = { token: 'mod_technology', technologyHandle: 1, continuityStartHour: ended - 240, level: 0, firstObservedHour: ended - 240,
      startedAfterHour: null, lastObservedHour: ended - 24, endedByHour: ended, outcome: 'completed' };
    const slot = { index: 0, availability: [], entries: [entry], states: [], progress: [] };
    const props = { item: { kind: 'completed', key: 'c', hour: ended, slot, entry }, slots: [slot], coverage: [],
      text: { name: () => 'Mod technology', icon: () => undefined, folder: () => undefined }, dimmed: () => false,
      tip() {}, untip() {}, jump: () => ({}), onOpen() {} };
    const cell = renderToStaticMarkup(createElement(LaneRow, props));
    assert.match(cell, /art-fallback/);
    assert.doesNotMatch(cell, /artwork unavailable|\?>/i);
    assert.match(cell, /aria-label="Mod technology, completed 1 Jan 1944, show time breakdown"/);
    assert.match(cell, /<small>1 Jan 1944<\/small>/, 'the plain date sits under the name');
    const pictured = renderToStaticMarkup(createElement(Art, { icon: 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png', className: 'research-cell-art' }));
    assert.match(pictured, /<img/);
    assert.doesNotMatch(pictured, /art-fallback/);
  } finally { await vite.close(); }
});
