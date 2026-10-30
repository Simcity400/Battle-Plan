import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { AirAcesManifestReply } from '../../generated/dashboard';
import type { TableProps } from '../../charts/table/Table';
import type { AceRow, AirAcesWindowReply } from './acesData';
import { BRANCH, humansBy, mountModule, PENDING_HOUR, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';

test('ace portrait slots use a helmet and refresh failures retain Retry without artwork reasons', async () => {
  const interval = { firstHour: 100, lastHour: 147 };
  const detail: AirAcesWindowReply = { branchId: '1', window: 0, ...interval, warnings: [], texts: ['Synthetic ace'],
    aces: [{ id: 'ace', firstSeen: 100, runs: [{ ...interval, tag: 'mod country', name: 0, callsign: null,
      wing: null, wingName: null, aircraft: [], alive: true, death: null }] }] };
  const manifest: AirAcesManifestReply = { branchId: '1', ...interval, available: true, unavailable: null,
    observed: [interval], readHours: [100], warnings: [], windows: [interval], aces: [{ id: 'ace', firstSeen: 100 }], variants: {} };
  const state = { manifest, detail, pending: false, failure: false, preparing: false, portrait: null as string | null, retries: 0,
    parity: undefined as TableProps<AceRow>['parity'], rows: [] as AceRow[] };
  const globals = globalThis as typeof globalThis & { __acePresentation?: typeof state };
  globals.__acePresentation = state;
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': `export const useProduct = () => { const state = globalThis.__acePresentation; return {
      data: state.pending ? undefined : { manifest: state.manifest, roster: state.detail, readHour: 100, preparing: state.preparing ? ['firstSeen','death'] : [] },
      error: state.failure ? 'Synthetic native refusal' : undefined, retry() { state.retries++; } }; };
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/dashboard/productWindows.ts': `export const useWindowDisplay = () => { const state = globalThis.__acePresentation; return {
      data: { manifest: state.manifest, roster: state.detail, readHour: 100, preparing: state.preparing ? ['firstSeen','death'] : [] }, hour: 100 }; };`,
    '/charts/table/useTableHour.ts': 'export const useTableHour = () => 100;',
    '/dashboard/playback.ts': 'export const usePlaybackSpeed = () => 0;',
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({retry() {}});',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({labelAt: () => tag => ({name: tag})});',
    '/dashboard/countryNamesAt.ts': `export const nameKey = (hour, tag) => JSON.stringify([hour, tag]);
      export const useCountryFactsAt = () => new Map([[nameKey(100, 'ace'), {image: globalThis.__acePresentation.portrait, missing: 'Synthetic portrait refusal'}]]);`,
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 1000;',
    '/charts/table/Table.tsx': `import {createElement} from 'react'; export default props => {
      globalThis.__acePresentation.parity = props.parity; globalThis.__acePresentation.rows = props.rows;
      return createElement('div', {}, props.rows.map(row => createElement('div', {key: row.key},
        props.columns.map(column => createElement('span', {key: column.key}, column.render?.(row)))))); };`,
  };
  const vite = await createServer({ configFile: false, mode: 'parity', optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'ace-presentation', enforce: 'pre',
      transform: (_source, id) => Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1],
    }] });
  try {
    const Widget = (await vite.ssrLoadModule('/ui/src/widgets/aces/AcesWidget.tsx')).default;
    const props = { widget: { id: 'aces', kind: 'aces', trackIds: ['main'], countries: { kind: 'all' }, time: { mode: 'campaign' } },
      runtime: { client: {}, tracks: [{ id: 'main', source: { id: 'source', revision: '1' }, branch: { id: '1', ...interval } }] },
      active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(Widget, props));
    const missing = render();
    assert.equal(typeof state.parity?.item, 'function', 'The real widget must supply its exact row projection');
    const operands = state.parity!.item!(state.rows[0]);
    assert.deepEqual((operands as { texts: unknown[] }).texts, [{ index: 0, value: 'Synthetic ace' }]);
    state.manifest = { ...manifest, warnings: ['Synthetic contextual content warning'] };
    state.detail = { ...detail, warnings: ['Synthetic window diagnostic'] };
    render();
    assert.deepEqual(state.parity!.item!(state.rows[0]), operands, 'Contextual warnings must not alter an ace operand');
    assert.match(missing, /art-fallback aces-portrait/);
    assert.doesNotMatch(missing, /Callsign not recorded|Hall of fame|Portrait not recorded|Synthetic portrait refusal|role="img"[^>]*Portrait/);
    state.portrait = 'http://artwork.localhost/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.png';
    const present = render();
    assert.match(present, /class="aces-portrait" src="http:\/\/artwork\.localhost\/a{32}\.png"/);
    assert.doesNotMatch(present, /art-fallback aces-portrait/);
    state.preparing = true;
    state.detail = { ...detail, aces: detail.aces.map(ace => ({ ...ace, firstSeen: null })) };
    assert.match(render(), /Alive; dates still being prepared/);
    state.pending = true; state.failure = true;
    const failed = render();
    assert.match(failed, /Couldn’t load aces\./); assert.match(failed, /Retry/);
    assert.doesNotMatch(failed, /Synthetic native refusal/);
  } finally { await vite.close(); delete globals.__acePresentation; }
});

test('aces read at an hour whose group membership is still being prepared are loading, not none matching', async () => {
  const interval = { firstHour: 0, lastHour: 200 };
  const detail: AirAcesWindowReply = { branchId: BRANCH.id, window: 0, ...interval, warnings: [], texts: ['Synthetic ace'],
    aces: [{ id: 'ace', firstSeen: 0, runs: [{ ...interval, tag: 'ENG', name: 0, callsign: null, wing: null, wingName: null, aircraft: [], alive: true, death: null }] }] };
  const manifest: AirAcesManifestReply = { branchId: BRANCH.id, ...interval, available: true, unavailable: null,
    observed: [interval], readHours: [PENDING_HOUR], warnings: [], windows: [interval], aces: [{ id: 'ace', firstSeen: 0 }], variants: {} };
  const state = { groups: pendingGroups() };
  const host = globalThis as typeof globalThis & { __acesPending?: typeof state }; host.__acesPending = state;
  const display = `{ manifest: ${JSON.stringify(manifest)}, roster: ${JSON.stringify(detail)}, readHour: ${PENDING_HOUR}, preparing: [] }`;
  const { module, close } = await mountModule<{ default: (props: WidgetProps) => ReactElement }>('/ui/src/widgets/aces/AcesWidget.tsx', {
    '/dashboard/products.ts': `export const useProduct = () => ({ data: ${display}, retry() {} });
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/dashboard/productWindows.ts': `export const useWindowDisplay = () => ({ data: ${display}, hour: ${PENDING_HOUR} });`,
    '/charts/table/useTableHour.ts': `export const useTableHour = () => ${PENDING_HOUR};`,
    '/dashboard/playback.ts': 'export const usePlaybackSpeed = () => 0;',
    '/dashboard/contentRevision.ts': 'export const useContentRevision = () => 0;',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__acesPending.groups, retry() {} });',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({labelAt: () => tag => ({name: tag})});',
    '/dashboard/countryNamesAt.ts': 'export const nameKey = (hour, tag) => JSON.stringify([hour, tag]); export const useCountryFactsAt = () => new Map();',
    '/charts/useChartTokens.ts': 'export const useViewportHeight = () => 1000;',
    '/charts/table/Table.tsx': 'export default () => null;',
  });
  try {
    const props = { widget: { id: 'aces', kind: 'aces', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'follow' } },
      runtime: { client: {}, clock: { getHour: () => PENDING_HOUR }, tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1' }, branch: BRANCH }] },
      active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => renderToStaticMarkup(createElement(module.default, props));
    const pending = render();
    assert.match(pending, /aria-label="This range is still being prepared\."/);
    assert.doesNotMatch(pending, /No aces match/);
    // The final reply proves no human player at that hour flies the ace.
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /No aces match these settings\./);
  } finally { delete host.__acesPending; await close(); }
});
