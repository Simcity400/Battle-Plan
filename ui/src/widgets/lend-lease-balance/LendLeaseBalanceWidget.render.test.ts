import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import type { LendLeaseBalanceReply } from '../../generated/dashboard';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';

test('lend-lease at an hour whose group membership is still being prepared is loading, neither an empty group nor no history', async () => {
  // ENG sends industry throughout; no human player has a lend-lease history.
  const data = { branchId: BRANCH.id, lastHour: BRANCH.lastHour, scale: 100000, holder: null, warnings: [], known: [[0, BRANCH.lastHour, '1']],
    histories: [{ holder: '1', tags: [{ hour: 0, tag: 'ENG' }], values: [[[0, BRANCH.lastHour, '500000']], [], [[0, BRANCH.lastHour, '0']], []] }],
    coverage: { firstHour: 0, lastHour: BRANCH.lastHour, complete: true }, pending: [] } as unknown as LendLeaseBalanceReply;
  const state = { data, groups: pendingGroups() };
  const host = globalThis as typeof globalThis & { __lendLeasePending?: typeof state }; host.__lendLeasePending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/lend-lease-balance/LendLeaseBalanceWidget.tsx', {
    '/dashboard/products.ts': 'export const useProduct = () => ({ data: globalThis.__lendLeasePending.data, retry() {} }); export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__lendLeasePending.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => {}; export const usePlaybackSpeed = () => 0;',
    '/dashboard/countryLooks.ts': 'export const useCountryLooks = () => ({ name: tag => tag, flag: () => null, color: () => null });',
    '/charts/race/countryColours.ts': 'export const useCountryColours = () => () => "#123456";',
    '/charts/useChartTokens.ts': 'export const useChartTokens = () => ({ categories: ["#123456"], text: "#ffffff", surface: "#000000", muted: "#777777" }); export const useViewportHeight = () => 1000;',
    '/charts/butterfly/index.ts': 'export const ButterflyChart = () => null;',
  });
  try {
    const props = { widget: { id: 'lend-lease-balance', kind: 'lend-lease-balance', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        client: {}, clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ playing: false, hoursPerSecond: 0 }) } }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    const preparing = /aria-label="This range is still being prepared\."/;
    // The branch end is still being prepared: no human player read there yet is not an empty group.
    const empty = render();
    assert.match(empty, preparing);
    assert.doesNotMatch(empty, /no members/);
    // A human player read into those hours, with no lend-lease history, is not yet proof of none either.
    state.groups = pendingGroups({ humans: [{ tag: 'GER', firstHour: 0, lastHour: BRANCH.lastHour }] });
    const unproven = render();
    assert.match(unproven, preparing);
    assert.doesNotMatch(unproven, /No lend-lease/);
    // The final reply proves both.
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /This group has no members at this date\./);
    state.groups = pendingGroups({ final: true, humans: [{ tag: 'GER', firstHour: 0, lastHour: BRANCH.lastHour }] });
    assert.match(render(), /No lend-lease history is recorded for these countries at this date\./);
  } finally { delete host.__lendLeasePending; await close(); }
});
