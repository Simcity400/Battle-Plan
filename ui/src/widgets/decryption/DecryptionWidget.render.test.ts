import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { WidgetProps } from '../../dashboard/definitions';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';

test('no decryption pair at an hour whose group membership is still being prepared is loading, not none', async () => {
  const state = { groups: pendingGroups() };
  const host = globalThis as typeof globalThis & { __decryptionPending?: typeof state }; host.__decryptionPending = state;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/decryption/DecryptionWidget.tsx', {
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__decryptionPending.groups, retry() {} });',
    '/dashboard/products.ts': `export const useProduct = () => ({ retry() {}, data: { pairs: [], coverage: [{ firstHour: 0, lastHour: 200, complete: true }], warnings: [] } });
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = () => {};',
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 1000;',
    '/charts/useRowTooltips.ts': 'export const useRowTooltips = () => ({});',
    '/components/ContextMenu.tsx': 'export const useContextMenu = () => ({ open() {}, menu: null });',
    '/decryption/decryptionCountries.ts': 'export const useDecryptionCountries = () => ({ pair: () => ({}), revision: 0 });',
    '/charts/WindowedRows.tsx': 'export default props => props.items.length ? null : props.empty;',
  });
  try {
    const props = { widget: { id: 'decryption', kind: 'decryption', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1', metadata: { countries: [], campaignKey: 'campaign' } }, branch: { ...BRANCH, gaps: [] } }],
        client: {}, clock: { getHour: () => BRANCH.lastHour, getSnapshot: () => ({ hoursPerSecond: 0, playing: false }) } }, active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    // The hour shown is still being prepared: its human players are not known yet.
    const pending = render();
    assert.match(pending, /This range is still being prepared\./);
    assert.doesNotMatch(pending, /No matching decryption pairs at this time\./);
    // The final reply proves there is no human player then.
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /No matching decryption pairs at this time\./);
  } finally { delete host.__decryptionPending; await close(); }
});
