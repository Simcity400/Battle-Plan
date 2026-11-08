import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createElement } from 'react';
import type { ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import type { CountryGroupReply } from '../../generated/dashboard';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import type { UnitLeadersData } from './generalsData';

test('refused leaders have named unavailable states and a partial-table note in both leader widgets', async () => {
  const data: UnitLeadersData = { branchId: BRANCH.id, firstHour: 0, people: [], roles: [], traits: [], leaders: [],
    observed: [{ firstHour: 0, lastHour: 200, leaders: true, landBattles: true, navalBattles: true, landCommands: true, navalCommands: true }],
    withheld: [
      { reference: '1', owner: 'GER', kind: 1, name: 'Synthetic general', reason: 'Template cannot be resolved', detail: null, spans: [{ firstHour: 100, lastHour: 200 }] },
      { reference: '2', owner: null, kind: 1, name: null, reason: 'Owner cannot be resolved', detail: 'raw owner 42 has no CountryLinks row', spans: [{ firstHour: 100, lastHour: 200 }] },
      { reference: '3', owner: 'ENG', kind: 2, name: 'Foreign admiral', reason: 'Foreign refusal', detail: null, spans: [{ firstHour: 100, lastHour: 200 }] },
      { reference: '4', owner: 'GER', kind: 1, name: 'Earlier general', reason: 'Earlier refusal', detail: null, spans: [{ firstHour: 0, lastHour: 99 }] },
      { reference: '6', owner: 'GER', kind: 2, name: 'Synthetic admiral', reason: 'Admiral refusal', detail: null, spans: [{ firstHour: 100, lastHour: 200 }] },
    ], resultsMissing: '', navyCommandsMissing: '', warnings: [] };
  const state = { data };
  const host = globalThis as typeof globalThis & { __leaderRefusals?: typeof state }; host.__leaderRefusals = state;
  const replacements = {
    '/widgets/generals/useLeaders.ts': `export const useLeaders = () => ({ data: globalThis.__leaderRefusals.data, retry() {} });
      export const useLeaderCountries = () => ({ countries: new Set(['GER']), known: true, group: {}, requestSelection: { kind: 'countries', tags: ['GER'] } });`,
    '/dashboard/products.ts': 'export const useFollowedProduct = () => ({ data: globalThis.__leaderRefusals.data, retry() {} });',
    '/dashboard/reachedProduct.ts': 'export const useCountryNameAt = () => "German Reich";',
    '/widgets/generals/portraits.ts': 'export const useLeaderPortraits = () => ({ at() {}, warnings: [] });',
    '/widgets/general-career/CareerView.tsx': 'export default () => null;',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/dashboard/playback.ts': 'export const usePlaybackMotion = () => 0;',
    '/charts/table/useTableHour.ts': 'export const useTableHour = () => 200;',
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 1000;',
    '/charts/drill/DrillHost.tsx': 'export default props => props.main;',
    '/charts/table/Table.tsx': 'export const CountryCell = () => null; export default () => null;',
  };
  try {
    for (const path of ['/ui/src/widgets/generals/GeneralsWidget.tsx', '/ui/src/widgets/general-career/GeneralCareerWidget.tsx']) {
      const { module, close } = await mountModule<{ default: ComponentType<WidgetProps> }>(path, replacements);
      try {
        const props = { widget: { id: 'leaders', kind: path.includes('GeneralCareer') ? 'general-career' : 'generals', trackIds: ['main'],
          countries: { kind: 'selected' }, time: { mode: 'campaign' }, options: { leaders: 'land' } },
          runtime: { tracks: [{ id: 'main', selectedTag: 'GER', axisOffset: 0, source: { id: 'synthetic', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
            clock: { getHour: () => 200 }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
        const render = () => renderToStaticMarkup(createElement(module.default, props));
        const refused = render();
        assert.match(refused, /2 of .*leaders can’t be shown: Template cannot be resolved; Owner cannot be resolved/);
        assert.match(refused, /Synthetic general: Template cannot be resolved/);
        assert.match(refused, /Leader 2: Owner cannot be resolved; raw owner 42 has no CountryLinks row/);
        assert.doesNotMatch(refused, /Admiral refusal|Foreign refusal|Earlier refusal|no generals|No leaders|weren’t recorded/);
        data.observed[0].leaders = false;
        assert.match(render(), /Leaders weren’t recorded at this time/);
        data.observed[0].leaders = true;
        const withheld = data.withheld; data.withheld = [];
        assert.match(render(), /no generals yet|No leaders for these countries/);
        data.withheld = withheld.filter(leader => leader.kind === 2);
        assert.match(render(), /no generals yet|No leaders for these countries/);
        if (path.includes('GeneralsWidget')) {
          props.widget.options = { leaders: 'navy' };
          assert.match(render(), /Synthetic admiral: Admiral refusal/);
          props.widget.options = { leaders: 'land' };
        }
        data.withheld = withheld;
        if (path.includes('GeneralsWidget')) {
          data.people.push({ name: 'Visible general', portraitHour: 0, portraitReference: '5' });
          data.roles.push({ owner: 'GER', category: 1, rank: null, kind: 1, maximumSkill: 10, attributeNames: [null, null, null, null] });
          data.leaders.push({ reference: '5', runs: [{ firstHour: 0, lastHour: 200,
            value: { person: 0, role: 0, skill: 3, components: [1, 2, 3, 4], progress: null, traits: [] } }],
            traitGains: [], commands: [], battleStarts: [], battleStartReadHours: [], unknownBattleStarts: [], partialFrom: null });
          assert.match(render(), /Some leaders are missing\./);
          assert.match(render(), /Synthetic general: Template cannot be resolved/);
          assert.match(render(), /Owner cannot be resolved; raw owner 42 has no CountryLinks row/);
          data.withheld = withheld.filter(leader => leader.kind === 2);
          assert.doesNotMatch(render(), /Some leaders are missing/);
          data.withheld = []; data.observed[0].leaders = false;
          assert.doesNotMatch(render(), /Some leaders are missing/);
          data.withheld = withheld; data.observed[0].leaders = true; data.leaders = [];
        }
      } finally { await close(); }
    }
  } finally { delete host.__leaderRefusals; }
});

test('leaders of a group counted at an hour still being prepared are loading, not none', async () => {
  // ENG's leader, in a campaign whose human player is GER.
  const data: UnitLeadersData = { branchId: BRANCH.id, firstHour: 0, selectionTags: [],
    people: [{ name: 'Synthetic general', portraitHour: 0, portraitReference: '5' }],
    roles: [{ owner: 'ENG', category: 1, rank: null, kind: 1, maximumSkill: 10, attributeNames: [null, null, null, null] }],
    traits: [], leaders: [{ reference: '5', runs: [{ firstHour: 0, lastHour: 200, value: { person: 0, role: 0, skill: 3, components: [1, 2, 3, 4], progress: null, traits: [] } }],
      traitGains: [], commands: [], battleStarts: [], battleStartReadHours: [], unknownBattleStarts: [], partialFrom: null }],
    withheld: [], observed: [{ firstHour: 0, lastHour: 200, leaders: true, landBattles: true, navalBattles: false, landCommands: true, navalCommands: false }],
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
