import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';

test('pending casualty state uses the shared preparation note without showing an empty result', async () => {
  const replacements: Record<string, string> = {
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 109; export const usePlaybackSpeed = () => 24;',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ categories: ["#123"] }); export const useViewportHeight = () => 1000;',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'pending-casualty-render', enforce: 'pre',
      transform: (_source, id) => { const path = id.replaceAll('\\', '/'); const key = Object.keys(replacements).find(key => path.endsWith(key)); return key ? replacements[key] : undefined; },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/casualties/CasualtiesWidget.tsx');
    const props = { widget: { id: 'casualties', trackIds: ['track'], countries: { kind: 'tags', tags: ['mod holder'] },
      time: { mode: 'campaign' }, options: {} }, runtime: { client: {}, clock: { getHour: () => 109 }, tracks: [{ id: 'track',
        axisOffset: 0, selectedTag: 'mod holder', source: { id: 'pending', revision: '1', metadata: { countries: [], campaignKey: 'synthetic' } },
        branch: { id: '1', firstHour: 100, lastHour: 109, gaps: [] } }] }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const html = renderToStaticMarkup(createElement(Widget, props));
    assert.equal((html.match(/This range is still being prepared\./g) ?? []).length, 1);
    assert.match(html, /role="status"/);
    assert.doesNotMatch(html, /No casualties yet|Casualties were not recorded/);
  } finally { await vite.close(); }
});
