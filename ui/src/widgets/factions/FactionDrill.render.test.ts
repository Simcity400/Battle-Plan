import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { CountryGroupReply, FactionGroup } from '../../generated/dashboard';
import { BRANCH, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import { factionViews } from './factionsData.ts';

test('a dissolved faction shows no power projection kept from its last reply', async () => {
  const groups = { ...pendingGroups({ final: true }), factions: [{ id: '5', name: 'Mod pact', recordedName: 'Mod pact', firstHour: 0, lastHour: 100,
    nameReason: null, names: [{ firstHour: 0, lastHour: 100, name: 'Mod pact', reason: null }], members: [{ tag: 'ENG', firstHour: 0, lastHour: 100 }],
    leaders: [], incomplete: [] }] } as CountryGroupReply;
  const [view] = factionViews(groups, (faction: FactionGroup) => ({ name: faction.name, reason: faction.nameReason }), () => '#123456');
  const { module, close } = await mountModule<{ default: (props: Record<string, unknown>) => ReactElement }>('/ui/src/widgets/factions/FactionDrill.tsx', {
    '/dashboard/factionChoice.ts': `export const useFactionDetails = () => ({ retry() {}, root() {}, at: 50, hour: null, shownAt: () => 50,
      data: { faction: '5', branchId: '1', definitions: [], manifest: null, goals: [], completed: [], cancelled: [], goalReason: null, resources: null,
        manpower: null, manpowerUnlocked: null, researchUnlocked: null, researchCountries: null, doctrineFolders: null, refusals: [], warnings: [], logo: null },
      live: { read: () => ({ get: () => 12345, missing: () => '' }), subscribe: () => () => {} } });`,
    '/dashboard/products.ts': 'export const useProduct = () => ({ retry() {} });',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/charts/useRowTooltips.ts': 'export const useRowTooltips = () => ({ hovered: null, rowProps: () => ({}), descriptionId: key => key });',
  });
  try {
    const clock = { getHour: () => 150, getSnapshot: () => ({ playing: false, position: 150, hoursPerSecond: 0, sequence: 0 }), subscribe: () => () => {}, subscribeFrame: () => () => {} };
    const tracks = [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { campaignKey: 'campaign', countries: [] } }, branch: { ...BRANCH, gaps: [] } }];
    const props = { runtime: { client: {}, tracks, clock }, widget: { id: 'factions', kind: 'factions', trackIds: ['main'], time: { mode: 'campaign' }, options: {} },
      track: { source: { id: 'source', revision: '1' }, branch: BRANCH }, view, names: [], at: 150, active: true, gaps: [], onBack() {} };
    const markup = renderToStaticMarkup(createElement(module.default, props));
    assert.match(markup, /This faction was dissolved/);
    assert.doesNotMatch(markup, /power projection/);
    assert.doesNotMatch(markup, /12,345/);
  } finally { await close(); }
});
