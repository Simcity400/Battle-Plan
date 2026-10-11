import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { ConstructionLogManifestReply, ConstructionLogReply, ConstructionProject } from '../../generated/dashboard';
import { mergeWindows } from './constructionData.ts';

test('construction uses a factory fallback in its art slot and retains the building-type chips', async () => {
  const interval = { firstHour: 100, lastHour: 120 };
  const project: ConstructionProject = { id: 'project', kind: 'building', tag: 'mod country', building: 0, repair: false,
    state: 1, toState: null, province: null, routeProvinces: null, baseLevel: null, targetLevel: null, costRaw: '100000',
    createdHour: 100, firstRead: 100, runs: [{ ...interval, factories: 5, remaining: 1, repairLevels: 0 }],
    earlierUnknown: false, built: [], unaccounted: [], possible: [], added: [], end: null, firstRepairTick: null };
  const manifest: ConstructionLogManifestReply = { costScale: 100000, windows: [interval], recorded: true, repairIcon: null, warnings: [],
    types: [{ id: 1, token: 'mod building', name: 'Synthetic building', icon: null }, { id: 2, token: 'mod second', name: 'Second building', icon: null }] };
  const reply: ConstructionLogReply = { window: 0, interval, outputReads: [], outputUnavailable: [], projects: [project], outputs: [], warnings: [] };
  const state = { manifest, log: mergeWindows(manifest, new Map([[0, reply]]), 0) };
  const globals = globalThis as typeof globalThis & { __constructionArt?: typeof state };
  globals.__constructionArt = state;
  const replacements: Record<string, string> = {
    '/widgets/construction-log/useConstructionLog.ts': `export const useConstructionLog = () => ({...globalThis.__constructionArt, hour: 100,
      retry() {}, onDrawn() {}});`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 100;',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({retry() {}});',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({labelAt: () => tag => ({name: tag})});',
    '/dashboard/useStateNamesAt.ts': 'export const useStateNamesAt = () => (_hour, state) => ({state, name: "Synthetic location"});',
    '/charts/log/LogList.tsx': `import {createElement} from 'react'; export default props => createElement('div', {},
      props.rows.map(row => createElement('div', {key: row.key}, props.renderRow(row))));`,
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'construction-art', enforce: 'pre',
      transform: (_source, id) => Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1],
    }] });
  try {
    const Widget = (await vite.ssrLoadModule('/ui/src/widgets/construction-log/ConstructionLogWidget.tsx')).default;
    const props = { widget: { trackIds: ['main'], countries: { kind: 'all' }, options: {}, time: { mode: 'campaign' } }, active: true,
      runtime: { client: {}, clock: { getHour: () => 100 }, tracks: [{ id: 'main', source: { id: 'source', revision: '1' }, branch: { id: '1', ...interval } }] },
      onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(Widget, props));
    const missing = render();
    assert.match(missing, /class="log-art"[^>]*><span class="art-fallback"/);
    assert.doesNotMatch(missing, /Artwork unavailable|Icon not recorded|>\?</);
    assert.match(missing, /Synthetic building/); assert.match(missing, /Second building/);
    assert.match(missing, /Building types/);
    manifest.types[0].icon = 'data:image/png;base64,synthetic';
    const art = render();
    assert.match(art, /src="data:image\/png;base64,synthetic"/);
    assert.doesNotMatch(art, /art-fallback/);
  } finally { await vite.close(); delete globals.__constructionArt; }
});
