import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import type { LandBattle, LandBattlesWindow } from './landBattlesData';

test('land battles over hours whose group membership is still being prepared are loading in the list itself, not no matching battles', async () => {
  // One battle at 180-195, between GER and ENG: a human player then only if membership says so.
  const battle: LandBattle = { id: 'episode', provinceId: 17, start: { hour: 180, bound: 'exact' }, end: { hour: 196, bound: 'exact' },
    endUnknownHour: null, counterBase: 0, priorPeak: null, priorSidePeaks: null, losses: [], result: null,
    runs: [{ firstHour: 180, lastHour: 195, counts: [3, 2], rosterUnknown: false, countries: [['GER'], ['ENG']], countriesUnknown: [false, false], duration: 1 }] };
  const window: LandBattlesWindow = { window: 0, interval: { firstHour: BRANCH.firstHour, lastHour: BRANCH.lastHour }, rows: [battle],
    observed: [[BRANCH.firstHour, BRANCH.lastHour]], warnings: [], contentWarnings: [] };
  const state = { groups: pendingGroups(), window };
  const host = globalThis as typeof globalThis & { __landPending?: typeof state }; host.__landPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/land-battles/LandBattlesWidget.tsx', {
    '/charts/table/useTableHour.ts': `export const useTableHour = () => ${BRANCH.lastHour};`,
    '/dashboard/products.ts': `export const useProduct = (_client, request) => ({ data: request?.product.endsWith('-manifest')
      ? { windows: [{ firstHour: ${BRANCH.firstHour}, lastHour: ${BRANCH.lastHour} }], available: true, unavailable: null, contentWarnings: [] } : undefined, retry() {} });
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/dashboard/productWindows.ts': 'export const useProductWindows = () => ({ data: new Map([[0, globalThis.__landPending.window]]), errors: new Map(), retry() {} });',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__landPending.groups, retry() {} });',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/charts/log/LogList.tsx': `import { createElement } from 'react'; export const PreparedLogCoverage = () => null;
      export default ({ rows, empty }) => createElement('p', null, rows.length ? 'battle rows' : empty);`,
    '/charts/useChartTokens.ts': `export const useChartTokens = () => ({ categories: ['red', 'blue'] });`,
    '/widgets/land-battles/CombatLogDetails.tsx': 'export default () => null;',
  });
  try {
    const props = { widget: { id: 'land', kind: 'land-battles', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'follow' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1' }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour, getMotion: () => undefined, subscribe: () => () => {} }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    const pending = render();
    assert.match(pending, /This range is still being prepared\./);
    assert.doesNotMatch(pending, /No recorded land battles match/);
    // The final reply proves neither side was a human player then.
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /No recorded land battles match these settings\./);
    // A human player in the battle shows it.
    state.groups = pendingGroups({ final: true, humans: [{ tag: 'GER', firstHour: 0, lastHour: 200 }] });
    assert.match(render(), /battle rows/);
  } finally { await close(); delete host.__landPending; }
});
