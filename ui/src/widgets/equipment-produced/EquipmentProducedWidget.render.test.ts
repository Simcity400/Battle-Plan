import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { CountryRaceViewProps } from '../../charts/race/CountryRaceView';
import type { WidgetProps } from '../../dashboard/definitions';
import { MEMBERSHIP_PREPARING } from '../../dashboard/groups.ts';
import { assertNoMemberStates, BRANCH, gapGroups, humansBy, KNOWN_HOUR, mountModule, PENDING_HOUR, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';
import type { CountryGroupReply, EquipmentProducedReply } from '../../generated/dashboard';

test('equipment produced by a group at an hour still being prepared is loading, not unavailable', async () => {
  const reply: EquipmentProducedReply = { branchId: BRANCH.id, measure: 'total', count: 'ic', decimals: 5, grouping: 'type', types: [], keys: [],
    series: [{ tag: 'GER', firstHours: [BRANCH.firstHour], lastHours: [BRANCH.lastHour], breaks: [0], membership: null, values: [], total: null }],
    known: [], reads: [BRANCH.firstHour], observations: [BRANCH.firstHour], warnings: [] };
  const state = { groups: pendingGroups() as CountryGroupReply, reply, view: undefined as CountryRaceViewProps | undefined };
  const host = globalThis as typeof globalThis & { __producedPending?: typeof state }; host.__producedPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/equipment-produced/EquipmentProducedWidget.tsx', {
    '/dashboard/products.ts': 'export const useProduct = () => ({ data: globalThis.__producedPending.reply, retry() {} }); export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__producedPending.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({ categories: [], muted: '#888' }); export const useViewportHeight = () => 800;",
    '/widgets/equipment-produced/useProducedTypes.ts': 'export const useProducedTypeList = () => [];',
    '/widgets/equipment-produced/usePreparedProduction.ts': 'export const hasPreparedProduction = () => false; export const usePreparedProduction = () => ({});',
    '/widgets/equipment-produced/preparedProducedView.ts': 'export const usePreparedProducedView = () => ({ unavailable: true });',
    '/charts/race/CountryPartsDrill.tsx': 'export default props => props.main(() => {}, true);',
    '/charts/race/CountryRaceView.tsx': 'export default props => { globalThis.__producedPending.view = props; return props.state ?? null; };',
  });
  try {
    const props = { widget: { id: 'produced', kind: 'equipment-produced', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' },
      options: { view: 'race', count: 'ic', measure: 'total' } },
    runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
      clock: { getHour: () => BRANCH.lastHour }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // The branch end is still being prepared: its human players are not known yet.
    render();
    assert.equal(state.view?.empty, MEMBERSHIP_PREPARING);
    state.reply = { ...reply, series: [] };
    assert.match(render(), /aria-label="This range is still being prepared\."/);
    // The final reply proves no human player at the end, and no production by the group's countries.
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /No production data at this time\./);
    state.reply = reply;
    render();
    assert.equal(state.view?.empty, 'No production data at this time.');
  } finally { delete host.__producedPending; await close(); }
});

test('equipment produced by a group with no member found yet says what the moment was once read, and loads until then', async () => {
  const empty: EquipmentProducedReply = { branchId: BRANCH.id, measure: 'total', count: 'ic', decimals: 5, grouping: 'type', types: [], keys: [],
    series: [], known: [], reads: [BRANCH.firstHour], observations: [BRANCH.firstHour], warnings: [] };
  const state = { groups: gapGroups(), reply: empty };
  const host = globalThis as typeof globalThis & { __producedGap?: typeof state }; host.__producedGap = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/equipment-produced/EquipmentProducedWidget.tsx', {
    // No member to ask for: no request, so no reply.
    '/dashboard/products.ts': 'export const useProduct = (_client, request) => ({ data: request ? globalThis.__producedGap.reply : undefined, retry() {} }); export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__producedGap.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({ categories: [], muted: '#888' }); export const useViewportHeight = () => 800;",
    '/widgets/equipment-produced/useProducedTypes.ts': 'export const useProducedTypeList = () => [];',
    '/widgets/equipment-produced/usePreparedProduction.ts': 'export const hasPreparedProduction = () => false; export const usePreparedProduction = () => ({});',
    '/widgets/equipment-produced/preparedProducedView.ts': 'export const usePreparedProducedView = () => ({ unavailable: true });',
    '/charts/race/CountryPartsDrill.tsx': 'export default props => props.main(() => {}, true);',
    '/charts/race/CountryRaceView.tsx': 'export default props => props.state ?? null;',
  });
  try {
    const render = (hour: number) => renderToStaticMarkup(createElement(module.default, { widget: { id: 'produced', kind: 'equipment-produced', trackIds: ['main'],
      countries: humansBy('hour'), time: { mode: 'follow' }, options: { view: 'race', count: 'ic', measure: 'total' } },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => hour }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps));
    assertNoMemberStates(render);
  } finally { delete host.__producedGap; await close(); }
});

test('an empty prepared race of a group waits only for membership at its own moment', async () => {
  const reply: EquipmentProducedReply = { branchId: BRANCH.id, measure: 'day', count: 'ic', decimals: 5, grouping: 'type', types: [], keys: [],
    series: [], known: [], reads: [], observations: [], warnings: [] };
  const page = { totalCountries: 0, known: false, recorded: true, countries: [], observation: null };
  const state = { groups: pendingGroups() as CountryGroupReply, history: 0 };
  const host = globalThis as typeof globalThis & { __producedMoment?: typeof state }; host.__producedMoment = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/equipment-produced/EquipmentProducedWidget.tsx', {
    '/dashboard/products.ts': 'export const useProduct = (_client, request) => { if (request) globalThis.__producedMoment.history++; return { retry() {} }; }; export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__producedMoment.groups, retry() {} });',
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/charts/useChartTokens.ts': "export const useChartTokens = () => ({ categories: [], muted: '#888' }); export const useViewportHeight = () => 800;",
    '/widgets/equipment-produced/useProducedTypes.ts': 'export const useProducedTypeList = () => [];',
    '/widgets/equipment-produced/usePreparedProduction.ts': 'export const hasPreparedProduction = () => true; export const usePreparedProduction = () => ({ pending: false, retry() {} });',
    '/widgets/equipment-produced/preparedProducedView.ts': `export const usePreparedProducedView = (_client, _source, params, hour) => params
      ? { page: ${JSON.stringify(page)}, reply: ${JSON.stringify(reply)}, hour, entries: [], tooltip: () => null, pending: false, retry() {} } : { pending: false, retry() {} };`,
    '/charts/race/CountryPartsDrill.tsx': 'export default props => props.main(() => {}, true);',
    '/charts/race/CountryRaceView.tsx': 'export default props => props.state ?? null;',
  });
  try {
    const render = (hour: number) => renderToStaticMarkup(createElement(module.default, { widget: { id: 'produced', kind: 'equipment-produced', trackIds: ['main'],
      countries: humansBy('hour'), time: { mode: 'follow' }, options: { view: 'race', count: 'ic', measure: 'day' } },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [] } }, branch: BRANCH }],
        clock: { getHour: () => hour }, client: {} }, active: true, onDiagnostics() {} } as unknown as WidgetProps));
    // Membership at KNOWN_HOUR is read though later hours are still being prepared.
    assert.match(render(KNOWN_HOUR), /A full day of production data is unavailable\./);
    assert.match(render(PENDING_HOUR), /This range is still being prepared\./);
    assert.equal(state.history, 0, 'no whole-branch history is asked');
  } finally { delete host.__producedMoment; await close(); }
});
