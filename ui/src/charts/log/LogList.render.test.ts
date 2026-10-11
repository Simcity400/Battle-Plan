import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

test('failed demand-loaded log pages show a plain row failure without backend text', async () => {
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'log-page-error-render', enforce: 'pre',
      transform(_source, id) {
        if (id.replaceAll('\\', '/').endsWith('/charts/useChartTokens.ts')) return `export const useViewportHeight = () => 300;
          export const useChartTokens = () => ({font: 'sans-serif', fontSize: {small: 10, body: 12, title: 14}, reducedMotion: true});`;
      },
    }] });
  try {
    const { default: Log } = await vite.ssrLoadModule('/ui/src/charts/log/LogList.tsx');
    const html = renderToStaticMarkup(createElement(Log, { label: 'Synthetic log', density: 'comfortable', months: false, animate: false,
      pages: [{ key: 'missing', start: 0, end: 10, count: 5, error: 'Synthetic private page refusal' }], renderRow: () => null,
      describe: () => '', empty: 'Empty' }));
    assert.match(html, /Couldn’t load rows\./);
    assert.doesNotMatch(html, /Synthetic private page refusal|Rows unavailable/);
  } finally { await vite.close(); }
});
