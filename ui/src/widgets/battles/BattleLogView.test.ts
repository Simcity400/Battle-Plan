import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';
import type { BattleViewAdapter } from './BattleLogView';
import { battleOptions } from './battleOptions.ts';
import { sortKey } from '../../charts/table/options.ts';
import { BRANCH, humansBy, mountModule, pendingGroups } from '../../dashboard/pendingGroups.fixture.ts';

test('background battle loading has one Updating status; dynamic and explicit sorts remain distinct', async () => {
  const windows = [{ firstHour: 100, lastHour: 150 }, { firstHour: 151, lastHour: 200 }];
  const data = { observed: [[100, 150]], warnings: [], contentWarnings: [] };
  const state = { data: new Map([[0, data]]), available: true, prefix: undefined as Record<string, unknown> | undefined, sort: undefined as { column: string } | undefined };
  const host = globalThis as typeof globalThis & { __battleRefresh?: typeof state };
  host.__battleRefresh = state;
  const replacements: Record<string, string> = {
    '/charts/table/useTableHour.ts': 'export const useTableHour = () => 125;',
    '/dashboard/products.ts': `export const useProduct = (_client, request) => ({
      data: request?.product.endsWith('-manifest') ? { windows: ${JSON.stringify(windows)},
        ...globalThis.__battleRefresh.prefix, available: globalThis.__battleRefresh.available, unavailable: 'Synthetic native refusal', contentWarnings: [] }
        : request?.product.endsWith('-ranking') ? {censusHour: 125, effectiveMeasure: 'losses', warnings: [], contentWarnings: []} : undefined,
      retry() {} });
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/dashboard/productWindows.ts': 'export const useProductWindows = () => ({data: globalThis.__battleRefresh.data, errors: new Map(), retry() {}});',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({retry() {}});',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({labelAt: () => tag => ({name: tag})});',
    '/widgets/battles/BattleLog.tsx': 'export default props => {globalThis.__battleRefresh.sort = props.sort; return null;};',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'battle-refresh', enforce: 'pre',
      transform: (_source, id) => Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1],
    }] });
  try {
    const { default: View } = await vite.ssrLoadModule('/ui/src/widgets/battles/BattleLogView.tsx') as typeof import('./BattleLogView');
    const fields = battleOptions([{ value: 'losses', label: 'Losses' }, { value: 'length', label: 'Duration' }]);
    const props = { widget: { id: 'battles', trackIds: ['main'], countries: { kind: 'all' }, time: { mode: 'follow' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1' }, branch: { id: '1', firstHour: 100, lastHour: 200 } }],
        clock: { getHour: () => 125, getMotion: () => undefined, subscribe: () => () => {} }, client: {} }, active: true, onDiagnostics() {},
      adapter: { definition: { title: 'Battles', icon: 'target', options: fields }, product: 'battles',
        validateManifest: (data: unknown) => data, validateWindow: (data: unknown) => data, validateRanking: (data: unknown) => data,
        rows: () => [{ id: 'battle', provinceId: 1, windowIndex: 0, place: { hour: 100, name: 'Synthetic location' } }],
        rankedRows: () => [{ id: 'battle', provinceId: 1, windowIndex: 0, place: { hour: 100, name: 'Synthetic location' } }],
        labels: {}, measures: [{ label: 'losses' }, { label: 'length' }] },
    } as unknown as WidgetProps & { adapter: BattleViewAdapter<never, never, never> };
    const render = () => renderToStaticMarkup(createElement(View<never, never, never>, props));
    const pending = render();
    assert.equal((pending.match(/Updating…/g) ?? []).length, 1);
    assert.doesNotMatch(pending, /Battle reads through|Shown census reads through/);
    state.data.set(1, data);
    assert.doesNotMatch(render(), /Updating…/);
    assert.doesNotMatch(render(), /whole battle/, 'outside a range the battle spans need no note');
    props.widget.time = { mode: 'range', firstOffset: 0, lastOffset: 100 } as never;
    assert.match(render(), /Peak and duration cover the whole battle\./, 'a range keeps the whole-battle note');
    props.widget.time = { mode: 'follow' } as never;
    props.widget.options = { view: 'biggest', measure: 'length' };
    render(); assert.equal(state.sort?.column, 'losses', 'Selected measure follows the effective backend ranking');
    props.widget.options = { view: 'biggest', measure: 'length', tableSort: sortKey({ column: 'length', direction: 'descending' }) };
    render(); assert.equal(state.sort?.column, 'length', 'an explicit column sort stays fixed');
    state.available = false;
    const unavailable = render();
    assert.match(unavailable, /Battles were not recorded\./);
    assert.doesNotMatch(unavailable, /Synthetic native refusal/);
    state.prefix = { coverage: { firstHour: 100, lastHour: 99, complete: false }, pending: [{ firstHour: 100, lastHour: 200 }] };
    const cold = render();
    assert.match(cold, /This range is still being prepared/);
    assert.doesNotMatch(cold, /Battles were not recorded/);
    state.available = true;
    state.prefix = { coverage: { firstHour: 100, lastHour: 150, complete: false }, pending: [{ firstHour: 151, lastHour: 200 }], approximate: [{ firstHour: 100, lastHour: 150 }] };
    assert.match(render(), /Approximate/);

  } finally { await vite.close(); delete host.__battleRefresh; }
});

test('actual battle loader refuses aligned Follow and stored Pin moments outside its branch', async () => {
  const replacements: Record<string, string> = {
    '/charts/table/useTableHour.ts': 'export const useTableHour = clock => clock.getHour();',
    '/dashboard/products.ts': `export const useProduct = (_client, request) => ({
      data: request?.product.endsWith('-manifest') ? { windows: [{ firstHour: 100, lastHour: 200 }],
        available: true, unavailable: null, contentWarnings: [] } : undefined, retry() {} });
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/dashboard/productWindows.ts': `export const useProductWindows = (_client, _identity, _request, wanted, active) => {
      globalThis.__battleWindowLoads.push({ wanted, active });
      return { data: new Map([[0, { observed: [[100, 200]], warnings: [], contentWarnings: [] }]]), errors: new Map(), retry() {} };
    };`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ retry() {} });',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
    '/widgets/battles/BattleLog.tsx': 'export default () => null;',
  };
  const host = globalThis as typeof globalThis & { __battleWindowLoads?: { wanted: number[]; active: boolean }[] };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'battle-window-fixture', enforce: 'pre',
      transform: (_source, id) => { const key = Object.keys(replacements).find(key => id.endsWith(key)); return key ? replacements[key] : undefined; },
    }] });
  try {
    const { default: View } = await vite.ssrLoadModule('/ui/src/widgets/battles/BattleLogView.tsx') as typeof import('./BattleLogView');
    const { resolveWindow } = await vite.ssrLoadModule('/ui/src/dashboard/model.ts') as typeof import('../../dashboard/model');
    let position = -850;
    const props = { widget: { id: 'battle', kind: 'naval-battles', trackIds: ['short'], countries: { kind: 'all' }, time: { mode: 'follow' } },
      runtime: { tracks: [{ id: 'short', axisOffset: 1000, source: { id: 'synthetic', revision: '1' }, branch: { id: '1', firstHour: 100, lastHour: 200 } }],
        clock: { getHour: () => position, getMotion: () => 24, subscribe: () => () => {} }, client: {} }, active: true, onDiagnostics() {},
      adapter: { definition: { title: 'Naval battles', icon: 'shield', options: [] }, product: 'naval-battles',
        validateManifest: (value: unknown) => value, validateWindow: (value: unknown) => value, validateRanking: (value: unknown) => value,
        rows: () => [], rankedRows: () => [], labels: {}, measures: [{ label: 'Ships sunk' }] } as unknown as BattleViewAdapter<never, never, never>,
    } as unknown as WidgetProps & { adapter: BattleViewAdapter<never, never, never> };
    host.__battleWindowLoads = [];
    const track = props.runtime.tracks[0];
    assert.equal(resolveWindow(props.widget, track, [track], position)?.lastHour, 150);
    assert.doesNotMatch(renderToStaticMarkup(createElement(View<never, never, never>, props)), /outside the recording/);
    for (const later of [-950, -750]) {
      position = later;
      host.__battleWindowLoads = [];
      assert.match(renderToStaticMarkup(createElement(View<never, never, never>, props)), /outside the recording/);
      assert.deepEqual(host.__battleWindowLoads, [{ wanted: [], active: false }]);
    }
    props.widget = { ...props.widget, time: { mode: 'pin', windows: { short: { firstOffset: 101, lastOffset: 101 } } } };
    assert.equal(resolveWindow(props.widget, track, [track], position)?.lastHour, 201);
    assert.match(renderToStaticMarkup(createElement(View<never, never, never>, props)), /outside the recording/);
    props.widget = { ...props.widget, time: { mode: 'pin', windows: { short: { firstOffset: 5, lastOffset: 5 } } } };
    position = 5000;
    assert.equal(resolveWindow(props.widget, track, [track], position)?.lastHour, 105);
    assert.doesNotMatch(renderToStaticMarkup(createElement(View<never, never, never>, props)), /outside the recording/);
  } finally {
    delete host.__battleWindowLoads;
    await vite.close();
  }
});

test('an adapter view gets the rows, an empty list, and names still loading told apart from unnamed places', async () => {
  const host = globalThis as typeof globalThis & { __battleView?: { places: unknown; error?: string; rows: unknown[]; ranking?: unknown;
    seen: { rows: unknown[]; pending: boolean[]; measure: string }[] } };
  host.__battleView = { places: undefined, rows: [], seen: [] };
  const replacements: Record<string, string> = {
    '/charts/table/useTableHour.ts': 'export const useTableHour = () => 125;',
    '/dashboard/products.ts': `export const useProduct = (_client, request) => ({
      data: request?.product.endsWith('-manifest') ? { windows: [{ firstHour: 100, lastHour: 200 }], available: true, unavailable: null, contentWarnings: [] }
        // Server rendering draws no rows, so the place-names hook asks for nothing yet.
        : request?.product.endsWith('-ranking') ? globalThis.__battleView.ranking
        : !request || request.product === 'place-names' ? globalThis.__battleView.places : undefined,
      error: !request || request.product === 'place-names' ? globalThis.__battleView.error : undefined, retry() {} });
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/dashboard/productWindows.ts': 'export const useProductWindows = () => ({ data: new Map([[0, { observed: [[100, 200]], warnings: [], contentWarnings: [] }]]), errors: new Map(), retry() {} });',
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ retry() {} });',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({ labelAt: () => tag => ({ name: tag }) });',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'battle-view-fixture', enforce: 'pre',
      transform: (_source, id) => Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1],
    }] });
  try {
    const { default: View } = await vite.ssrLoadModule('/ui/src/widgets/battles/BattleLogView.tsx') as typeof import('./BattleLogView');
    const view = ({ rows, context }: { rows: { provinceId: number; place: { hour: number } }[]; context: { placePending: (row: never) => boolean; rankedMeasure: string } }) => {
      host.__battleView!.seen.push({ rows, pending: rows.map(row => context.placePending(row as never)), measure: context.rankedMeasure });
      return null;
    };
    const row = (provinceId: number) => ({ id: String(provinceId), provinceId, windowIndex: 0, start: null, end: null, sides: [], winner: { unknown: 'x' }, place: { hour: 100, name: null } });
    const props = { widget: { id: 'battles', trackIds: ['main'], countries: { kind: 'all' }, time: { mode: 'campaign' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1' }, branch: { id: '1', firstHour: 100, lastHour: 200 } }],
        clock: { getHour: () => 125, getMotion: () => undefined, subscribe: () => () => {} }, client: {} }, active: true, onDiagnostics() {},
      adapter: { definition: { title: 'Battles', icon: 'target', options: [] }, product: 'battles', view,
        validateManifest: (data: unknown) => data, validateWindow: (data: unknown) => data, validateRanking: (data: unknown) => data,
        rows: () => host.__battleView!.rows, rankedRows: () => host.__battleView!.rows, labels: {}, measures: [{ label: 'losses' }, { label: 'size' }] },
    } as unknown as WidgetProps & { adapter: BattleViewAdapter<never, never, never> };
    const render = () => { host.__battleView!.seen = []; renderToStaticMarkup(createElement(View<never, never, never>, props)); return host.__battleView!.seen.at(-1); };
    // No rows still reach the view, which draws its own empty list beside its header controls.
    assert.deepEqual(render(), { rows: [], pending: [], measure: 'losses' });
    host.__battleView.rows = [row(1), row(2)];
    assert.deepEqual(render()?.pending, [true, true], 'nothing answered yet: both names are loading');
    host.__battleView.places = { places: [{ provinceId: 1, hour: 100, name: null, missing: 'no name' }], states: [], warnings: [] };
    assert.deepEqual(render()?.pending, [false, true], 'an answered place without a name is unnamed, not loading');
    host.__battleView.error = 'Place names unavailable';
    assert.deepEqual(render()?.pending, [false, false], 'a failed lookup is not loading');
    // Biggest tells the view what the ranking ranked by, which falls back from the saved choice.
    host.__battleView.ranking = { censusHour: 125, effectiveMeasure: 'size', warnings: [], contentWarnings: [] };
    props.widget.options = { view: 'biggest', measure: 'losses' };
    assert.equal(render()?.measure, 'size');
  } finally { await vite.close(); delete host.__battleView; }
});

test('a battle log over hours whose group membership is still being prepared is loading, not no matching battles', async () => {
  const state = { groups: pendingGroups() };
  const host = globalThis as typeof globalThis & { __battlePending?: typeof state }; host.__battlePending = state;
  const { module, close } = await mountModule<typeof import('./BattleLogView')>('/ui/src/widgets/battles/BattleLogView.tsx', {
    '/charts/table/useTableHour.ts': `export const useTableHour = () => ${BRANCH.lastHour};`,
    '/dashboard/products.ts': `export const useProduct = (_client, request) => ({ data: request?.product.endsWith('-manifest')
      ? { windows: [{ firstHour: ${BRANCH.firstHour}, lastHour: ${BRANCH.lastHour} }], available: true, unavailable: null, contentWarnings: [] } : undefined, retry() {} });
export const useFollowedProduct = (client, request, moving, ...rest) => ({ ...useProduct(client, request, ...rest), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/dashboard/productWindows.ts': `export const useProductWindows = () => ({ data: new Map([[0, { observed: [[${BRANCH.firstHour}, ${BRANCH.lastHour}]], warnings: [], contentWarnings: [] }]]), errors: new Map(), retry() {} });`,
    '/dashboard/useCountryGroups.ts': 'export const useCountryGroups = () => ({ data: globalThis.__battlePending.groups, retry() {} });',
    '/dashboard/countryFlagsAt.ts': 'export const useCountryLabelsAt = () => ({labelAt: () => tag => ({name: tag})});',
  });
  try {
    const props = { widget: { id: 'battles', trackIds: ['main'], countries: humansBy('hour'), time: { mode: 'follow' }, options: {} },
      runtime: { tracks: [{ id: 'main', axisOffset: 0, source: { id: 'source', revision: '1' }, branch: BRANCH }],
        clock: { getHour: () => BRANCH.lastHour, getMotion: () => undefined, subscribe: () => () => {} }, client: {} }, active: true, onDiagnostics() {},
      adapter: { definition: { title: 'Battles', icon: 'target', options: battleOptions([{ value: 'losses', label: 'Losses' }]) }, product: 'battles',
        validateManifest: (data: unknown) => data, validateWindow: (data: unknown) => data, validateRanking: (data: unknown) => data,
        // The one battle is GER's at hour 190: a human player there only if membership says so.
        rows: (_data: unknown, _hour: number, matches: (tag: string, hour: number) => boolean) => matches('GER', 190)
          ? [{ id: 'battle', provinceId: 1, windowIndex: 0, place: { hour: 190, name: 'Synthetic location' } }] : [],
        rankedRows: () => [], labels: {}, measures: [{ label: 'losses' }] },
    } as unknown as WidgetProps & { adapter: BattleViewAdapter<never, never, never> };
    const render = () => renderToStaticMarkup(createElement(module.default<never, never, never>, props));
    const pending = render();
    assert.match(pending, /aria-label="This range is still being prepared\."/);
    assert.doesNotMatch(pending, /No recorded battles match/);
    // The final reply proves GER was no human player at 190.
    state.groups = pendingGroups({ final: true });
    assert.match(render(), /No recorded battles match these settings\./);
  } finally { await close(); delete host.__battlePending; }
});
