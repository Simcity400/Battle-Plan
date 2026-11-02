import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { PreparedFocusHour } from './preparedFocusData';

test('prepared current and completed titles survive an independent static-text failure', async () => {
  const fixture = { stateIndex: 0, answer: null as unknown };
  const host = globalThis as typeof globalThis & { __preparedTitleState?: (initial: unknown) => unknown };
  host.__preparedTitleState = initial => [fixture.stateIndex++ === 1 ? fixture.answer : initial, () => {}];
  const replacements: Record<string, string> = {
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 35;',
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/useDiagnostics.ts': 'export const useDiagnostics = () => {};',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/national-focus/useFocusArtwork.ts': 'export const useFocusArtwork = () => ({ details: new Map(), warnings: [] }); export const useContinuousFocusArtwork = () => ({ read: new Map(), details: new Map(), warnings: [] });',
    '/components/VirtualList.tsx': `import { createElement } from 'react'; export const VirtualList = props => createElement('ol', null,
      props.items.map((item, index) => createElement('li', {key:index}, props.children(item))));`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'prepared-title-failure', enforce: 'pre', transform(source, id) {
      const path = id.replaceAll('\\', '/'), replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
      if (replacement) return replacement[1];
      if (path.endsWith('/national-focus/PreparedFocusWidget.tsx')) return source.replace(/\buseState(?:<[\s\S]*?>)?\(/g, 'globalThis.__preparedTitleState(');
    } }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/national-focus/PreparedFocusWidget.tsx') as typeof import('./PreparedFocusWidget');
    const { loadPreparedFocusPage } = await vite.ssrLoadModule('/ui/src/widgets/national-focus/preparedFocusData.ts') as typeof import('./preparedFocusData');
    const { RefusedError } = await vite.ssrLoadModule('/ui/src/dashboard/queryError.ts') as typeof import('../../dashboard/queryError');
    const source = { id: 'synthetic', revision: '1' }, branch = { id: '1', firstHour: 0, lastHour: 40, gaps: [] };
    const row: PreparedFocusHour = { schema: 6, slot: 1, countryId: 7, hour: 30, lastHour: 40, orderFirstHour: 10, at: 35,
      current: 'current', continuous: null, paused: false, progressRaw: '0', dailyGainRaw: null, completed: { first: '0', count: '1', rows: '1' } };
    for (const error of [new Error('Synthetic static-text decoder failed'), new RefusedError('Synthetic static focus text was not recorded')]) {
      const client = { preparedQuery: async (_source: unknown, product: string) => {
        if (product === 'focus-order-page') return [{ ordinal: 0, token: 'completed', completedByHour: 10 }];
        throw error;
      }, product: async (request: any) => ({ ...request, data: { art: request.params.tokens.map((token: string) => ({
        artwork: { token, name: `Dated ${token} title`, description: null, baseDurationDays: null, icon: null }, reason: null })), warnings: [], failures: [] } }) };
      const props = { widget: { id: 'focus', kind: 'national-focus', trackIds: ['main'], countries: { kind: 'tags', tags: ['TAG'] }, time: { mode: 'pin', windows: { main: { firstOffset: 35, lastOffset: 35 } } } },
        runtime: { client, tracks: [{ id: 'main', axisOffset: 0, source, branch }], clock: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
      const loaded = await loadPreparedFocusPage(client as never, source as never, '1', 'TAG', 35, row, 0n, new AbortController().signal);
      for (const art of loaded.art.values()) { assert.equal(art.failure, error); assert.equal(art.titleFailure, undefined); }
      fixture.answer = { identity: JSON.stringify([source.id, source.revision, branch.id, 'TAG', 0]), client, hour: 35,
        snapshot: { ...loaded, hour: row, page: 0n }, pending: false, error: null, warnings: [] };
      fixture.stateIndex = 0;
      const html = renderToStaticMarkup(createElement(Widget, props));
      assert.match(html, />Dated current title</);
      assert.match(html, />Dated completed title</);
      assert.ok(html.includes(error instanceof RefusedError ? `>${error.text}<` : '>Couldn’t load focus details.<'));
      assert.ok(html.includes(`data-detail="${error.message}"`));
    }
  } finally { delete host.__preparedTitleState; await vite.close(); }
});
