import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import type { CountryGroupReply } from '../../generated/dashboard';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';

const faction = (lastHour: number): CountryGroupReply['factions'][number] => ({ id: '5', name: 'Mod pact', recordedName: 'Mod pact', firstHour: 0, lastHour,
  nameReason: null, names: [{ firstHour: 0, lastHour, name: 'Mod pact', reason: null }], members: [{ tag: 'ENG', firstHour: 0, lastHour }], leaders: [], incomplete: [] });

test('no faction for the chosen countries and no faction at all are said, each in its own words', async () => {
  const state = { groups: { ...pendingGroups({ final: true, humans: [{ tag: 'GER', firstHour: 0, lastHour: 200 }] }), factions: [faction(200)] } as CountryGroupReply };
  const host = globalThis as typeof globalThis & { __factionsGroups?: typeof state }; host.__factionsGroups = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/factions/FactionsWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__factionsGroups.groups });',
    '/dashboard/FactionColours.tsx': 'export const useFactionColours = () => ({ colours: new Map(), names: () => ({ name: "Mod pact", reason: null }), warnings: [], setColour() {} });',
    '/widgets/army-size/headHour.ts': 'export const useHeadHour = clock => clock.getHour();',
    '/dashboard/widgetPerf.ts': 'export const useShownEachFrame = () => {};',
    '/dashboard/useCountriesAt.ts': 'export const useCountriesAt = () => ({ country: tag => ({ name: tag }), revision: 0 });',
    '/dashboard/useHoverLabels.ts': 'export const useHoverLabels = () => ({ ask() {}, labelAt: () => tag => ({ name: tag }) });',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ categories: ["#123456"], text: "#ffffff", surface: "#000000", muted: "#777777" }); export const useViewportHeight = () => 1000;',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ menu: null });',
    '/charts/timeline/Timeline.tsx': 'export default () => null;',
    '/widgets/factions/FactionDrill.tsx': 'export default () => null;',
  });
  try {
    const props = (countries: unknown) => ({ widget: { id: 'factions', kind: 'factions', trackIds: ['main'], countries, time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [], campaignKey: 'c' } }, branch: BRANCH }],
        client: {}, clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ hoursPerSecond: 0, playing: false, position: BRANCH.lastHour }), subscribeFrame: () => () => {} } },
      active: true, onDiagnostics() {} }) as unknown as WidgetProps;
    const render = (countries: unknown = humansBy('hour')) => renderToStaticMarkup(createElement(module.default, props(countries)));
    // GER, human throughout, was never in a faction; the faction ENG led is not GER's.
    assert.match(render(), /None of the chosen countries has been in a faction yet\./);
    state.groups = { ...pendingGroups({ final: true }), factions: [] } as CountryGroupReply;
    assert.match(render({ kind: 'all' }), /No factions recorded\./);
  } finally { delete host.__factionsGroups; await close(); }
});
