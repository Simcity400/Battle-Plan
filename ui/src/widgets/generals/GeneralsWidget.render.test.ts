import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createElement } from 'react';
import type { ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import type { CountryGroupReply } from '../../generated/dashboard';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import type { UnitLeadersData } from './generalsData';

test('leaders of a group counted at an hour still being prepared are loading, not none', async () => {
  // ENG's leader, in a campaign whose human player is GER.
  const data: UnitLeadersData = { branchId: BRANCH.id, firstHour: 0, selectionTags: [],
    people: [{ characterId: 5, token: null, name: 'Synthetic general', portraitHour: 0, portraitReference: '5' }],
    roles: [{ owner: 'ENG', category: 1, rank: null, kind: 1, maximumSkill: 10, attributeNames: [null, null, null, null] }],
    traits: [], leaders: [{ reference: '5', runs: [{ firstHour: 0, lastHour: 200, value: { person: 0, role: 0, skill: 3, components: [1, 2, 3, 4], progress: null, traits: [] } }],
      traitGains: [], commands: [], battleStarts: [], battleStartReadHours: [], unknownBattleStarts: [], partialFrom: null }],
    observed: [{ firstHour: 0, lastHour: 200, leaders: true, landBattles: true, navalBattles: false, landCommands: true, navalCommands: false }],
    resultsMissing: 'Result not recorded', navyCommandsMissing: 'Navy commands not recorded', warnings: [] };
  const state = { groups: pendingGroups() as CountryGroupReply, answered: undefined as number | undefined };
  const host = globalThis as typeof globalThis & { __generalsPending?: typeof state }; host.__generalsPending = state;
  const { module, close } = await mountModule<{ default: ComponentType<WidgetProps> }>('/ui/src/widgets/generals/GeneralsWidget.tsx', {
    '/widgets/generals/useLeaders.ts': `import { useCountriesSelectionAt } from '../../dashboard/useCountriesSelectionAt';
      export const useLeaders = () => ({ data: ${JSON.stringify(data)}, retry() {}, answered: globalThis.__generalsPending.answered });
      export const useLeaderCountries = (widget, runtime, hour, active) => ({ ...useCountriesSelectionAt(widget, runtime, hour, active), requestSelection: { kind: 'all' } });`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__generalsPending.groups, retry() {} });',
    '/widgets/generals/portraits.ts': 'export const useLeaderPortraits = () => ({ at() {}, warnings: [] });',
    '/widgets/general-career/CareerView.tsx': 'export default () => null;',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/playback.ts': 'export const usePlaybackMotion = () => 0;',
    '/charts/table/useTableHour.ts': `export const useTableHour = () => ${BRANCH.lastHour};`,
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 1000;',
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
    '/charts/table/Table.tsx': 'export const CountryCell = () => null; export default () => null;',
  });
  try {
    const props = { widget: { id: 'generals', kind: 'generals', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'synthetic', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // The branch end is still being prepared: its human players are not known yet.
    const pending = render();
    assert.match(pending, /aria-label="This range is still being prepared\."/);
    assert.doesNotMatch(pending, /No leaders for these countries/);
    // Read, GER is the human player then and has no leader.
    state.groups = pendingGroups({ final: true, humans: [{ tag: 'GER', firstHour: 0, lastHour: 200 }] });
    assert.match(render(), /No leaders for these countries at this time\./);
    // ENG was the human player until 100, GER after. Rows held from hour 80 list ENG's
    // leader, as the players were then, while the newest hour is read.
    state.groups = pendingGroups({ final: true, humans: [{ tag: 'ENG', firstHour: 0, lastHour: 100 }, { tag: 'GER', firstHour: 101, lastHour: 200 }] });
    assert.match(render(), /No leaders for these countries at this time\./, 'at the newest hour GER has none');
    state.answered = 80;
    const held = render();
    assert.doesNotMatch(held, /No leaders for these countries/);
    assert.match(held, /class="leader-table"/);
  } finally { delete host.__generalsPending; await close(); }
});
