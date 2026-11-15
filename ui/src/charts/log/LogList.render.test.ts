import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

const server = () => createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
  server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'log-page-error-render', enforce: 'pre',
    transform(_source, id) {
      if (id.replaceAll('\\', '/').endsWith('/charts/useChartTokens.ts')) return `export const useViewportHeight = () => 300;
        export const useChartTokens = () => ({font: 'sans-serif', fontSize: {small: 10, body: 12, title: 14}, reducedMotion: true});`;
    },
  }] });

const render = async (pages: unknown[] | undefined, more: Record<string, unknown> = {}) => {
  const vite = await server();
  try {
    const { default: Log } = await vite.ssrLoadModule('/ui/src/charts/log/LogList.tsx');
    return renderToStaticMarkup(createElement(Log, { label: 'Synthetic log', density: 'comfortable', months: false, animate: false,
      pages, renderRow: () => null, describe: () => '', empty: 'Empty', ...more }));
  } finally { await vite.close(); }
};

test('failed demand-loaded log pages show a plain row failure without backend text', async () => {
  const html = await render([{ key: 'missing', start: 0, end: 10, count: 5, error: new Error('Synthetic private page refusal') }]);
  assert.match(html, /Couldn’t load rows\./);
  assert.equal(html.match(/data-detail="([^"]*)"/)?.[1], 'Synthetic private page refusal');
  assert.doesNotMatch(html.replace(/<[^>]*>/g, ''), /Synthetic private page refusal|Rows unavailable/);
  assert.match(html, /class="log-page"[^>]*data-state="error"/, 'a page that failed reads as failed, never as rows');
});

test('a page whose rows are still being read is marked loading, as the acceptance probe reads a loading placeholder', async () => {
  const html = await render([{ key: '0', start: 0, end: 4, count: 5 }]);
  assert.match(html, /Loading rows…/);
  assert.match(html, /class="log-page"[^>]*data-state="loading"/);
});

test('a list of sections draws every heading, and each empty section\'s note, in its one scroll even with no row at all', async () => {
  const html = await render(undefined, { sections: [{ key: 'a', label: 'Under way', rows: [], note: 'No operations under way.' },
    { key: 'b', label: 'Completed', pages: [{ key: '0', start: 0, end: 0, count: 1 }], note: 'None completed.' }, { key: 'c', label: 'Ended', rows: [], note: 'None ended.' }] });
  assert.doesNotMatch(html, /class="log-empty"/, 'never the whole list\'s empty state');
  assert.equal(html.match(/class="log-scroll"/g)?.length, 1, 'one scroll');
  assert.deepEqual([...html.matchAll(/role="heading"[^>]*>([^<]*)</g)].map(match => match[1]), ['Under way', 'Completed', 'Ended']);
  assert.match(html, /No operations under way\./);
  assert.match(html, /None ended\./);
  assert.doesNotMatch(html, /None completed\./, 'a section with a window to draw has no note');
  assert.match(html, /Loading rows…/);
});
