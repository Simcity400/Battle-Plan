import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import type { ComponentProps, ComponentType } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { createServer } from 'vite';
import type CountryPicker from '../../components/CountryPicker';
import type { CountryRaceViewProps } from '../../charts/race/CountryRaceView';
import type { WidgetProps } from '../../dashboard/definitions';
import type { OccupationTrendReply, OccupationTrendRun } from '../../generated/dashboard';

test('the actual occupation mix waits for a native country choice and keeps that holder across tag reuse', async () => {
  const identity = (firstHour: number, lastHour: number, slot: number, countryId: number) => ({ firstHour, lastHour, slot, countryId });
  const run = (firstHour: number, lastHour: number, resistanceRaw: string): OccupationTrendRun => ({ firstHour, lastHour, readHour: firstHour,
    startsSegment: true, resistanceRaw, complianceRaw: '0', states: 1, heldStates: 1, heldComplete: true, collaboration: [] });
  const reply: OccupationTrendReply = { branchId: '1', decimals: 5, warnings: [], series: [
    { tag: 'root', membership: null, segments: [{ firstHour: 100, lastHour: 104 }, { firstHour: 105, lastHour: 110 }],
      identities: [identity(100, 104, 1, 11), identity(105, 110, 2, 22)], runs: [run(100, 104, '100000'), run(105, 110, '200000')] },
    { tag: 'alias', membership: null, segments: [{ firstHour: 105, lastHour: 108 }, { firstHour: 109, lastHour: 110 }],
      identities: [identity(105, 108, 1, 11), identity(109, 110, 3, 33)], runs: [run(105, 108, '300000'), run(109, 110, '900000')] },
  ] };
  const state = { reply, hour: 108, cursor: 0, hooks: [] as unknown[], chart: undefined as CountryRaceViewProps | undefined,
    picker: undefined as ComponentProps<typeof CountryPicker> | undefined,
    requests: [] as { product: string; params: { tags: string[]; family: { kind: string }; reads: { hour: number }[] } }[],
    groups: { error: null as Error | null }, groupsActive: false, productError: null as Error | null };
  const globals = globalThis as typeof globalThis & { __occupationHolderAdapter?: typeof state };
  globals.__occupationHolderAdapter = state;
  const hooks = `const cell = init => { const state = globalThis.__occupationHolderAdapter, index = state.cursor++; return state.hooks[index] ??= init(); };
    export const useEffect = () => {};
    export const useRef = initial => cell(() => ({ current: initial }));
    export const useMemo = (make, deps) => { const saved = cell(() => ({ deps, value: make() }));
      if (saved.deps.length !== deps.length || saved.deps.some((dep, i) => !Object.is(dep, deps[i]))) { saved.deps = deps; saved.value = make(); }
      return saved.value; };
    export const useCallback = (callback, deps) => useMemo(() => callback, deps);
    export const useState = initial => { const saved = cell(() => ({ value: typeof initial === 'function' ? initial() : initial }));
      return [saved.value, next => { saved.value = typeof next === 'function' ? next(saved.value) : next; }]; };`;
  const replacements: Record<string, string> = {
    '/dashboard/products.ts': `export const useProduct = (_client, request, _active, validate) => {
      const state = globalThis.__occupationHolderAdapter;
      if (request) state.requests.push(request);
      return { data: request ? validate(state.reply) : undefined, error: request ? state.productError : null, retry() {} }; };
    export const useFollowedProduct = (client, request, moving, active, validate) => ({ ...useProduct(client, request, active, validate), moment: request ? JSON.stringify(moving.map(name => request.params?.[name])) : undefined });`,
    '/dashboard/useCountryGroups.ts': `export const useCountryGroups = (_runtime, _track, active) => {
      const state = globalThis.__occupationHolderAdapter; state.groupsActive = active; return state.groups; };`,
    '/dashboard/playback.ts': 'export const usePlaybackHour = clock => clock.getHour();',
    '/charts/race/CountryRaceView.tsx': `import { createElement } from 'react'; export default props => {
      globalThis.__occupationHolderAdapter.chart = props; return createElement('div', null, props.beforeChart); };`,
    '/components/CountryPicker.tsx': 'export default props => { globalThis.__occupationHolderAdapter.picker = props; return null; };',
  };
  const vite = await createServer({ configFile: false, optimizeDeps: { noDiscovery: true },
    server: { middlewareMode: true, watch: null, hmr: false, ws: false }, plugins: [{ name: 'occupation-native-holder-fixture', enforce: 'pre',
      resolveId(id) { if (id === 'occupation-holder-hooks') return '\0occupation-holder-hooks'; },
      load(id) { if (id === '\0occupation-holder-hooks') return hooks; },
      transform(source, id) {
        const path = id.replaceAll('\\', '/'), replacement = Object.entries(replacements).find(([suffix]) => path.endsWith(suffix));
        if (replacement) return replacement[1];
        if (path.endsWith('/occupation-trend/OccupationTrendWidget.tsx') || path.endsWith('/occupation-trend/preparedOccupation.ts')) {
          return source.replace("from 'react'", "from 'occupation-holder-hooks'");
        }
      },
    }] });
  try {
    const registry = await vite.ssrLoadModule('/ui/src/dashboard/definitions.ts');
    registry.registerWidgets([(await vite.ssrLoadModule('/ui/src/widgets/occupation-trend/definition.ts')).definition]);
    const Widget = (await vite.ssrLoadModule('/ui/src/widgets/occupation-trend/OccupationTrendWidget.tsx')).default as ComponentType<WidgetProps>;
    const props = { widget: { id: 'occupation', kind: 'occupation-trend', title: 'Occupation trend', trackIds: ['track'],
      countries: { kind: 'tags', tags: ['root'] }, time: { mode: 'follow' }, options: { view: 'mix', measure: 'both' } },
      runtime: { tracks: [{ id: 'track', axisOffset: 0, source: { id: 'synthetic', revision: '1', metadata: { countries: [] } },
        branch: { id: '1', firstHour: 100, lastHour: 110 } }], client: {}, clock: { getHour: () => state.hour } },
      active: true, onDiagnostics() {} } as unknown as WidgetProps;
    const render = () => {
      state.cursor = 0; state.chart = undefined; state.picker = undefined;
      state.requests = [];
      const markup = renderToStaticMarkup(createElement(Widget, props));
      assert.ok(state.requests.length <= 5, 'one render has at most the five prepared request slots');
      return markup;
    };
    render();
    assert.equal(state.requests[0].product, 'series-state');
    assert.equal(state.requests[0].params.family.kind, 'occupation-trend');
    assert.deepEqual(state.requests[0].params.reads, [{ hour: 108 }]);
    assert.deepEqual(state.requests[0].params.tags, ['root'], 'the request keeps the original literal root');
    assert.deepEqual(new Set(state.picker!.allowedTags), new Set(['root', 'alias']));
    assert.equal(state.chart!.mix, null, 'several native holders need a choice instead of choosing the first tag or race leader');
    const obsolete = state.picker!.onChange;
    state.picker!.onChange('alias'); render();
    assert.equal(state.chart!.mix!.tag, 'alias');
    assert.ok(state.chart!.mix!.lines!.every(line => line.runs.every(run => run.lastHour <= (line.tag === 'root' ? 104 : 108))),
      'only the chosen native holder contributes');
    const oldTips = [...(state.chart!.tooltip('alias', 100, 'alias', new Map(), () => tag => ({ name: tag })) ?? [])];
    assert.equal(oldTips[0].title, 'root', 'historical tooltips use the chosen holder’s alias at that hour');
    state.hour = 109;
    assert.match(render(), /Country unavailable at this time/);
    assert.deepEqual(state.picker!.allowedTags, ['root'], 'a reused alias is not an additional selected holder');
    assert.equal(state.chart!.mix!.tag, 'alias', 'a missing chosen holder never becomes the tag replacement');
    props.active = false; render(); props.active = true; render();
    obsolete('root'); render();
    assert.equal(state.chart!.mix!.tag, 'alias', 'visibility reentry invalidates an old chooser');
    state.picker!.onChange('root'); render();
    assert.equal(state.chart!.mix!.tag, 'root');
    state.hour = 108; render();
    const replaced = state.picker!.onChange;
    state.reply = structuredClone(reply); state.reply.series[0].runs[1].resistanceRaw = '400000'; render();
    replaced('alias'); render();
    assert.equal(state.chart!.mix!.tag, 'root', 'a replaced numeric model invalidates its chooser callback');
    state.groups.error = new Error('Group catalogue refused'); render();
    assert.equal(state.groupsActive, false, 'manual roots use the product native catalogue');
    assert.equal(state.chart!.state, null, 'manual roots bypass a retained inactive group error');
    props.widget.countries = { kind: 'all' }; render();
    assert.equal(state.chart!.state, null, 'All bypasses that same retained catalogue error');
    assert.deepEqual(state.requests[0].params.tags, []);
    props.widget.countries = { kind: 'group', group: { type: 'faction', id: '17' }, membership: 'hour', aggregate: false }; render();
    assert.equal(state.groupsActive, true);
    assert.match(renderToStaticMarkup(state.chart!.state), /Group catalogue refused/, 'an active smart-group catalogue error stays visible');
    props.widget.countries = { kind: 'all' }; render();
    assert.equal(state.groupsActive, false);
    assert.equal(state.chart!.state, null, 'switching the failed smart group to All removes its inactive error');
    props.widget.countries = { kind: 'tags', tags: ['root'] }; state.productError = new Error('Native occupation catalogue refused'); render();
    assert.match(renderToStaticMarkup(state.chart!.state), /Native occupation catalogue refused/, 'the required manual product error remains visible');
  } finally { await vite.close(); delete globals.__occupationHolderAdapter; }
});
