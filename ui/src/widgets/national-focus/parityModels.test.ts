import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { ParityList } from '../../dashboard/parityRows';
import type { FocusReply, ResearchReply } from '../../generated/dashboard';

test('eager focus and research parity models retain complete exact rows without additional content demand', async () => {
  const completed = Array.from({ length: 5000 }, (_, ordinal) => ({ ordinal, token: `mod_focus_${ordinal % 3}`,
    completedByHour: ordinal === 0 ? null : 100 + ordinal }));
  const focus: FocusReply = { history: { segments: [{ firstHour: 100, lastHour: 11000,
    order: { completed, current: 'unfinished_mod_focus', continuous: null, paused: true } }], progress: [] },
    artwork: [], staticText: [{ token: 'mod_focus_0', name: '', description: null, baseDurationDays: null, icon: null }], continuousStaticText: [], warnings: [] };
  const entries = Array.from({ length: 5000 }, (_, index) => ({ token: `mod_technology_${index % 3}`, technologyHandle: index,
    continuityStartHour: 100, level: index % 7, firstObservedHour: 100 + index * 2, startedAfterHour: 99 + index * 2,
    lastObservedHour: 101 + index * 2, endedByHour: 102 + index * 2, outcomeObservedHour: 102 + index * 2, outcome: 'completed' as const }));
  const research: ResearchReply = { history: { coverage: [{ firstHour: 100, lastHour: 11000 }], slots: [
    { index: 0, availability: [{ firstHour: 100, lastHour: 11000 }], entries, progress: [], states: [] },
    { index: 5, availability: [{ firstHour: 11000, lastHour: 11000 }], entries: [], progress: [], states: [{
      firstHour: 11000, lastHour: 11000, token: null, entryFirstObservedHour: null, speedMultiplierRaw: '100000',
      bankedDaysRaw: '9223372036854775807', appliedDaysRaw: '0', aheadReductionYearsRaw: null,
    }] },
  ] }, artwork: [], folders: [], technologyFolders: [], warnings: [] };
  const state = { focus, research, kind: 'national-focus', wanted: [] as unknown[], imageTokens: [] as string[],
    reads: [] as (() => ParityList)[] };
  const host = globalThis as typeof globalThis & { __eagerParityModels?: typeof state };
  host.__eagerParityModels = state;
  const replacements: Record<string, string> = {
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ categories: [] });',
    '/research/preparedResearch.ts': 'export const usePreparedResearch = () => ({ data: globalThis.__eagerParityModels.research });',
    '/dashboard/products.ts': 'export const useProduct = () => ({ data: globalThis.__eagerParityModels.kind === "national-focus" ? globalThis.__eagerParityModels.focus : globalThis.__eagerParityModels.research }); export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
    '/dashboard/parityRows.ts': `export const useParityRows = (_root, read) => {
      globalThis.__eagerParityModels.reads.push(read); };`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/dashboard/useDiagnostics.ts': 'export const useDiagnostics = () => {};',
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/national-focus/useFocusArtwork.ts': `export const useFocusArtwork = (_client, _track, _tag, wanted, _active, details) => {
      globalThis.__eagerParityModels.wanted = wanted;
      if (details.length) throw new Error('Unrequested focus descriptions');
      return { read: new Map(), warnings: [], details: new Map() }; };
      export const useContinuousFocusArtwork = () => ({ read: new Map(), warnings: [], details: new Map() });`,
    '/research/useTechnologyArtwork.ts': `export const useTechnologyArtwork = (_client, _track, tokens) => {
      globalThis.__eagerParityModels.imageTokens = tokens; return {artwork: new Map(), warnings: []}; };`,
  };
  const open = (mode: string) => createServer({ configFile: false, mode, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'eager-parity-models', enforce: 'pre',
      transform(_source, id) { const path = id.replaceAll('\\', '/'); return Object.entries(replacements).find(([suffix]) => path.endsWith(suffix))?.[1]; },
    }] });
  const props = { widget: { id: 'eager', kind: 'national-focus', trackIds: ['main'], countries: { kind: 'tags', tags: ['mod_country'] }, time: { mode: 'campaign' } },
    runtime: { client: {}, clock: { getHour: () => 11000 }, tracks: [{ id: 'main', axisOffset: 0,
      branch: { id: '1', firstHour: 100, lastHour: 11000 }, source: { id: 'source', revision: '1' } }] }, active: true,
    headerSlot: null, onDiagnostics() {} } as unknown as WidgetProps;
  try {
    for (const mode of ['parity', 'production']) {
      const vite = await open(mode);
      try {
        const { default: Focus } = await vite.ssrLoadModule('/ui/src/widgets/national-focus/NationalFocusWidget.tsx');
        state.kind = 'national-focus'; state.reads = [];
        const markup = renderToStaticMarkup(createElement(Focus, props));
        assert.equal(state.wanted.length, 5001);
        if (mode === 'production') { assert.equal(state.reads.length, 0); assert.doesNotMatch(markup, /data-row-index/); }
        else {
          assert.equal(state.reads.length, 1);
          const wanted = state.wanted.slice(), list = state.reads[0]();
          assert.equal(list.complete, true); assert.equal(list.total, 5001);
          assert.equal(new Set(list.rows.map(row => (row as { key: string }).key)).size, 5001);
          const rows = list.rows as { index: number; item: { entry?: typeof completed[number]; readHour: number; unfinished?: boolean } }[];
          assert.deepEqual(rows[0].item, { entry: completed[0], readHour: 100 });
          assert.deepEqual(rows[4999].item.entry, completed[4999]);
          assert.equal(rows[5000].item.unfinished, true);
          assert.deepEqual(state.wanted, wanted, 'reading every row must not alter normal content demand');
          assert.doesNotMatch(JSON.stringify(list.rows), /Loading name|Focus name unavailable/);
        }
        const { default: Research } = await vite.ssrLoadModule('/ui/src/widgets/research/ResearchWidget.tsx');
        state.kind = 'research'; state.reads = [];
        props.widget.time = { mode: 'follow' };
        const researchMarkup = renderToStaticMarkup(createElement(Research, props));
        assert.deepEqual(state.imageTokens, ['mod_technology_0', 'mod_technology_1', 'mod_technology_2']);
        if (mode === 'production') { assert.equal(state.reads.length, 0); assert.doesNotMatch(researchMarkup, /data-row-index|data-log-key/); }
        else {
          assert.equal(state.reads.length, 1);
          const images = state.imageTokens.slice(), list = state.reads[0]();
          assert.equal(list.total, 5003); assert.equal(list.complete, true);
          const rows = list.rows as { index: number; group: number; item: { slot: number; offset: number; kind?: string;
            entry?: typeof entries[number]; current?: ResearchReply['history']['slots'][number]['states'][number] } }[];
          assert.deepEqual(rows[0].item.entry, entries[0]);
          assert.deepEqual(rows[4999].item.entry, entries[4999]);
          assert.equal(rows[5001].item.kind, 'opened', 'the redesigned history retains the late slot opening marker');
          assert.equal(rows[5002].item.slot, 5); assert.equal(rows[5002].item.offset, 5000); assert.equal(rows[5002].group, 1);
          assert.equal(rows[5002].item.current?.bankedDaysRaw, '9223372036854775807');
          assert.deepEqual(state.imageTokens, images);
          assert.doesNotMatch(JSON.stringify(list.rows), /Technology name unavailable/);
        }
        props.widget.time = { mode: 'campaign' };
      } finally { await vite.close(); }
    }
  } finally { delete host.__eagerParityModels; }
});
