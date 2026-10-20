import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import type { CasualtyReply, CountryGroupReply } from '../../generated/dashboard';

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

test('casualties of a group at an hour still being prepared are loading, not none yet', async () => {
  const reply: CasualtyReply = { branchId: BRANCH.id, warnings: [], unknownLabel: 'Unknown losses', observed: [{ firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour }], hasGap: false,
    series: [{ tag: 'GER', reads: [BRANCH.firstHour], ends: [[0, BRANCH.lastHour, null]], dealt: ['60', []], taken: ['20', []], unknown: null, unknownUntil: null, atLeastFrom: null,
      identities: [{ firstHour: BRANCH.firstHour, id: 'a' }], labels: ['GER'], membership: null }] };
  const state = { groups: pendingGroups() as CountryGroupReply };
  const host = globalThis as typeof globalThis & { __casualtiesPending?: typeof state }; host.__casualtiesPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/casualties/CasualtiesWidget.tsx', {
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 0; export const usePlaybackSpeed = () => 0;',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ categories: ["#123"] }); export const useViewportHeight = () => 1000;',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__casualtiesPending.groups, retry() {} });',
    '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({ name: tag => tag, color: () => [1, 2, 3], flag() {}, flagPending: () => false, flagReason() {} });',
    '/widgets/casualties/preparedCasualties.ts': `export const usePreparedCasualties = (_client, _track, _params, hour) => ({ data: ${JSON.stringify(reply)}, hour, retry() {} });`,
    '/widgets/land-battles/CombatLogDetails.tsx': 'export default () => null;',
    '/charts/butterfly/ButterflyChart.tsx': 'export default () => "casualty bars";',
  });
  try {
    const props = { widget: { id: 'casualties', kind: 'casualties', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [], campaignKey: 'campaign' } }, branch: { ...BRANCH, gaps: [] } }],
        clock: { getHour: () => BRANCH.lastHour }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // The branch end is still being prepared: its human players are not known yet.
    const pending = render();
    assert.match(pending, /aria-label="This range is still being prepared\."/);
    assert.doesNotMatch(pending, /No casualties yet/);
    // The final reply proves no human player at the end.
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /No casualties yet\./);
  } finally { delete host.__casualtiesPending; await close(); }
});

test('casualties of a group with no member found yet say what the hour was once read, and load until then', async () => {
  const state = { groups: { ...pendingGroups({ humans: [] }), unobserved: { humans: [{ firstHour: 140, lastHour: 150 }], factions: [], wars: [] } } as CountryGroupReply };
  const host = globalThis as typeof globalThis & { __casualtiesGap?: typeof state }; host.__casualtiesGap = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/casualties/CasualtiesWidget.tsx', {
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => 0; export const usePlaybackSpeed = () => 0;',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ categories: ["#123"] }); export const useViewportHeight = () => 1000;',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__casualtiesGap.groups, retry() {} });',
    '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({ name: tag => tag, color: () => [1, 2, 3], flag() {}, flagPending: () => false, flagReason() {} });',
    // No request, no reply.
    '/widgets/casualties/preparedCasualties.ts': 'export const usePreparedCasualties = (_client, _track, _params, hour) => ({ data: undefined, hour, retry() {} });',
    '/widgets/land-battles/CombatLogDetails.tsx': 'export default () => null;',
    '/charts/butterfly/ButterflyChart.tsx': 'export default () => "casualty bars";',
  });
  try {
    const render = (hour: number) => renderToStaticMarkup(createElement(module.default, { widget: { id: 'casualties', kind: 'casualties', trackIds: ['main'],
      countries: humansBy('hour'), time: { mode: 'follow' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [], campaignKey: 'campaign' } }, branch: { ...BRANCH, gaps: [] } }],
        clock: { getHour: () => hour }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps));
    assert.match(render(145), /Group membership is unavailable at this time\./);
    assert.match(render(100), /This group has no members at this moment\./);
    const later = render(170);
    assert.match(later, /aria-label="This range is still being prepared\."/);
    assert.doesNotMatch(later, /unavailable|no members/);
  } finally { delete host.__casualtiesGap; await close(); }
});
