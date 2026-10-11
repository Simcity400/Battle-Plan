import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

test('tooltip headers and rows keep unobtrusive flag slots without explanatory rows or spoken failures', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false } });
  try {
    const { default: Layer } = await vite.ssrLoadModule('/ui/src/charts/TooltipLayer.tsx') as typeof import('./TooltipLayer');
    const { showTooltip, hideTooltip, tooltipText } = await vite.ssrLoadModule('/ui/src/charts/tooltip.ts') as typeof import('./tooltip');
    const content = { title: 'Country', countryFlag: { flagReason: 'Synthetic header refusal' },
      rows: [{ label: 'Member', value: '5', countryFlag: { flagReason: 'Synthetic member refusal' } },
        { label: 'Pending', countryFlag: { flagPending: true } }] };
    showTooltip('e1-render', content, 10, 20);
    try {
      const html = renderToStaticMarkup(createElement(Layer));
      assert.equal((html.match(/country-flag-unavailable/g) ?? []).length, 3);
      assert.match(html, /aria-label="Flag unavailable"/);
      assert.match(html, /aria-label="Flag loading"/);
      assert.doesNotMatch(html, /<small>Flag|Synthetic .* refusal/);
      assert.equal(tooltipText(content), 'Country; Member 5; Pending');
    } finally { hideTooltip('e1-render'); }
  } finally { await vite.close(); }
});
