import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type { WidgetProps } from '../../dashboard/definitions';

test('General Career preserves its caption, empty and unavailable states when the country name fails', async () => {
  const state = { failure: new Error('Synthetic country styles decoder failed'), top: true, rows: true, recorded: true };
  const host = globalThis as typeof globalThis & { __careerName?: typeof state }; host.__careerName = state;
  const replacements: Record<string, string> = {
    '/dashboard/countryNamesAt.ts': `export const nameKey = (hour,tag) => JSON.stringify([hour,tag]); export const useCountryNamesAt = (_client,_track,wanted) =>
      new Map(wanted.map(row => [nameKey(row.hour,row.tag),{kind:'failed',failure:globalThis.__careerName.failure}]));`,
    '/dashboard/products.ts': `export const loadTransientProduct = () => Promise.resolve(); export const useFollowedProduct = () => ({
      data: { observed: [{ firstHour: 0, lastHour: 10, leaders: globalThis.__careerName.recorded }] }, moment: "[10]" });`,
    '/widgets/generals/useLeaders.ts': `export const useLeaderCountries = () => ({ requestSelection: {}, countries: null, known: true, group: {} });`,
    '/widgets/generals/generalsData.ts': `export const leadersAt = () => globalThis.__careerName.rows ? [{}] : [];
      export const topLeader = () => globalThis.__careerName.top ? {key:"1"} : null; export const validateUnitLeadersReply = data => data;
      export const withheldLeadersAt = () => []; export const withheldLeaderState = () => ({ message: "", detail: "" });`,
    '/widgets/general-career/CareerView.tsx': 'export default props => props.caption;',
    '/charts/table/useTableHour.ts': 'export const useTableHour = () => 10;',
    '/dashboard/countrySelection.ts': 'export const defaultCountryTag = () => "MOD"; export const selectedTags = () => null;',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true }, server: { middlewareMode: true, watch: null, hmr: false, ws: false },
    plugins: [{ name: 'career-name-state', enforce: 'pre', transform(_source, id) { return Object.entries(replacements).find(([suffix]) => id.replaceAll('\\', '/').endsWith(suffix))?.[1]; } }] });
  try {
    const { default: Widget } = await vite.ssrLoadModule('/ui/src/widgets/general-career/GeneralCareerWidget.tsx') as typeof import('./GeneralCareerWidget');
    const { RefusedError } = await vite.ssrLoadModule('/ui/src/dashboard/queryError.ts');
    const props = { widget: { id: 'career', kind: 'general-career', trackIds: ['main'], time: { mode: 'campaign' }, countries: { kind: 'selected' }, options: {} },
      runtime: { client: {}, tracks: [{ id: 'main', source: { id: 'source', revision: '1' }, branch: { id: '1', firstHour: 0, lastHour: 10 }, axisOffset: 0 }], clock: {} }, active: true } as unknown as WidgetProps;
    for (const failure of [state.failure, new RefusedError('Synthetic country styles were not recorded')]) {
      state.failure = failure;
      state.top = true;
      const html = renderToStaticMarkup(createElement(Widget, props));
      assert.ok(html.includes(`data-detail="${failure.message}"`));
      assert.ok(html.includes(failure instanceof RefusedError ? `>${failure.message}<` : '>Couldn’t load country name.<'));
      state.top = false;
      for (const [rows, recorded, message] of [
        [false, true, 'No generals yet.'],
        [true, true, 'Generals’ skills weren’t recorded at this time.'],
        [false, false, 'Leaders weren’t recorded at this time.'],
      ] as const) {
        state.rows = rows;
        state.recorded = recorded;
        const result = renderToStaticMarkup(createElement(Widget, props));
        assert.ok(result.includes(`<span>${message}</span>`));
        assert.ok(result.includes(`<small><span data-detail="${failure.message}">${failure instanceof RefusedError ? failure.message : 'Couldn’t load country name.'}</span></small>`));
        assert.doesNotMatch(result, /is-error|role="alert"|>0<|MOD/, 'the primary state remains intact without inventing a name or zero');
      }
    }
  } finally { delete host.__careerName; await vite.close(); }
});
