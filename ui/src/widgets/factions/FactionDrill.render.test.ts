import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { CountryGroupReply, FactionGroup } from '../../generated/dashboard';
import { BRANCH, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import { factionViews } from './factionsData.ts';

test('a dissolved faction shows no power projection kept from its last reply', async () => {
  // Founded at 0 and dissolved after 100; every later hour is recorded.
  const groups = { ...pendingGroups({ final: true }), factions: [{ id: '5', name: 'Mod pact', recordedName: 'Mod pact', firstHour: 0, lastHour: 100,
    nameReason: null, names: [{ firstHour: 0, lastHour: 100, name: 'Mod pact', reason: null }], members: [{ tag: 'ENG', firstHour: 0, lastHour: 100 }],
    leaders: [], incomplete: [] }] } as CountryGroupReply;
  const [view] = factionViews(groups, (faction: FactionGroup) => ({ name: faction.name, reason: faction.nameReason }), () => '#123456');
  const { module, close } = await mountModule<{ default: (props: Record<string, unknown>) => ReactElement }>('/ui/src/widgets/factions/FactionDrill.tsx', {
    // The reply read while the faction stood: what a held reply keeps showing.
    '/dashboard/products.ts': `export const useProduct = () => ({ retry() {}, error: null, data: { faction: '5', hour: 50, projectionRaw: '1234500000',
      goals: [], completed: [], cancelled: [], manifest: null, resources: null, logo: null } });`,
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/charts/useRowTooltips.ts': 'export const useRowTooltips = () => ({ rowProps: () => ({}), descriptionId: key => key });',
  });
  try {
    const props = { runtime: { client: {} }, track: { source: { id: 'source', revision: '1' }, branch: BRANCH }, view, names: [], at: 150, active: true,
      gaps: [], pending: [], onBack() {} };
    const markup = renderToStaticMarkup(createElement(module.default, props));
    assert.match(markup, /This faction was dissolved/);
    assert.doesNotMatch(markup, /power projection/);
    assert.doesNotMatch(markup, /12,345/);
  } finally { await close(); }
});
