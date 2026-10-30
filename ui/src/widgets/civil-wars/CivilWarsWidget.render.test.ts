import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createElement } from 'react';
import type { ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import type { CivilWarsReply, CountryGroupReply } from '../../generated/dashboard';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';

test('civil wars of a group counted at an hour still being prepared are loading, not none for these countries', async () => {
  // ENG fights a breakaway throughout; GER, the human player, is in no civil war.
  const data: CivilWarsReply = { branchId: BRANCH.id, lastHour: BRANCH.lastHour, known: [{ firstHour: 0, lastHour: BRANCH.lastHour }], warnings: [],
    wars: [{ id: 'w1', firstHour: 10, lastHour: 200, atWar: [{ firstHour: 10, lastHour: 200 }], outcome: null, sides: [
      { tags: [{ hour: 0, tag: 'ENG' }], firstHour: 10, lastHour: 200, parent: null, existedBefore: true, exit: null },
      { tags: [{ hour: 10, tag: 'D01' }], firstHour: 10, lastHour: 200, parent: 0, existedBefore: false, exit: null }] }] };
  const tags = ['GER', 'ENG', 'D01'];
  const state = { groups: pendingGroups({ tags }) as CountryGroupReply };
  const host = globalThis as typeof globalThis & { __civilPending?: typeof state }; host.__civilPending = state;
  const { module, close } = await mountModule<{ default: ComponentType<WidgetProps> }>('/ui/src/widgets/civil-wars/CivilWarsWidget.tsx', {
    '/dashboard/products.ts': `export const useProduct = () => ({ data: ${JSON.stringify(data)}, retry() {} });
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__civilPending.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({ name: tag => tag, flag() {}, color() {}, revision: 0 });',
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryFactsRead = () => new Map();',
    '/dashboard/useCountriesAt.ts': 'export const countryReadName = () => null; export const FLAGS = {}; export const STYLES = {}; export const useCountriesAt = () => ({ country: tag => ({ name: tag }), revision: 0 });',
    '/dashboard/useHoverLabels.ts': 'export const useHoverLabels = () => ({ ask() {}, labelAt: () => tag => ({ name: tag }), revision: 0 });',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ surface: "#000000", text: "#ffffff", positive: "#00ff00", muted: "#777777", categories: ["#123456"] }); export const useViewportHeight = () => 1000;',
    '/charts/timeline/Timeline.tsx': 'export default () => null;',
  });
  try {
    const props = { widget: { id: 'civil', kind: 'civil-wars', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'synthetic', revision: '1', metadata: { countries: [] } }, branch: { ...BRANCH, gaps: [] } }],
        clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ hoursPerSecond: 0 }) }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // The branch end is still being prepared: its human players are not known yet.
    const pending = render();
    assert.match(pending, /aria-label="This range is still being prepared\."/);
    assert.doesNotMatch(pending, /No civil wars|No ongoing civil wars/);
    // Read, GER is the human player then and is in no civil war.
    state.groups = pendingGroups({ final: true, tags, humans: [{ tag: 'GER', firstHour: 0, lastHour: 200 }] });
    assert.match(render(), /No civil wars for these countries\./);
  } finally { delete host.__civilPending; await close(); }
});
