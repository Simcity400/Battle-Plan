import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { ResearchReply } from '../../generated/dashboard';

for (const completed of [false, true]) test(`research ${completed ? 'populated' : 'empty'} lanes keep named native ordered lists and virtual positions`, async () => {
  const data: ResearchReply = { history: { coverage: [{ firstHour: 0, lastHour: 30 }], slots: Array.from({ length: 4 }, (_, index) => ({
    index, availability: [{ firstHour: 0, lastHour: 30 }], states: [], progress: [],
    entries: completed ? [{ token: `synthetic_technology_${index}`, technologyHandle: index + 1, continuityStartHour: 0,
      level: 0, firstObservedHour: 0, startedAfterHour: null, lastObservedHour: 9, endedByHour: 10, outcome: 'completed' as const }] : [],
  })) }, artwork: [], folders: [], technologyFolders: [], warnings: [] };
  const host = globalThis as typeof globalThis & { __researchListReply?: ResearchReply };
  host.__researchListReply = data;
  const replacements: Record<string, string> = {
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ categories: [] });',
    '/research/preparedResearch.ts': 'export const usePreparedResearch = () => ({ data: globalThis.__researchListReply });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/dashboard/useDiagnostics.ts': 'export const useDiagnostics = () => {};',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/research/useTechnologyArtwork.ts': 'export const useTechnologyArtwork = () => ({ artwork: new Map(), warnings: [] });',
  };
  const vite = await createServer({ root: fileURLToPath(new URL('../../../../', import.meta.url)), configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'research-list-fixture', enforce: 'pre',
      transform(_source, id) { const path = id.replaceAll('\\', '/'); return Object.entries(replacements).find(([suffix]) => path.endsWith(suffix))?.[1]; },
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/research/ResearchWidget.tsx');
    const props = { widget: { id: 'research', kind: 'research', trackIds: ['track'], countries: { kind: 'tags', tags: ['synthetic'] }, time: { mode: 'follow' } },
      runtime: { client: {}, tracks: [{ id: 'track', axisOffset: 0, branch: { id: 'branch', firstHour: 0, lastHour: 30 },
        source: { id: 'source', revision: '1' } }], clock: { getHour: () => 30 } }, active: true, headerSlot: null, onDiagnostics() {} } as unknown as WidgetProps;
    const markup = renderToStaticMarkup(createElement(Widget, props));
    const lists = [...markup.matchAll(/<ol\b([^>]*)>([\s\S]*?)<\/ol>/g)];
    assert.equal(lists.length, 4);
    for (const [index, list] of lists.entries()) {
      assert.match(list[1], /class="research-lane"/, 'the native lane is the ordered-list root');
      assert.match(list[1], new RegExp(`aria-label="Slot ${index + 1}"`));
      assert.equal([...list[2].matchAll(/<li\b/g)].length, Number(completed));
      assert.doesNotMatch(list[1], /position:|height:/, 'fitting lanes retain native flow and grid stretching');
      if (completed) {
        assert.match(list[2], /aria-posinset="1"/);
        assert.match(list[2], /aria-setsize="1"/);
        assert.match(list[2], /Technology name unavailable, completed .*, show time breakdown/);
      }
    }
    assert.doesNotMatch(markup, /<div class="research-lane"/, 'no extra lane paint surface wraps the list');
  } finally { delete host.__researchListReply; await vite.close(); }
});


test('List parity preserves known row operands without serializing future slot backing or adding production hooks', async () => {
  const earlier = { token: 'earlier', technologyHandle: 1, continuityStartHour: 0, level: 0,
    firstObservedHour: 0, startedAfterHour: null, lastObservedHour: 9, endedByHour: 10, outcome: 'completed' as const };
  const future = { ...earlier, token: 'future', firstObservedHour: 80, lastObservedHour: 89, endedByHour: 90 };
  const data: ResearchReply = { history: { coverage: [{ firstHour: 0, lastHour: 100 }], slots: [{ index: 0,
    availability: [{ firstHour: 0, lastHour: 100 }], states: [], progress: [], entries: [earlier, future] }] },
    artwork: [], folders: [], technologyFolders: [], warnings: [] };
  type TableProbe = { rows: import('./researchLayout').ResearchRow[]; parity?: { item: (row: import('./researchLayout').ResearchRow) => unknown } };
  const host = globalThis as typeof globalThis & { __researchListReply?: ResearchReply; __researchTable?: TableProbe };
  host.__researchListReply = data;
  const replacements: Record<string, string> = {
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ categories: [] });',
    '/charts/table/Table.tsx': 'export default props => { globalThis.__researchTable = props; return null; };',
    '/research/preparedResearch.ts': 'export const usePreparedResearch = () => ({ data: globalThis.__researchListReply });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/dashboard/useDiagnostics.ts': 'export const useDiagnostics = () => {};',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/research/useTechnologyArtwork.ts': 'export const useTechnologyArtwork = () => ({ artwork: new Map(), warnings: [] });',
  };
  try {
    for (const mode of ['parity', 'production']) {
      const vite = await createServer({ mode, configFile: false, optimizeDeps: { noDiscovery: true },
        server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'research-table-operands', enforce: 'pre',
          transform(_source, id) { return Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1]; } }] });
      try {
        const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/research/ResearchWidget.tsx');
        const props = { widget: { id: 'research', kind: 'research', trackIds: ['track'], countries: { kind: 'tags', tags: ['synthetic'] },
          time: { mode: 'follow' }, options: { view: 'list' } }, runtime: { client: {}, tracks: [{ id: 'track', axisOffset: 0,
          branch: { id: 'branch', firstHour: 0, lastHour: 100 }, source: { id: 'source', revision: '1' } }],
          clock: { getHour: () => 30 } }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
        renderToStaticMarkup(createElement(Widget, props));
        const probe = host.__researchTable!;
        assert.equal(probe.rows.length, 1);
        const row = probe.rows[0];
        assert.equal(row.kind, 'completed');
        if (row.kind !== 'completed') throw new Error('Missing completion');
        assert.equal(row.slot.entries.length, 2, 'the indexed source retains future backing');
        if (mode === 'parity') {
          const item = probe.parity!.item(row);
          const { slot: _slot, ...operands } = row;
          assert.deepEqual(item, { ...operands, slot: 0 });
          assert.doesNotMatch(JSON.stringify(item), /future/);
        } else assert.equal(Object.hasOwn(probe, 'parity'), false);
      } finally { await vite.close(); }
    }
  } finally { delete host.__researchListReply; delete host.__researchTable; }
});
