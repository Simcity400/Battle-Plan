import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';

test('fixed grey colours use the full country catalogue when visible pair order changes', async () => {
  const groups = { countryIdentities: ['T', 'B', 'A'].map((tag, index) => ({ tag,
    identities: [{ firstHour: 0, lastHour: 20, slot: index + 1, countryId: index + 10 }] })) };
  const reply = { branchId: '1', warnings: [], pairs: [{ id: 'opaque', owner: 'A', target: 'T',
    runs: [{ firstHour: 0, lastHour: 20, count: 1, operatives: [{ id: '1', name: null, unnamed: true }] }] }],
    readHours: [0], periods: [{ firstHour: 0, lastHour: 20, availability: 'recorded' }] };
  const host = globalThis as typeof globalThis & { __placementColourNodes?: { label: string; color: string }[] };
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': `export const reply = ${JSON.stringify(reply)}; export const useProduct = (_client, request) => ({ data: request?.product === 'country-groups' ? ${JSON.stringify(groups)} : reply });`,
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({labelAt: () => tag => ({name: tag})});',
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryFactsAt = (_client, _track, wanted) => new Map(wanted.map(item => [nameKey(item.hour, item.tag), [128,128,128]]));',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 20;',
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({muted: '#888888', surface: '#111111', countryColours: 'fixed'});",
    '/charts/chord/ChordChart.tsx': 'export default props => { globalThis.__placementColourNodes = props.frameAt(0).data?.nodes; return null; };',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({menu: null});',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'placements-full-colour-catalogue', enforce: 'pre',
      transform: (_source, id) => Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1],
    }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/who-spies-on-whom/WhoSpiesOnWhomWidget.tsx');
    const { reply: supplied } = await vite.ssrLoadModule('/ui/src/dashboard/products.ts');
    const props = { widget: { id: 'colours', kind: 'who-spies-on-whom', trackIds: ['track'], countries: { kind: 'all' }, time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'track', selectedTag: null, axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [], playerCountryId: null } },
        branch: { id: '1', firstHour: 0, lastHour: 20 } }], client: {}, clock: { getHour: () => 20,
        getSnapshot: () => ({ position: 20, seekSequence: 0, playing: false, hoursPerSecond: 1 }) } }, active: true, onDiagnostics() {} };
    const colour = () => host.__placementColourNodes?.find(node => node.label === 'A')?.color;
    renderToStaticMarkup(createElement(Widget, props));
    const before = colour();
    assert.ok(before);
    supplied.pairs[0].owner = 'B'; supplied.pairs[0].target = 'A';
    renderToStaticMarkup(createElement(Widget, props));
    assert.equal(colour(), before);
  } finally {
    delete host.__placementColourNodes;
    await vite.close();
  }
});

test('registered operative placements retain feature and missing-read reasons in the unavailable state', async () => {
  const reply = { branchId: 'branch', warnings: [], pairs: [], readHours: [],
    periods: [{ firstHour: 0, lastHour: 20, availability: 'featureAbsent' }] };
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': `export const reply = ${JSON.stringify(reply)}; export const useProduct = () => ({ data: reply });`,
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({labelAt: () => tag => ({name: tag})});',
    '/dashboard/countryNamesAt.ts': 'export const useCountryFactsAt = () => new Map(); export const nameKey = (hour, tag) => JSON.stringify([hour, tag]);',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 0;',
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({muted: '#888888', surface: '#111111', countryColours: 'game'});",
    '/charts/chord/ChordChart.tsx': 'export default () => null;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({menu: null});',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'operative-unavailable-state', enforce: 'pre',
      transform: (_source, id) => {
        const path = id.replaceAll('\\', '/');
        const replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        return replacement?.[1];
      },
    }] });
  try {
    const { default: registered } = await vite.ssrLoadModule('/ui/src/widgets/who-spies-on-whom/index.ts');
    const { reply: supplied } = await vite.ssrLoadModule('/ui/src/dashboard/products.ts');
    const props = { widget: { id: 'placements', kind: 'who-spies-on-whom', trackIds: ['track'],
      countries: { kind: 'all' }, time: { mode: 'campaign' }, options: {} }, runtime: {
      tracks: [{ id: 'track', selectedTag: null, axisOffset: 0, source: { id: 'source', revision: '1',
        metadata: { countries: [], playerCountryId: null } },
        branch: { id: 'branch', firstHour: 0, lastHour: 20 } }], client: {}, clock: { getHour: () => 20 },
    }, active: true, onDiagnostics() {} };
    const render = () => renderToStaticMarkup(createElement(registered.render, props));
    assert.match(render(), /Not in this recording\. The intelligence agency feature was disabled\./);
    supplied.periods[0].availability = 'tablesNotRecorded';
    assert.match(render(), /Not in this recording\. Operative placements were not recorded at this time\./);
    props.runtime.tracks = [];
    const empty = render();
    assert.match(empty, /Load a campaign to see its operative placements\./);
    assert.match(empty, /<svg/);
    assert.match(empty, /<circle/);
  } finally {
    await vite.close();
  }
});
